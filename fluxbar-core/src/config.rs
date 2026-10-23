//! Configuration model and loader.
//!
//! FluxBar is configured with TOML. The main configuration file is usually
//! `~/.config/fluxbar/fluxbar.toml`. Modular configuration is supported through
//! the `include` list and `plugins.d` directory.

use crate::error::{FluxError, Result};
use crate::geometry::{EdgeInsets, Position};
use crate::item::ItemKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Root configuration document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Bar-level settings.
    #[serde(default)]
    pub bar: BarConfig,

    /// Defaults merged into every item.
    #[serde(default)]
    pub defaults: ItemConfig,

    /// Items declared eagerly. Items can also be added at runtime via IPC.
    #[serde(default)]
    pub items: BTreeMap<String, ItemConfig>,

    /// Plugins to launch.
    #[serde(default)]
    pub plugins: BTreeMap<String, PluginConfig>,

    /// Event / timer definitions.
    #[serde(default)]
    pub events: EventConfig,

    /// Animation tuning.
    #[serde(default)]
    pub animations: AnimationConfig,

    /// Built-in system monitoring.
    #[serde(default)]
    pub system: crate::system::SystemConfig,

    /// Extra TOML files to merge, in order (relative paths resolve against the
    /// config directory).
    #[serde(default)]
    pub include: Vec<PathBuf>,

    /// Directory with modular TOML files (defaults to `plugins.d` next to the
    /// config; relative paths resolve against the config directory).
    #[serde(default)]
    pub include_dir: Option<PathBuf>,

    /// Optional theme file with named colors.
    #[serde(default)]
    pub theme: Option<PathBuf>,
}

/// Bar-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarConfig {
    /// Bar height in points.
    #[serde(default = "default_bar_height")]
    pub height: f64,

    /// Vertical placement of the bar.
    #[serde(default)]
    pub position: BarPosition,

    /// Background color (hex `0xAARRGGBB` or `#RRGGBB`).
    #[serde(default = "default_bar_background")]
    pub background: String,

    /// Optional background blur radius.
    #[serde(default)]
    pub blur_radius: f64,

    /// Corner radius of the bar background.
    #[serde(default)]
    pub corner_radius: f64,

    /// Inner padding of the bar.
    #[serde(default = "default_bar_padding")]
    pub padding: EdgeInsets,

    /// Horizontal gap between items.
    #[serde(default = "default_item_spacing")]
    pub item_spacing: f64,

    /// Screen index for multi-monitor setups. `None` means main display.
    #[serde(default)]
    pub display: Option<usize>,

    /// Hide the bar at startup.
    #[serde(default)]
    pub hidden: bool,

    /// Extra top margin in points (useful on notched displays).
    #[serde(default)]
    pub top_margin: f64,

    /// Whether the bar spans the full screen width.
    #[serde(default = "default_true")]
    pub full_width: bool,

    /// Font used for labels when an item does not override it.
    #[serde(default)]
    pub font: Option<String>,

    /// Default font size for labels.
    #[serde(default = "default_font_size")]
    pub font_size: f64,

    /// Default foreground color for labels.
    #[serde(default = "default_foreground")]
    pub foreground: String,

    /// Default icon font used for icons when an item does not override it.
    #[serde(default)]
    pub icon_font: Option<String>,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            height: default_bar_height(),
            position: BarPosition::default(),
            background: default_bar_background(),
            blur_radius: 0.0,
            corner_radius: 0.0,
            padding: default_bar_padding(),
            item_spacing: default_item_spacing(),
            display: None,
            hidden: false,
            top_margin: 0.0,
            full_width: true,
            font: None,
            font_size: default_font_size(),
            foreground: default_foreground(),
            icon_font: None,
        }
    }
}

/// Vertical placement of the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    #[default]
    Top,
    Bottom,
}

