//! Desktop / workspace tiles for FluxBar.
//!
//! Interoperability notes: this plugin talks to [yabai](https://github.com/koekeishiya/yabai)
//! by Koekeishiya and [AeroSpace](https://github.com/nikitabobko/AeroSpace) by
//! Nikita Bobko through their public command-line interfaces. Their command
//! names and JSON fields are reproduced for compatibility; no source code from
//! either project is vendored or copied.
//!
//! The plugin polls the active window manager once per second because macOS
//! does not expose a public API for Mission Control spaces. AeroSpace and yabai
//! are supported. For every space it creates a `workspace.*` item through the
//! `sketchybar` compatibility CLI, then keeps the number, short name, selected
//! state, styling and click handler in sync.

use fluxbar_sdk::{Plugin, PluginContext, PluginEvent, PluginOutput, SdkError, run};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

const SHORT_NAME_LEN: usize = 3;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_COMMAND_OUTPUT: usize = 256 * 1024;

/// Backend used to discover and switch desktops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Aerospace,
    Yabai,
}

/// Optional per-workspace overrides for the tile appearance.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct TileStyle {
    icon: Option<String>,
    color: Option<String>,
    foreground: Option<String>,
    opacity: Option<f64>,
    blur_radius: Option<f64>,
    corner_radius: Option<f64>,
    shape: Option<String>,
    width: Option<f64>,
    tile_height: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct WorkspaceConfig {
    /// `auto`, `aerospace` or `yabai`. `auto` prefers yabai when it responds.
    backend: String,
    /// AeroSpace executable name or absolute path.
    aerospace: String,
    /// yabai executable name or absolute path.
    yabai: String,
    /// Show spaces from every monitor instead of only the focused one.
    all_monitors: bool,
    /// Optional short display names. Keyed by workspace name (AeroSpace) or
    /// space label / index (yabai).
    names: BTreeMap<String, String>,
    /// Fallback icon for spaces without an entry in `icons`.
    icon: String,
    icons: BTreeMap<String, String>,
    /// Optional per-workspace background color overrides.
    colors: BTreeMap<String, String>,
    background: String,
    selected_background: String,
    visible_background: Option<String>,
    foreground: String,
    selected_foreground: String,
    visible_foreground: Option<String>,
    opacity: f64,
    blur_radius: f64,
    corner_radius: f64,
    shape: String,
    /// Tile width in points. `0` uses the natural text width.
    width: f64,
    /// Background height in points.
    tile_height: f64,
    padding_left: f64,
    padding_right: f64,
    /// Apply rules to windows that already existed when the plugin started.
    /// Rules are currently AeroSpace-only.
    move_existing: bool,
    /// App bundle id / app name (wildcards `*` allowed) -> workspace name.
    rules: BTreeMap<String, String>,
    /// Per-workspace tile overrides, keyed by workspace name / space label.
    tiles: BTreeMap<String, TileStyle>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            backend: "auto".into(),
            aerospace: "aerospace".into(),
            yabai: "yabai".into(),
            all_monitors: true,
            names: BTreeMap::new(),
            icon: String::new(),
            icons: BTreeMap::new(),
            colors: BTreeMap::new(),
            background: "0x1FFFFFFF".into(),
            selected_background: "0x52FFFFFF".into(),
            visible_background: Some("0x2EFFFFFF".into()),
            foreground: "0xC6FFFFFF".into(),
            selected_foreground: "0xFFFFFFFF".into(),
            visible_foreground: None,
            opacity: 1.0,
            blur_radius: 14.0,
            corner_radius: 13.0,
            shape: "rounded".into(),
            width: 64.0,
            tile_height: 30.0,
            padding_left: 4.0,
            padding_right: 4.0,
            move_existing: false,
            rules: BTreeMap::new(),
            tiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct WorkspaceInfo {
    /// Stable identity used for the FluxBar item name.
    item_id: String,
    /// Key used by `names`, `icons`, `colors` and `tiles`.
    key: String,
    /// Workspace name for AeroSpace actions; index for yabai.
    target: String,
    /// Name from the window manager (workspace name or yabai space label).
    name: String,
    /// One-based desktop number shown on the tile.
    number: usize,
    focused: bool,
    visible: bool,
}

#[derive(Debug, Deserialize)]
struct YabaiSpace {
    id: i64,
    index: i64,
    #[serde(default)]
    label: String,
    #[serde(default, rename = "has-focus")]
    has_focus: bool,
    #[serde(default, rename = "is-visible")]
    is_visible: bool,
    #[serde(default)]
    display: Option<i64>,
}

#[derive(Debug, Clone)]
struct WindowInfo {
    id: String,
    bundle_id: String,
    app_name: String,
    workspace: String,
}

struct Workspaces {
    config: WorkspaceConfig,
    active_backend: Option<Backend>,
    known_items: HashSet<String>,
    cache_path: Option<PathBuf>,
    previous_cache: HashSet<String>,
    cache_checked: bool,
    seen_windows: Option<HashSet<String>>,
    reported_error: Option<String>,
}

#[fluxbar_sdk::async_trait]
impl Plugin for Workspaces {
    async fn on_init(&mut self, context: &PluginContext) -> fluxbar_sdk::Result<()> {
        if !context.config.is_null() {
            self.config = serde_json::from_value(context.config.clone())
                .map_err(|error| SdkError::Plugin(format!("invalid workspaces config: {error}")))?;
        }

        self.config.aerospace = normalize_command(&self.config.aerospace, "aerospace");
        self.config.yabai = normalize_command(&self.config.yabai, "yabai");
        self.config.backend = self.config.backend.trim().to_ascii_lowercase();
        self.active_backend = match self.config.backend.as_str() {
            "aerospace" => Some(Backend::Aerospace),
            "yabai" => Some(Backend::Yabai),
            // `auto` (and unknown values) are detected on the first tick.
            _ => None,
        };
        self.config.opacity = self.config.opacity.clamp(0.0, 1.0);
        self.config.blur_radius = self.config.blur_radius.max(0.0);
        self.config.corner_radius = self.config.corner_radius.max(0.0);
        self.config.tile_height = self.config.tile_height.max(1.0);
        self.config.width = self.config.width.max(0.0);
        normalize_shape(&mut self.config.shape);
        for tile in self.config.tiles.values_mut() {
            if let Some(opacity) = tile.opacity.as_mut() {
                *opacity = opacity.clamp(0.0, 1.0);
            }
            if let Some(blur_radius) = tile.blur_radius.as_mut() {
                *blur_radius = blur_radius.max(0.0);
            }
            if let Some(corner_radius) = tile.corner_radius.as_mut() {
                *corner_radius = corner_radius.max(0.0);
            }
            if let Some(tile_height) = tile.tile_height.as_mut() {
                *tile_height = tile_height.max(1.0);
            }
            if let Some(width) = tile.width.as_mut() {
                *width = width.max(0.0);
            }
            if let Some(shape) = tile.shape.as_mut() {
                normalize_shape(shape);
            }
        }

        self.cache_path = environment_config_dir().map(|dir| dir.join("workspaces.items"));
        if let Some(path) = &self.cache_path
            && let Ok(contents) = fs::read_to_string(path)
        {
            self.previous_cache = contents
                .lines()
                .map(str::trim)
                .filter(|line| line.starts_with("workspace."))
                .map(str::to_string)
                .collect();
        }

        Ok(())
    }

    async fn on_event(
        &mut self,
        _event: &PluginEvent,
        _context: &PluginContext,
    ) -> fluxbar_sdk::Result<PluginOutput> {
        Ok(PluginOutput::new())
    }

    async fn on_tick(&mut self, _context: &PluginContext) -> fluxbar_sdk::Result<PluginOutput> {
        let backend = match self.resolve_backend().await {
            Ok(backend) => backend,
            Err(error) => return Ok(self.fail(error)),
        };

        match self.sync(backend).await {
            Ok(output) => Ok(output),
            Err(error) => Ok(self.fail(error)),
        }
    }
}

impl Workspaces {
    async fn resolve_backend(&mut self) -> Result<Backend, String> {
        if let Some(backend) = self.active_backend {
            return Ok(backend);
        }

        match self.config.backend.as_str() {
            "yabai" => Ok(Backend::Yabai),
            "aerospace" => Ok(Backend::Aerospace),
            _ => {
                // `auto`: prefer yabai when its daemon answers, then AeroSpace.
                if self.yabai_available().await {
                    self.active_backend = Some(Backend::Yabai);
                    self.config.backend = "yabai".into();
                    Ok(Backend::Yabai)
                } else if self.aerospace_available().await {
                    self.active_backend = Some(Backend::Aerospace);
                    self.config.backend = "aerospace".into();
                    Ok(Backend::Aerospace)
                } else {
                    Err("no workspace backend found; start yabai or AeroSpace, \
                         or set `backend` to `yabai` / `aerospace`"
                        .to_string())
                }
            }
        }
    }

    async fn yabai_available(&self) -> bool {
        self.yabai(&["-m", "query", "--spaces"]).await.is_ok()
    }

    async fn aerospace_available(&self) -> bool {
        self.aerospace(&["list-workspaces", "--format", "%{workspace}"])
            .await
            .is_ok()
    }

    async fn sync(&mut self, backend: Backend) -> Result<PluginOutput, String> {
        let workspaces = match backend {
            Backend::Aerospace => self.list_aerospace_workspaces().await?,
            Backend::Yabai => self.list_yabai_spaces().await?,
        };
        let current_names: HashSet<String> = workspaces
            .iter()
            .map(|workspace| item_name(&workspace.item_id))
            .collect();

        self.remove_stale(&current_names).await;

        for (index, workspace) in workspaces.iter().enumerate() {
            let name = item_name(&workspace.item_id);
            self.ensure_tile(&name).await?;
            self.render_tile(workspace, &name, index, backend).await?;
        }

        self.write_cache(&current_names);

        if backend == Backend::Aerospace && !self.config.rules.is_empty() {
            self.sync_windows().await?;
        }

        let mut output = PluginOutput::new();
        if self.reported_error.take().is_some() {
            output.log("workspaces: connection restored");
        }
        Ok(output)
    }

    async fn list_aerospace_workspaces(&self) -> Result<Vec<WorkspaceInfo>, String> {
        let mut args = vec!["list-workspaces"];
        if self.config.all_monitors {
            args.push("--all");
        } else {
            args.extend(["--monitor", "focused"]);
        }
        args.extend([
            "--format",
            "%{workspace}%{tab}%{workspace-is-focused}%{tab}%{workspace-is-visible}",
        ]);

        let output = self.aerospace(&args).await?;
        let mut workspaces = Vec::new();
        for line in output.lines() {
            let mut fields = line.splitn(3, '\t');
            let name = fields.next().unwrap_or_default().trim();
            if name.is_empty() {
                continue;
            }
            let number = workspaces.len() + 1;
            workspaces.push(WorkspaceInfo {
                item_id: name.to_string(),
                key: name.to_string(),
                target: name.to_string(),
                name: name.to_string(),
                number,
                focused: fields.next().is_some_and(parse_bool),
                visible: fields.next().is_some_and(parse_bool),
            });
        }
        Ok(workspaces)
    }

    async fn list_yabai_spaces(&self) -> Result<Vec<WorkspaceInfo>, String> {
        let output = self.yabai(&["-m", "query", "--spaces"]).await?;
        let mut spaces: Vec<YabaiSpace> = serde_json::from_str(&output)
            .map_err(|error| format!("cannot parse `yabai -m query --spaces`: {error}"))?;
        spaces.sort_by_key(|space| space.index);

        let focused_display = if self.config.all_monitors {
            None
        } else {
            Some(self.focused_yabai_display().await?)
        };

        Ok(spaces
            .into_iter()
            .filter(|space| focused_display.is_none_or(|display| space.display == Some(display)))
            .filter(|space| space.index > 0)
            .map(yabai_space_info)
            .collect())
    }

    async fn focused_yabai_display(&self) -> Result<i64, String> {
        #[derive(Deserialize)]
        struct Display {
            index: i64,
            #[serde(default, rename = "has-focus")]
            has_focus: bool,
        }

        let output = self.yabai(&["-m", "query", "--displays"]).await?;
        let displays: Vec<Display> = serde_json::from_str(&output)
            .map_err(|error| format!("cannot parse `yabai -m query --displays`: {error}"))?;
        displays
            .iter()
            .find(|display| display.has_focus)
            .or_else(|| displays.first())
            .map(|display| display.index)
            .ok_or_else(|| "yabai reported no displays".to_string())
    }

    async fn sync_windows(&mut self) -> Result<(), String> {
        let output = self
            .aerospace(&[
                "list-windows",
                "--all",
                "--format",
                "%{window-id}%{tab}%{app-bundle-id}%{tab}%{app-name}%{tab}%{workspace}",
            ])
            .await?;

        let windows: Vec<WindowInfo> = output
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(4, '\t');
                let id = fields.next()?.trim();
                if id.is_empty() {
                    return None;
                }
                Some(WindowInfo {
                    id: id.to_string(),
                    bundle_id: fields.next().unwrap_or_default().to_string(),
                    app_name: fields.next().unwrap_or_default().to_string(),
                    workspace: fields.next().unwrap_or_default().to_string(),
                })
            })
            .collect();

        let first_snapshot = self.seen_windows.is_none();
        let mut current = HashSet::new();
        for window in &windows {
            current.insert(window.id.clone());
            let is_new = self
                .seen_windows
                .as_ref()
                .is_none_or(|previous| !previous.contains(&window.id));
            if !is_new || (first_snapshot && !self.config.move_existing) {
                continue;
            }
            let Some(target) = self.rule_target(window) else {
                continue;
            };
            if target.is_empty() || target == window.workspace {
                continue;
            }
            let window_id = window.id.clone();
            self.aerospace(&["move-node-to-workspace", "--window-id", &window_id, &target])
                .await?;
        }
        self.seen_windows = Some(current);
        Ok(())
    }

    fn rule_target(&self, window: &WindowInfo) -> Option<String> {
        if let Some(target) = self
            .config
            .rules
            .get(&window.bundle_id)
            .or_else(|| self.config.rules.get(&window.app_name))
        {
            return Some(target.clone());
        }
        if let Some((_, target)) = self.config.rules.iter().find(|(pattern, _)| {
            pattern.eq_ignore_ascii_case(&window.bundle_id)
                || pattern.eq_ignore_ascii_case(&window.app_name)
        }) {
            return Some(target.clone());
        }
        self.config
            .rules
            .iter()
            .find(|(pattern, _)| {
                wildcard_match(pattern, &window.bundle_id)
                    || wildcard_match(pattern, &window.app_name)
            })
            .map(|(_, target)| target.clone())
    }

    async fn remove_stale(&mut self, current: &HashSet<String>) {
        let mut stale: HashSet<String> = self.known_items.difference(current).cloned().collect();
        if !self.cache_checked {
            stale.extend(self.previous_cache.difference(current).cloned());
            self.cache_checked = true;
        }

        for name in stale {
            let _ = self.sketchybar(&["--remove", &name]).await;
            self.known_items.remove(&name);
        }
    }

    async fn ensure_tile(&mut self, name: &str) -> Result<(), String> {
        if self.known_items.contains(name) {
            return Ok(());
        }
        if self.sketchybar(&["--query", name]).await.is_err() {
            self.sketchybar(&["--add", "item", name, "left"]).await?;
        }
        self.known_items.insert(name.to_string());
        Ok(())
    }

    async fn render_tile(
        &mut self,
        workspace: &WorkspaceInfo,
        name: &str,
        index: usize,
        backend: Backend,
    ) -> Result<(), String> {
        let args = self.tile_args(workspace, name, index, backend);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Err(error) = self.sketchybar(&refs).await {
            // Let the next tick recreate a tile that was removed externally.
            self.known_items.remove(name);
            return Err(error);
        }
        Ok(())
    }

    fn tile_args(
        &self,
        workspace: &WorkspaceInfo,
        name: &str,
        index: usize,
        backend: Backend,
    ) -> Vec<String> {
        let selected = workspace.focused;
        let visible = workspace.visible && !selected;
        let tile = lookup(&self.config.tiles, workspace);
        let color = lookup(&self.config.colors, workspace).cloned();
        let tile_color = tile.and_then(|tile| tile.color.clone());
        let background = if selected {
            tile_color
                .or(color)
                .unwrap_or_else(|| self.config.selected_background.clone())
        } else if visible {
            tile_color
                .or_else(|| self.config.visible_background.clone())
                .or(color)
                .unwrap_or_else(|| self.config.background.clone())
        } else {
            tile_color
                .or(color)
                .unwrap_or_else(|| self.config.background.clone())
        };
        let tile_foreground = tile.and_then(|tile| tile.foreground.clone());
        let foreground = if selected {
            tile_foreground.unwrap_or_else(|| self.config.selected_foreground.clone())
        } else {
            tile_foreground
                .or_else(|| {
                    visible
                        .then(|| self.config.visible_foreground.clone())
                        .flatten()
                })
                .unwrap_or_else(|| self.config.foreground.clone())
        };
        let icon = tile
            .and_then(|tile| tile.icon.clone())
            .or_else(|| lookup(&self.config.icons, workspace).cloned())
            .unwrap_or_else(|| self.config.icon.clone());
        let shape = tile
            .and_then(|tile| tile.shape.as_deref())
            .unwrap_or(&self.config.shape);
        let opacity = tile
            .and_then(|tile| tile.opacity)
            .unwrap_or(self.config.opacity);
        let blur_radius = tile
            .and_then(|tile| tile.blur_radius)
            .unwrap_or(self.config.blur_radius);
        let corner_radius = tile
            .and_then(|tile| tile.corner_radius)
            .unwrap_or(self.config.corner_radius);
        let tile_height = tile
            .and_then(|tile| tile.tile_height)
            .unwrap_or(self.config.tile_height);
        let display_name = lookup(&self.config.names, workspace)
            .cloned()
            .unwrap_or_else(|| workspace.name.clone());
        let label = tile_label(workspace.number, &display_name);
        let width = if shape == "circle" {
            tile_height
        } else {
            tile.and_then(|tile| tile.width)
                .unwrap_or(self.config.width)
        };

        vec![
            "--set".to_string(),
            name.to_string(),
            format!("label={label}"),
            format!("icon={icon}"),
            "label.drawing=on".to_string(),
            format!(
                "icon.drawing={}",
                if icon.is_empty() { "off" } else { "on" }
            ),
            "background.drawing=on".to_string(),
            format!("background.color={background}"),
            format!("label.color={foreground}"),
            format!("icon.color={foreground}"),
            format!("background.shape={shape}"),
            format!("background.corner_radius={corner_radius}"),
            format!("background.blur_radius={blur_radius}"),
            format!("background.height={tile_height}"),
            format!("opacity={opacity}"),
            format!("padding_left={}", self.config.padding_left),
            format!("padding_right={}", self.config.padding_right),
            format!("width={width}"),
            format!("drawing_order={}", 100 + index),
            format!("click_script={}", self.focus_command(backend, workspace)),
        ]
    }

    fn focus_command(&self, backend: Backend, workspace: &WorkspaceInfo) -> String {
        match backend {
            Backend::Aerospace => format!(
                "{} workspace {}",
                shell_quote(&expand_home(&self.config.aerospace)),
                shell_quote(&workspace.target)
            ),
            Backend::Yabai => format!(
                "{} -m space --focus {}",
                shell_quote(&expand_home(&self.config.yabai)),
                workspace.number
            ),
        }
    }

    fn write_cache(&self, names: &HashSet<String>) {
        let Some(path) = &self.cache_path else {
            return;
        };
        let mut names: Vec<&String> = names.iter().collect();
        names.sort();
        let contents = names
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = fs::write(path, format!("{contents}\n"));
    }

    fn fail(&mut self, error: String) -> PluginOutput {
        let mut output = PluginOutput::new();
        if self.reported_error.as_deref() != Some(&error) {
            output.log(format!("workspaces: {error}"));
            self.reported_error = Some(error);
        }
        output
    }

    async fn aerospace(&self, args: &[&str]) -> Result<String, String> {
        run_command(&expand_home(&self.config.aerospace), args, "aerospace").await
    }

    async fn yabai(&self, args: &[&str]) -> Result<String, String> {
        run_command(&expand_home(&self.config.yabai), args, "yabai").await
    }

    async fn sketchybar(&self, args: &[&str]) -> Result<String, String> {
        let program = env::var("SKETCHYBAR").unwrap_or_else(|_| "sketchybar".into());
        run_command(&program, args, "sketchybar").await
    }
}

async fn read_capped<R: AsyncRead + Unpin>(reader: &mut R, limit: usize) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut limited = reader.take((limit as u64).saturating_add(1));
    limited.read_to_end(&mut output).await?;
    if output.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("output exceeds {limit} bytes"),
        ));
    }
    Ok(output)
}

async fn run_command(program: &str, args: &[&str], label: &str) -> Result<String, String> {
    // Window-manager queries can block forever when the daemon's messaging
    // port is wedged. Without a timeout every poll would leak a child process
    // and the FluxBar plugin host would eventually restart us.
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("cannot run {label} (`{program}`): {error}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{label}: stdout unavailable"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{label}: stderr unavailable"))?;

    let output = tokio::time::timeout(COMMAND_TIMEOUT, async {
        let result = tokio::try_join!(
            read_capped(&mut stdout, MAX_COMMAND_OUTPUT),
            read_capped(&mut stderr, MAX_COMMAND_OUTPUT)
        );
        if result.is_err() {
            let _ = child.kill().await;
        }
        drop(stdout);
        drop(stderr);
        let status = child.wait().await?;
        let (stdout, stderr) = result?;
        Ok::<_, io::Error>((status, stdout, stderr))
    })
    .await
    .map_err(|_| {
        format!(
            "{label} {} timed out after {}s",
            args.join(" "),
            COMMAND_TIMEOUT.as_secs()
        )
    })?
    .map_err(|error| format!("{label} {} failed: {error}", args.join(" ")))?;

    let (status, stdout, stderr) = output;
    if status.success() {
        return Ok(String::from_utf8_lossy(&stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    Err(format!(
        "{label} {} failed{}",
        args.join(" "),
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        }
    ))
}

fn item_name(workspace: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in workspace.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("workspace.{hash:016x}")
}

fn yabai_space_info(space: YabaiSpace) -> WorkspaceInfo {
    let number = space.index.max(0) as usize;
    let label = space.label.trim().to_string();
    let key = if label.is_empty() {
        number.to_string()
    } else {
        label.clone()
    };
    WorkspaceInfo {
        item_id: format!("yabai:{}", space.id),
        key,
        target: number.to_string(),
        name: label,
        number,
        focused: space.has_focus,
        visible: space.is_visible,
    }
}

/// Build the tile text: the desktop number followed by up to three
/// characters of its name. Numeric names are not repeated (`1`, not `1 1`).
fn tile_label(number: usize, name: &str) -> String {
    let short = truncate_chars(name.trim(), SHORT_NAME_LEN);
    let number = number.to_string();
    if short.is_empty() || short == number {
        number
    } else {
        format!("{number} {short}")
    }
}

fn lookup<'a, V>(map: &'a BTreeMap<String, V>, workspace: &WorkspaceInfo) -> Option<&'a V> {
    map.get(&workspace.key)
        .or_else(|| map.get(&workspace.number.to_string()))
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn normalize_command(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn environment_config_dir() -> Option<PathBuf> {
    ["CONFIG_DIR", "FLUXBAR_CONFIG_DIR"]
        .into_iter()
        .find_map(|key| env::var_os(key).map(PathBuf::from))
}

fn parse_bool(value: &str) -> bool {
    matches!(value.trim(), "true" | "yes" | "1")
}

fn expand_home(value: &str) -> String {
    if let Some(relative) = value.strip_prefix("~/")
        && let Some(home) = env::var_os("HOME")
    {
        return PathBuf::from(home)
            .join(relative)
            .to_string_lossy()
            .to_string();
    }
    value.to_string()
}

fn normalize_shape(shape: &mut String) {
    if !matches!(shape.as_str(), "rectangle" | "rounded" | "circle") {
        *shape = "rounded".into();
    }
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return value.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return value.ends_with(suffix);
    }
    pattern.eq_ignore_ascii_case(value)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> fluxbar_sdk::Result<()> {
    run(Workspaces {
        config: WorkspaceConfig::default(),
        active_backend: None,
        known_items: HashSet::new(),
        cache_path: None,
        previous_cache: HashSet::new(),
        cache_checked: false,
        seen_windows: None,
        reported_error: None,
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_names_are_stable_and_unique() {
        assert_eq!(item_name("1"), item_name("1"));
        assert_ne!(item_name("1"), item_name("2"));
        assert!(item_name("with space").starts_with("workspace."));
    }

    #[test]
    fn labels_are_limited_to_three_characters() {
        assert_eq!(truncate_chars("welcome", 3), "wel");
        assert_eq!(truncate_chars("рабочий", 3), "раб");
        assert_eq!(truncate_chars("", 3), "");
    }

    #[test]
    fn tile_labels_combine_number_and_short_name() {
        assert_eq!(tile_label(1, "web"), "1 web");
        assert_eq!(tile_label(2, "welcome"), "2 wel");
        assert_eq!(tile_label(3, "рабочий"), "3 раб");
        assert_eq!(tile_label(4, ""), "4");
        assert_eq!(tile_label(5, "5"), "5");
        assert_eq!(tile_label(6, "  development  "), "6 dev");
    }

    #[test]
    fn yabai_spaces_are_parsed() {
        let json = r#"[
            {"id": 7, "index": 1, "label": "", "has-focus": false, "is-visible": false, "display": 1},
            {"id": 34, "index": 2, "label": "web", "has-focus": true, "is-visible": true, "display": 1}
        ]"#;
        let spaces: Vec<YabaiSpace> = serde_json::from_str(json).unwrap();
        let spaces: Vec<WorkspaceInfo> = spaces.into_iter().map(yabai_space_info).collect();

        assert_eq!(spaces[0].item_id, "yabai:7");
        assert_eq!(spaces[0].key, "1");
        assert_eq!(spaces[0].name, "");
        assert_eq!(spaces[0].number, 1);
        assert!(!spaces[0].focused);

        assert_eq!(spaces[1].item_id, "yabai:34");
        assert_eq!(spaces[1].key, "web");
        assert_eq!(spaces[1].name, "web");
        assert_eq!(spaces[1].number, 2);
        assert!(spaces[1].focused);
        assert!(spaces[1].visible);
    }

    #[test]
    fn names_override_the_workspace_name() {
        let mut plugin = test_plugin();
        plugin.config.names.insert("1".into(), "web".into());
        let args = plugin.tile_args(
            &workspace("1", 1, true),
            "workspace.test",
            0,
            Backend::Yabai,
        );

        assert!(args.contains(&"label=1 web".to_string()));
    }

    #[test]
    fn click_commands_use_the_active_backend() {
        let plugin = test_plugin();
        let space = workspace("web", 2, true);

        let yabai = plugin.tile_args(&space, "workspace.test", 0, Backend::Yabai);
        assert!(yabai.contains(&"click_script='yabai' -m space --focus 2".to_string()));

        let aerospace = plugin.tile_args(&space, "workspace.test", 0, Backend::Aerospace);
        assert!(aerospace.contains(&"click_script='aerospace' workspace 'web'".to_string()));
    }

    #[tokio::test]
    async fn commands_time_out_instead_of_hanging_the_plugin() {
        let started = std::time::Instant::now();
        let error = run_command("/bin/sh", &["-c", "sleep 10"], "test")
            .await
            .expect_err("the command should time out");
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn wildcard_rules_match_bundle_and_name() {
        assert!(wildcard_match("com.apple.*", "com.apple.Safari"));
        assert!(wildcard_match("*Safari", "com.apple.Safari"));
        assert!(wildcard_match("safari", "Safari"));
        assert!(!wildcard_match("com.google.*", "com.apple.Safari"));
    }

    #[test]
    fn expands_home_in_aerospace_path() {
        let home = env::var_os("HOME").expect("HOME is set");
        assert_eq!(
            expand_home("~/bin/aerospace"),
            PathBuf::from(home)
                .join("bin/aerospace")
                .display()
                .to_string()
        );
        assert_eq!(expand_home("aerospace"), "aerospace");
    }

    #[test]
    fn per_tile_style_overrides_are_used() {
        let mut plugin = test_plugin();
        plugin.config.tiles.insert(
            "2".into(),
            TileStyle {
                icon: Some("X".into()),
                color: Some("0xFF112233".into()),
                foreground: Some("0xFF000000".into()),
                opacity: Some(0.35),
                blur_radius: Some(0.0),
                corner_radius: Some(12.0),
                shape: Some("circle".into()),
                width: Some(99.0),
                tile_height: Some(30.0),
            },
        );
        let workspace = workspace("2", 2, true);
        let args = plugin.tile_args(&workspace, "workspace.test", 0, Backend::Aerospace);

        assert!(args.contains(&"background.color=0xFF112233".to_string()));
        assert!(args.contains(&"label.color=0xFF000000".to_string()));
        assert!(args.contains(&"icon=X".to_string()));
        assert!(args.contains(&"opacity=0.35".to_string()));
        assert!(args.contains(&"background.blur_radius=0".to_string()));
        assert!(args.contains(&"background.corner_radius=12".to_string()));
        assert!(args.contains(&"background.shape=circle".to_string()));
        assert!(args.contains(&"background.height=30".to_string()));
        assert!(args.contains(&"width=30".to_string()));
    }

    #[test]
    fn tiles_without_overrides_use_global_style() {
        let plugin = test_plugin();
        let workspace = workspace("1", 1, false);
        let args = plugin.tile_args(&workspace, "workspace.test", 0, Backend::Yabai);

        assert!(args.contains(&"opacity=1".to_string()));
        assert!(args.contains(&"background.blur_radius=14".to_string()));
        assert!(args.contains(&"background.corner_radius=13".to_string()));
        assert!(args.contains(&"background.height=30".to_string()));
        assert!(args.contains(&"background.shape=rounded".to_string()));
        assert!(args.contains(&"width=64".to_string()));
        assert!(args.contains(&"background.color=0x1FFFFFFF".to_string()));
        assert!(args.contains(&"label=1".to_string()));
        assert!(args.contains(&"icon=".to_string()));
        assert!(args.contains(&"icon.drawing=off".to_string()));
    }

    fn workspace(name: &str, number: usize, focused: bool) -> WorkspaceInfo {
        WorkspaceInfo {
            item_id: name.to_string(),
            key: name.to_string(),
            target: name.to_string(),
            name: name.to_string(),
            number,
            focused,
            visible: false,
        }
    }

    fn test_plugin() -> Workspaces {
        Workspaces {
            config: WorkspaceConfig::default(),
            active_backend: None,
            known_items: HashSet::new(),
            cache_path: None,
            previous_cache: HashSet::new(),
            cache_checked: false,
            seen_windows: None,
            reported_error: None,
        }
    }
}