/// Per-item configuration. All fields are optional in practice: unspecified
/// fields inherit from [`Config::defaults`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemConfig {
    /// Logical item kind (affects default rendering).
    #[serde(default)]
    pub kind: ItemKind,

    /// Which section of the bar the item belongs to.
    #[serde(default)]
    pub position: Position,

    /// Static label text.
    #[serde(default)]
    pub label: String,

    /// Label font family.
    #[serde(default)]
    pub label_font: Option<String>,

    /// Label font size.
    #[serde(default = "default_font_size")]
    pub label_font_size: f64,

    /// Label color.
    #[serde(default = "default_foreground")]
    pub label_color: String,

    /// Padding around the label.
    #[serde(default)]
    pub label_padding: EdgeInsets,

    /// Static icon text (Nerd Font glyph or SF Symbol name).
    #[serde(default)]
    pub icon: String,

    /// Icon font family.
    #[serde(default)]
    pub icon_font: Option<String>,

    /// Icon font size.
    #[serde(default = "default_font_size")]
    pub icon_font_size: f64,

    /// Icon color.
    #[serde(default = "default_foreground")]
    pub icon_color: String,

    /// Padding around the icon.
    #[serde(default)]
    pub icon_padding: EdgeInsets,

    /// Item background color.
    #[serde(default)]
    pub background: Option<String>,

    /// Item background corner radius.
    #[serde(default)]
    pub background_corner_radius: f64,

    /// Explicit item background height.
    #[serde(default)]
    pub background_height: Option<f64>,

    /// Padding between background and content.
    #[serde(default)]
    pub background_padding: EdgeInsets,

    /// Border color.
    #[serde(default)]
    pub border_color: Option<String>,

    /// Border width in points.
    #[serde(default)]
    pub border_width: f64,

    /// Fixed width in points (`0` means auto).
    #[serde(default)]
    pub width: f64,

    /// Minimum width in points.
    #[serde(default)]
    pub min_width: f64,

    /// Maximum width in points (`0` means unlimited).
    #[serde(default)]
    pub max_width: f64,

    /// Content alignment inside the item box.
    #[serde(default)]
    pub align: ItemAlign,

    /// Padding around the whole item.
    #[serde(default)]
    pub padding: EdgeInsets,

    /// Action bound to a left click.
    #[serde(default)]
    pub click_action: Option<MouseAction>,

    /// Action bound to scroll events.
    #[serde(default)]
    pub scroll_action: Option<MouseAction>,

    /// Action bound to mouse enter.
    #[serde(default)]
    pub enter_action: Option<MouseAction>,

    /// Action bound to mouse leave.
    #[serde(default)]
    pub leave_action: Option<MouseAction>,

    /// Optional shell command periodically invoked to update the item.
    #[serde(default)]
    pub script: Option<String>,

    /// Update interval for `script` in seconds.
    #[serde(default)]
    pub update_interval: Option<u64>,

    /// Plugin that owns this item.
    #[serde(default)]
    pub plugin: Option<String>,

    /// Arbitrary item properties forwarded to plugins.
    #[serde(default)]
    pub properties: BTreeMap<String, serde_json::Value>,

    /// Painting order (lower is painted first).
    #[serde(default)]
    pub drawing_order: i32,

    /// Whether the item is visible.
    #[serde(default = "default_true")]
    pub visible: bool,
}

/// Content alignment inside an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// Action performed in response to a mouse event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MouseAction {
    /// Action type.
    #[serde(rename = "type", default)]
    pub action_type: MouseActionType,

    /// Shell command to run (for `shell` actions).
    #[serde(default)]
    pub command: Option<String>,

    /// Event name to emit (for `trigger` actions).
    #[serde(default)]
    pub event: Option<String>,

    /// Optional event payload.
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

/// What a mouse binding does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseActionType {
    /// Execute a shell command.
    #[default]
    Shell,
    /// Emit a custom FluxBar event.
    Trigger,
    /// Send a JSON-RPC notification to a plugin.
    Plugin,
}

/// A single plugin entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    /// Plugin transport.
    #[serde(rename = "type", default)]
    pub plugin_type: PluginType,

    /// Executable path (for external / UDS plugins).
    #[serde(default)]
    pub path: Option<PathBuf>,

    /// Command used to start the plugin.
    #[serde(default)]
    pub command: Option<String>,

    /// Extra arguments appended to `command`.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables passed to the plugin.
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Working directory for the plugin process.
    #[serde(default)]
    pub working_dir: Option<PathBuf>,

    /// Event names this plugin subscribes to. `*` receives everything.
    #[serde(default)]
    pub subscribe: Vec<String>,

    /// How often (seconds) to invoke `on_tick`.
    #[serde(default)]
    pub tick_interval: Option<u64>,

    /// Restart the plugin automatically after a crash.
    #[serde(default = "default_true")]
    pub auto_restart: bool,

    /// Maximum restart attempts (`0` = unlimited).
    #[serde(default)]
    pub max_restarts: u32,

    /// Base delay between restarts, in seconds (exponential backoff).
    #[serde(default = "default_restart_delay")]
    pub restart_delay: u64,

    /// Plugin-specific settings.
    #[serde(default)]
    pub config: BTreeMap<String, serde_json::Value>,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            plugin_type: PluginType::default(),
            path: None,
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            working_dir: None,
            subscribe: Vec::new(),
            tick_interval: None,
            auto_restart: true,
            max_restarts: 0,
            restart_delay: default_restart_delay(),
            config: BTreeMap::new(),
        }
    }
}

/// Plugin transport types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginType {
    /// Line-delimited JSON over stdin/stdout.
    #[default]
    External,
    /// Line-delimited JSON over a Unix domain socket.
    Uds,
    /// WebAssembly module (reserved for v2).
    Wasm,
    /// Built into the FluxBar binary.
    Builtin,
}

/// Event configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventConfig {
    /// Timer definitions.
    #[serde(default)]
    pub timers: BTreeMap<String, TimerConfig>,

    /// Named custom events (used for documentation / validation).
    #[serde(default)]
    pub custom: BTreeMap<String, CustomEventConfig>,
}

/// A timer that periodically emits an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimerConfig {
    /// Interval in seconds.
    pub interval: u64,

    /// Event name emitted on each tick.
    pub event: String,

    /// Optional payload.
    #[serde(default)]
    pub payload: Option<serde_json::Value>,

    /// Whether this timer is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Custom event declaration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomEventConfig {
    #[serde(default)]
    pub description: String,
}

/// Animation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationConfig {
    /// Default duration in milliseconds.
    #[serde(default = "default_animation_duration")]
    pub duration: u64,

    /// Default easing function.
    #[serde(default)]
    pub easing: EasingFunction,

    /// Disable all animations (useful for tests / low power mode).
    #[serde(default)]
    pub disabled: bool,

    /// Per-property overrides.
    #[serde(default)]
    pub properties: BTreeMap<String, PropertyAnimation>,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            duration: default_animation_duration(),
            easing: EasingFunction::default(),
            disabled: false,
            properties: BTreeMap::new(),
        }
    }
}

/// Per-property animation override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyAnimation {
    #[serde(default)]
    pub duration: Option<u64>,
    #[serde(default)]
    pub easing: Option<EasingFunction>,
}

/// Supported easing functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EasingFunction {
    Linear,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    EaseInQuart,
    EaseOutQuart,
    EaseInOutQuart,
    EaseInQuint,
    EaseOutQuint,
    EaseInOutQuint,
    EaseInSine,
    EaseOutSine,
    EaseInOutSine,
    EaseInCirc,
    EaseOutCirc,
    EaseInOutCirc,
    EaseInExpo,
    EaseOutExpo,
    #[default]
    EaseInOutExpo,
    EaseInBack,
    EaseOutBack,
    EaseInOutBack,
    EaseInElastic,
    EaseOutElastic,
    EaseInOutElastic,
    EaseInBounce,
    EaseOutBounce,
    EaseInOutBounce,
}

fn default_true() -> bool {
    true
}

fn default_bar_height() -> f64 {
    32.0
}

fn default_bar_background() -> String {
    "0xCC1E1E2E".to_string()
}

fn default_bar_padding() -> EdgeInsets {
    EdgeInsets::horizontal(8.0)
}

fn default_item_spacing() -> f64 {
    6.0
}

fn default_font_size() -> f64 {
    13.0
}

fn default_foreground() -> String {
    "0xFFFFFFFF".to_string()
}

fn default_restart_delay() -> u64 {
    1
}

fn default_animation_duration() -> u64 {
    220
}

impl Config {
    /// Load a configuration file, merging explicit includes and `plugins.d`.
    ///
    /// Includes are merged at the TOML level, so partial files only override
    /// the keys they actually define.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let mut root = read_toml(path)?;

        // Explicit includes, relative to the main config.
        let includes: Vec<PathBuf> = root
            .get("include")
            .and_then(toml::Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();

        for include in includes {
            let include = if include.is_absolute() {
                include
            } else {
                base.join(include)
            };
            let value = read_toml(&include)?;
            deep_merge(&mut root, value);
        }

        let include_dir = root
            .get("include_dir")
            .and_then(toml::Value::as_str)
            .map(PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    base.join(path)
                }
            })
            .unwrap_or_else(|| base.join("plugins.d"));

        if include_dir.is_dir() {
            let pattern = include_dir.join("*.toml").to_string_lossy().to_string();
            let mut files: Vec<_> = glob::glob(&pattern)
                .map_err(|e| FluxError::Config(format!("invalid include glob: {e}")))?
                .filter_map(std::result::Result::ok)
                .collect();
            files.sort();
            for file in files {
                let value = read_toml(&file)?;
                deep_merge(&mut root, value);
            }
        }

        let config: Self = root
            .try_into()
            .map_err(|e| FluxError::Config(format!("cannot parse {}: {e}", path.display())))?;
        config.validate()?;
        Ok(config)
    }

    /// Load the default configuration, or a built-in default when no file exists.
    pub fn load_default() -> Result<Self> {
        let Some(paths) = config_paths() else {
            return Ok(Self::default());
        };

        if paths.config_file.exists() {
            Self::load(&paths.config_file)
        } else {
            Ok(Self::default())
        }
    }

    /// Path to the primary configuration file.
    pub fn default_path() -> Option<PathBuf> {
        config_paths().map(|p| p.config_file)
    }

    /// Path to the configuration directory.
    pub fn default_dir() -> Option<PathBuf> {
        config_paths().map(|p| p.config_dir)
    }

    /// Validate semantic constraints that serde cannot express.
    pub fn validate(&self) -> Result<()> {
        if !(0.0..=256.0).contains(&self.bar.height) {
            return Err(FluxError::Config(
                "bar.height must be between 0 and 256".into(),
            ));
        }
        if self.bar.item_spacing < 0.0 {
            return Err(FluxError::Config("bar.item_spacing must be >= 0".into()));
        }
        for (name, item) in &self.items {
            if name.trim().is_empty() {
                return Err(FluxError::Config("item names must not be empty".into()));
            }
            if item.width < 0.0 || item.min_width < 0.0 || item.max_width < 0.0 {
                return Err(FluxError::Config(format!(
                    "item `{name}` has negative width"
                )));
            }
            if item.max_width > 0.0 && item.max_width < item.min_width {
                return Err(FluxError::Config(format!(
                    "item `{name}` has max_width smaller than min_width"
                )));
            }
        }
        for (name, timer) in &self.events.timers {
            if timer.interval == 0 {
                return Err(FluxError::Config(format!(
                    "timer `{name}` interval must be > 0"
                )));
            }
        }
        for (name, plugin) in &self.plugins {
            if plugin.command.is_none()
                && plugin.path.is_none()
                && plugin.plugin_type != PluginType::Builtin
            {
                return Err(FluxError::Config(format!(
                    "plugin `{name}` needs either `command` or `path`"
                )));
            }
        }
        Ok(())
    }

    /// Effective configuration for an item, with defaults applied.
    pub fn item(&self, name: &str) -> ItemConfig {
        let mut config = self.defaults.clone();
        if let Some(item) = self.items.get(name) {
            merge_item(&mut config, item);
        }
        config
    }

    /// Serialize the configuration back to TOML.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

fn merge_item(base: &mut ItemConfig, over: &ItemConfig) {
    // Start from the override and fill in unset fields from defaults.
    let defaults = base.clone();
    let mut merged = over.clone();

    if merged.label.is_empty() {
        merged.label = defaults.label;
    }
    if merged.icon.is_empty() {
        merged.icon = defaults.icon;
    }
    if merged.icon_font.is_none() {
        merged.icon_font = defaults.icon_font;
    }
    if merged.label_font.is_none() {
        merged.label_font = defaults.label_font;
    }
    if merged.label_font_size == default_font_size() {
        merged.label_font_size = defaults.label_font_size;
    }
    if merged.icon_font_size == default_font_size() {
        merged.icon_font_size = defaults.icon_font_size;
    }
    if merged.label_color == default_foreground() {
        merged.label_color = defaults.label_color;
    }
    if merged.icon_color == default_foreground() {
        merged.icon_color = defaults.icon_color;
    }
    if merged.background.is_none() {
        merged.background = defaults.background;
    }
    if merged.width == 0.0 {
        merged.width = defaults.width;
    }
    if merged.min_width == 0.0 {
        merged.min_width = defaults.min_width;
    }
    if merged.max_width == 0.0 {
        merged.max_width = defaults.max_width;
    }
    if merged.update_interval.is_none() {
        merged.update_interval = defaults.update_interval;
        if merged.script.is_none() {
            merged.script = defaults.script;
        }
    }

    *base = merged;
}

fn read_toml(path: &Path) -> Result<toml::Value> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| FluxError::Config(format!("cannot read {}: {e}", path.display())))?;
    toml::from_str(&content)
        .map_err(|e| FluxError::Config(format!("cannot parse {}: {e}", path.display())))
}

/// Recursively merge TOML tables. Scalar values from `over` win.
fn deep_merge(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(base_table), toml::Value::Table(over_table)) => {
            for (key, value) in over_table {
                match base_table.get_mut(&key) {
                    Some(existing) => deep_merge(existing, value),
                    None => {
                        base_table.insert(key, value);
                    }
                }
            }
        }
        (base, over) => *base = over,
    }
}

struct ConfigPaths {
    config_dir: PathBuf,
    config_file: PathBuf,
}
fn config_paths() -> Option<ConfigPaths> {
    let config_dir = if let Some(dir) = std::env::var_os("FLUXBAR_CONFIG_DIR") {
        PathBuf::from(dir)
    } else if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty())
    {
        PathBuf::from(xdg).join("fluxbar")
    } else {
        PathBuf::from(std::env::var_os("HOME")?)
            .join(".config")
            .join("fluxbar")
    };
    let config_file = config_dir.join("fluxbar.toml");
    Some(ConfigPaths {
        config_dir,
        config_file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, contents: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[test]
    fn parses_full_config() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "fluxbar.toml",
            r##"
[bar]
height = 40
position = "top"
background = "0xFF000000"

[defaults]
label_font_size = 14.0
icon_font_size = 16.0

[items.cpu]
position = "left"
label = "CPU"
icon = ""

[items.memory]
position = "right"
label = "RAM"
icon = ""

[plugins.sysmon]
type = "external"
command = "fluxbar-plugin-sysmon"
subscribe = ["tick"]

[events.timers.tick]
interval = 1
event = "tick"

[animations]
duration = 150
easing = "ease_out_expo"
"##,
        );

        let config = Config::load(&path).unwrap();
        assert_eq!(config.bar.height, 40.0);
        assert_eq!(config.defaults.label_font_size, 14.0);
        assert_eq!(config.items.len(), 2);
        assert_eq!(config.plugins.len(), 1);
        assert_eq!(config.animations.duration, 150);
    }

    #[test]
    fn merges_include_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("plugins.d")).unwrap();
        write(
            &dir,
            "fluxbar.toml",
            r#"
[bar]
height = 30

[items.a]
label = "A"
"#,
        );
        write(
            &dir,
            "plugins.d/extra.toml",
            r#"
[items.b]
label = "B"
"#,
        );

        let config = Config::load(dir.path().join("fluxbar.toml")).unwrap();
        assert_eq!(config.items.len(), 2);
        assert!(config.items.contains_key("a"));
        assert!(config.items.contains_key("b"));
    }

    #[test]
    fn merges_relative_include_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("modules")).unwrap();
        write(
            &dir,
            "fluxbar.toml",
            r#"
include_dir = "modules"

[items.a]
label = "A"
"#,
        );
        write(
            &dir,
            "modules/extra.toml",
            r#"
[items.b]
label = "B"
"#,
        );

        let config = Config::load(dir.path().join("fluxbar.toml")).unwrap();
        assert_eq!(config.items.len(), 2);
        assert!(config.items.contains_key("b"));
    }

    #[test]
    fn rejects_invalid_timer() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "fluxbar.toml",
            r#"
[events.timers.bad]
interval = 0
event = "tick"
"#,
        );
        assert!(Config::load(path).is_err());
    }

    #[test]
    fn parses_mouse_actions() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "fluxbar.toml",
            r#"
[items.media]
label = "Media"

[items.media.click_action]
type = "shell"
command = "open -a Music"
"#,
        );
        let config = Config::load(path).unwrap();
        let action = config.items["media"].click_action.as_ref().unwrap();
        assert_eq!(action.action_type, MouseActionType::Shell);
        assert_eq!(action.command.as_deref(), Some("open -a Music"));
    }
}
