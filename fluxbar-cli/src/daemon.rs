//! Daemon orchestration: wires state, IPC, plugins, system events and the UI.

use crate::default_config;
use crate::handler::DaemonHandler;
use crate::plugin_manager;
use crate::scripts::{self, ScriptRunner};
use crate::sketchybar;
use anyhow::{Context, Result};
use fluxbar_core::builtin;
use fluxbar_core::config::{Config, PluginType};
use fluxbar_core::event::Event;
use fluxbar_core::ipc::IpcServer;
use fluxbar_core::item::ItemOrigin;
use fluxbar_core::plugin::{PluginHost, PluginSpec};
use fluxbar_core::renderer::{Renderer, layout_with_metrics};
use fluxbar_core::state::StateHandle;
use fluxbar_core::system::{SystemConfig, SystemMonitor};
use fluxbar_macos::{MacRenderer, UiCommand, UiEvent};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// PID file next to the socket. A custom socket therefore gets its own
/// independently locked instance.
fn pid_path_for_socket(socket: &Path) -> PathBuf {
    socket.with_extension("pid")
}

/// An advisory-locked PID file. The lock is released automatically if the
/// process dies, so a leftover file is detected as stale on the next start.
struct PidFile {
    path: PathBuf,
    file: fs::File,
}

impl PidFile {
    fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() {
                anyhow::bail!("refusing to use symlinked PID file {}", path.display());
            }
        }

        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        if let Err(error) = fs2::FileExt::try_lock_exclusive(&file) {
            let pid = fs::read_to_string(path).unwrap_or_default();
            let pid = pid.trim();
            if pid.is_empty() {
                return Err(error).with_context(|| format!("failed to lock {}", path.display()));
            }
            anyhow::bail!(
                "another FluxBar instance is running (pid {pid}, pid file {})",
                path.display()
            );
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("failed to secure {}", path.display()))?;
        }

        file.set_len(0)
            .with_context(|| format!("failed to truncate {}", path.display()))?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_all()?;

        Ok(Self {
            path: path.to_path_buf(),
            file,
        })
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

/// Start the daemon. Blocks the main thread while the native UI is running.
pub fn run(config_path: Option<PathBuf>, socket: PathBuf, headless: bool) -> Result<()> {
    let _pid_file = PidFile::acquire(&pid_path_for_socket(&socket))?;
    let (config, used_path) = load_config(config_path)?;

    let config_dir = used_path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(Config::default_dir);
    let compat_dir =
        config_dir
            .as_deref()
            .and_then(|directory| match sketchybar::ensure_shims(directory) {
                Ok(path) => Some(path),
                Err(error) => {
                    warn!(%error, "could not create sketchybar compatibility shims");
                    None
                }
            });

    let (screen_width, screen_height) = if headless {
        (1920.0, 1080.0)
    } else {
        fluxbar_macos::main_screen_size().unwrap_or((1920.0, 1080.0))
    };

    let (renderer, command_rx, event_tx) = MacRenderer::new(screen_width, screen_height);
    let renderer = Arc::new(renderer);
    let ui_events = renderer
        .take_events()
        .context("failed to take UI event receiver")?;

    let bar_height = config.bar.height;
    let shutdown = Arc::new(AtomicBool::new(false));
    let frames = Arc::new(AtomicU64::new(0));

    let renderer_for_daemon = renderer.clone();
    let shutdown_for_daemon = shutdown.clone();
    let frames_for_daemon = frames.clone();
    let socket_for_daemon = socket.clone();

    let daemon_thread = std::thread::Builder::new()
        .name("fluxbar-daemon".into())
        .spawn(move || -> Result<()> {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;

            runtime.block_on(async_main(
                config,
                used_path,
                socket_for_daemon,
                compat_dir,
                renderer_for_daemon,
                ui_events,
                shutdown_for_daemon,
                frames_for_daemon,
            ))
        })
        .context("failed to spawn daemon thread")?;

    info!(socket = %socket.display(), headless, "starting FluxBar");

    let ui_result = if headless {
        // Keep the event channel alive so the daemon does not interpret the
        // missing UI as a disconnect.
        let _keepalive = event_tx;
        fluxbar_macos::run_headless(command_rx).map_err(anyhow::Error::msg)
    } else {
        fluxbar_macos::run_bar(screen_width, bar_height, command_rx, event_tx)
            .map_err(anyhow::Error::msg)
    };

    shutdown.store(true, Ordering::Relaxed);

    match daemon_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(error),
        Err(_) => anyhow::bail!("daemon thread panicked"),
    }

    ui_result?;
    println!("FluxBar stopped");
    Ok(())
}

fn load_config(config_path: Option<PathBuf>) -> Result<(Config, Option<PathBuf>)> {
    match config_path {
        Some(path) => {
            let config = Config::load(&path)
                .with_context(|| format!("failed to load {}", path.display()))?;
            Ok((config, Some(path)))
        }
        None => {
            let Some(path) = Config::default_path() else {
                warn!("could not determine the configuration directory, using built-in defaults");
                return Ok((Config::default(), None));
            };

            if let Err(error) = bootstrap_default_layout(&path) {
                warn!(%error, path = %path.display(), "could not prepare the default configuration");
            }

            if path.exists() {
                let config = Config::load(&path)
                    .with_context(|| format!("failed to load {}", path.display()))?;
                Ok((config, Some(path)))
            } else {
                warn!("no configuration file found, using built-in defaults");
                Ok((Config::default(), Some(path)))
            }
        }
    }
}

/// Create `~/.config/fluxbar`, its plugin directories, and a default config
/// on first start. Existing files are never overwritten.
fn bootstrap_default_layout(config_path: &Path) -> Result<()> {
    let Some(directory) = config_path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(directory)?;
    plugin_manager::ensure_layout(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }

    if config_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Existing installs may predate the private-config policy. Only
            // change a regular file; a user-managed symlink is left alone.
            if let Ok(metadata) = fs::symlink_metadata(config_path) {
                if metadata.file_type().is_file() {
                    fs::set_permissions(config_path, fs::Permissions::from_mode(0o600))?;
                }
            }
        }
        return Ok(());
    }

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // The file may contain plugin environment variables and shell
        // commands; keep it private to the current user.
        options.mode(0o600);
    }
    match options.open(config_path) {
        Ok(mut file) => {
            file.write_all(default_config::DEFAULT_CONFIG.as_bytes())?;
            info!(path = %config_path.display(), "created default configuration");
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another process won the race; use the file it created.
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Result of applying a validated configuration to the running daemon.
#[derive(Debug, Default, Serialize)]
pub(crate) struct ReloadReport {
    pub added_items: usize,
    pub updated_items: usize,
    pub removed_items: usize,
    pub started_plugins: usize,
    pub stopped_plugins: usize,
    pub timers: usize,
    pub monitor_restarted: bool,
}

/// Owns runtime resources that depend on configuration: timers, the system
/// monitor, plugin processes, and the bar visibility.
pub(crate) struct RuntimeReload {
    state: StateHandle,
    plugins: Arc<PluginHost>,
    renderer: Arc<MacRenderer>,
    bar_visible: Arc<AtomicBool>,
    config_path: Option<PathBuf>,
    reload_lock: Mutex<()>,
    timer_tasks: Mutex<Vec<JoinHandle<()>>>,
    monitor_task: Mutex<Option<JoinHandle<()>>>,
    monitor_config: Mutex<Option<SystemConfig>>,
}

impl RuntimeReload {
    fn new(
        state: StateHandle,
        plugins: Arc<PluginHost>,
        renderer: Arc<MacRenderer>,
        bar_visible: Arc<AtomicBool>,
        config_path: Option<PathBuf>,
    ) -> Self {
        Self {
            state,
            plugins,
            renderer,
            bar_visible,
            config_path,
            reload_lock: Mutex::new(()),
            timer_tasks: Mutex::new(Vec::new()),
            monitor_task: Mutex::new(None),
            monitor_config: Mutex::new(None),
        }
    }

    fn start_timers(&self, config: &Config) -> usize {
        let tasks: Vec<JoinHandle<()>> = config
            .events
            .timers
            .iter()
            .filter(|(_, timer)| timer.enabled)
            .map(|(name, timer)| {
                let state = self.state.clone();
                let event_name = timer.event.clone();
                let payload = timer.payload.clone().unwrap_or(serde_json::Value::Null);
                let interval = Duration::from_secs(timer.interval.max(1));
                let timer_name = name.clone();
                tokio::spawn(async move {
                    let mut ticker = tokio::time::interval(interval);
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        ticker.tick().await;
                        state.emit(
                            Event::custom(event_name.clone(), payload.clone())
                                .with_source(format!("timer:{timer_name}")),
                        );
                    }
                })
            })
            .collect();

        let count = tasks.len();
        let old = std::mem::replace(&mut *self.timer_tasks.lock().unwrap(), tasks);
        for task in old {
            task.abort();
        }
        count
    }

    fn start_monitor(&self, config: &Config) -> bool {
        let unchanged = {
            let task = self.monitor_task.lock().unwrap();
            let monitor_config = self.monitor_config.lock().unwrap();
            task.is_some() && monitor_config.as_ref() == Some(&config.system)
        };
        if unchanged {
            return false;
        }

        if let Some(task) = self.monitor_task.lock().unwrap().take() {
            task.abort();
        }
        let monitor = SystemMonitor::new(self.state.clone(), config.system.clone());
        *self.monitor_task.lock().unwrap() = Some(monitor.spawn());
        *self.monitor_config.lock().unwrap() = Some(config.system.clone());
        true
    }

    /// Validate and apply a configuration file as one operation.
    ///
    /// The file is fully parsed and validated before any runtime state is
    /// touched. Items, plugins, timers, the system monitor, and bar
    /// visibility are then reconciled against the new configuration.
    pub(crate) fn reload(&self) -> fluxbar_core::error::Result<ReloadReport> {
        // Reloads can be triggered by both the event bus and an IPC client at
        // the same time. Serialize the whole reconcile so validation, state
        // replacement and plugin changes are one transaction.
        let _reload_guard = self.reload_lock.lock().unwrap();
        let path = self.config_path.as_ref().ok_or_else(|| {
            fluxbar_core::error::FluxError::Config(
                "no configuration file is in use; pass --config to enable reload".into(),
            )
        })?;
        let config = Config::load(path)?;

        let mut report = reconcile_items(&self.state, &config);
        self.state.replace_config(config.clone());

        let specs: BTreeMap<String, PluginSpec> = config
            .plugins
            .iter()
            .filter(|(_, plugin)| matches!(plugin.plugin_type, PluginType::External))
            .map(|(name, plugin)| (name.clone(), PluginSpec::new(name.clone(), plugin.clone())))
            .collect();
        let (started, stopped) = self.plugins.reconcile(specs);
        report.started_plugins = started;
        report.stopped_plugins = stopped;

        report.timers = self.start_timers(&config);
        report.monitor_restarted = self.start_monitor(&config);

        let visible = !config.bar.hidden;
        self.bar_visible.store(visible, Ordering::Relaxed);
        self.renderer.set_visible(visible)?;

        Ok(report)
    }

    fn shutdown(&self) {
        for task in self.timer_tasks.lock().unwrap().drain(..) {
            task.abort();
        }
        if let Some(task) = self.monitor_task.lock().unwrap().take() {
            task.abort();
        }
    }
}

/// Reconcile item existence and static fields against a validated config.
fn reconcile_items(state: &StateHandle, config: &Config) -> ReloadReport {
    let mut report = ReloadReport::default();
    let mut guard = state.write();

    let runtime_names: Vec<String> = guard
        .items
        .iter()
        .filter(|(_, item)| item.origin == ItemOrigin::Runtime)
        .map(|(name, _)| name.clone())
        .collect();

    let existing: Vec<String> = guard.items.keys().cloned().collect();
    for name in existing {
        let Some(raw) = config.items.get(&name) else {
            if guard
                .items
                .get(&name)
                .is_some_and(|item| item.origin == ItemOrigin::Config)
            {
                guard.items.remove(&name);
                report.removed_items += 1;
            }
            continue;
        };
        if runtime_names.contains(&name) {
            // A runtime-created item owns its name; config reloads leave it
            // untouched instead of silently replacing user state.
            continue;
        }
        let effective = config.item(&name);
        if let Some(item) = guard.items.get_mut(&name) {
            let runtime_label = item.state.label.clone();
            let runtime_icon = item.state.icon.clone();
            item.kind = effective.kind;
            item.position = effective.position;
            item.config = merge_item_config(&item.config, &effective, raw);
            if !raw.label.is_empty() {
                item.state.label = effective.label.clone();
            } else {
                item.state.label = runtime_label;
            }
            if !raw.icon.is_empty() {
                item.state.icon = effective.icon.clone();
            } else {
                item.state.icon = runtime_icon;
            }
            item.state.visible = effective.visible;
            report.updated_items += 1;
        }
    }

    for name in config.items.keys() {
        if guard.items.contains_key(name) {
            continue;
        }
        let effective = config.item(name);
        let mut item = fluxbar_core::Item::new(name.clone(), effective);
        item.origin = ItemOrigin::Config;
        item.state.visible = item.config.visible;
        guard.items.insert(name.clone(), item);
        report.added_items += 1;
        report.updated_items += 1;
    }

    guard.next_version();
    report
}

#[allow(clippy::too_many_arguments)]
async fn async_main(
    config: Config,
    config_path: Option<PathBuf>,
    socket: PathBuf,
    compat_dir: Option<PathBuf>,
    renderer: Arc<MacRenderer>,
    ui_events: std::sync::mpsc::Receiver<UiEvent>,
    shutdown: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
) -> Result<()> {
    let state = StateHandle::new(config.clone());
    let plugins = Arc::new(PluginHost::new(state.clone()));
    let bar_visible = Arc::new(AtomicBool::new(!config.bar.hidden));

    let script_context = scripts::context_for(config_path.as_deref(), &socket, compat_dir);
    plugins.set_script_context(script_context.clone());

    let runtime_reload = Arc::new(RuntimeReload::new(
        state.clone(),
        plugins.clone(),
        renderer.clone(),
        bar_visible.clone(),
        config_path.clone(),
    ));
    runtime_reload.start_timers(&config);
    runtime_reload.start_monitor(&config);

    // Signals only flip the shutdown flag; the event loop then performs the
    // normal graceful shutdown path and tells the UI thread to exit.
    let signal_shutdown = shutdown.clone();
    #[cfg(unix)]
    let signal_task = {
        use tokio::signal::unix::{SignalKind, signal};
        let mut interrupt =
            signal(SignalKind::interrupt()).context("failed to install SIGINT handler")?;
        let mut terminate =
            signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?;
        tokio::spawn(async move {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
            info!("received shutdown signal");
            signal_shutdown.store(true, Ordering::Relaxed);
        })
    };
    #[cfg(not(unix))]
    let signal_task = {
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!("received shutdown signal");
                signal_shutdown.store(true, Ordering::Relaxed);
            }
        })
    };

    // Subscribe to the bus before starting event sources.
    let mut bus = state.bus().subscribe();

    // IPC server must be available before scripts can call the CLI.
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).ok();
        // The default socket lives in a cache directory shared with nothing
        // else. Keep it private; custom socket paths are left to the user.
        if socket == fluxbar_core::ipc::default_socket_path() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(error) =
                    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                {
                    warn!(%error, "could not restrict IPC directory permissions");
                }
            }
        }
    }
    let handler = Arc::new(DaemonHandler {
        state: state.clone(),
        plugins: plugins.clone(),
        renderer: renderer.clone(),
        started: Instant::now(),
        frames: frames.clone(),
        shutdown: shutdown.clone(),
        config_path: config_path.clone(),
        bar_visible: bar_visible.clone(),
        reload: Some(runtime_reload.clone()),
    });
    let server = IpcServer::bind(&socket, handler)
        .with_context(|| format!("failed to bind {}", socket.display()))?;
    let server_task = tokio::spawn(async move {
        if let Err(error) = server.run().await {
            tracing::error!(%error, "IPC server stopped");
        }
    });

    // Start plugins and item scripts after IPC is ready.
    plugins.start_all();
    let script_runner = ScriptRunner::new(state.clone(), script_context).spawn();

    let mut render_tick = tokio::time::interval(Duration::from_millis(16));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut clock_tick = tokio::time::interval(Duration::from_secs(1));
    clock_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut last_version = u64::MAX;
    if config.bar.hidden {
        renderer.set_visible(false)?;
    }

    info!("daemon ready");

    loop {
        tokio::select! {
            _ = render_tick.tick() => {
                // Drain UI events.
                loop {
                    match ui_events.try_recv() {
                        Ok(event) => handle_ui_event(&state, event).await,
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            info!("UI thread disconnected");
                            shutdown.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }

                let version = state.read().version;
                if version != last_version {
                    last_version = version;
                    let frame = {
                        let state_guard = state.read();
                        let config_guard = state.config();
                        layout_with_metrics(
                            &state_guard,
                            &config_guard,
                            renderer.screen_size(),
                            renderer.as_ref(),
                        )
                    };
                    // Keep the bar height/visibility in sync with config changes.
                    if bar_visible.load(Ordering::Relaxed) {
                        renderer.render(&frame)?;
                    }
                    frames.fetch_add(1, Ordering::Relaxed);
                }

                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
            }
            event = bus.recv() => {
                match event {
                    Ok(event) => {
                        builtin::apply_event(&state, &event);
                        match event.name() {
                            "config_reload" if event.source != "core" => {
                                match runtime_reload.reload() {
                                    Ok(report) => {
                                        info!(
                                            added_items = report.added_items,
                                            removed_items = report.removed_items,
                                            started_plugins = report.started_plugins,
                                            stopped_plugins = report.stopped_plugins,
                                            "configuration reloaded"
                                        );
                                    }
                                    Err(error) => {
                                        warn!(%error, "configuration reload failed");
                                    }
                                }
                            }
                            "bar_toggle" => {
                                let visible = !bar_visible.fetch_xor(true, Ordering::Relaxed);
                                renderer.set_visible(visible)?;
                                if visible {
                                    // The bar may have been hidden while state
                                    // changed; force a fresh frame before showing.
                                    last_version = u64::MAX;
                                }
                            }
                            "bar_show" => {
                                bar_visible.store(true, Ordering::Relaxed);
                                renderer.set_visible(true)?;
                                last_version = u64::MAX;
                            }
                            "bar_hide" => {
                                bar_visible.store(false, Ordering::Relaxed);
                                renderer.set_visible(false)?;
                            }
                            _ => {}
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "daemon event subscriber lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = clock_tick.tick() => {
                builtin::refresh_clock(&state);
            }
        }
    }

    info!("shutting down");
    signal_task.abort();
    runtime_reload.shutdown();
    script_runner.abort();
    server_task.abort();
    plugins.shutdown_all().await;
    let _ = renderer.send(UiCommand::Shutdown);
    Ok(())
}

/// Preserve runtime tweaks when a config reload only sets a subset of fields.
///
/// `effective` has the new defaults applied, while `raw` is the item's section
/// from the file. Styling comes from `effective` so inherited Liquid Glass
/// properties are refreshed, while runtime label/icon/background tweaks are
/// kept when the file does not specify them.
fn merge_item_config(
    base: &fluxbar_core::config::ItemConfig,
    effective: &fluxbar_core::config::ItemConfig,
    raw: &fluxbar_core::config::ItemConfig,
) -> fluxbar_core::config::ItemConfig {
    let mut merged = effective.clone();
    if raw.label.is_empty() {
        merged.label = base.label.clone();
    }
    if raw.icon.is_empty() {
        merged.icon = base.icon.clone();
    }
    if raw.background.is_none() {
        merged.background = base.background.clone();
    }
    merged
}

async fn handle_ui_event(state: &StateHandle, event: UiEvent) {
    let (event_name, payload) = match &event {
        UiEvent::Click {
            item,
            button,
            modifier,
        } => (
            "mouse.clicked",
            json!({ "item": item, "button": button, "modifier": modifier }),
        ),
        UiEvent::Scroll { item, delta } => {
            ("mouse.scrolled", json!({ "item": item, "delta": delta }))
        }
        UiEvent::Enter { item } => ("mouse.entered", json!({ "item": item })),
        UiEvent::Leave { item } => ("mouse.left", json!({ "item": item })),
    };
    state.emit(Event::custom(event_name, payload).with_source("ui"));

    let (item_name, action) = match event {
        UiEvent::Click { item, .. } => (
            item.clone(),
            state.item(&item).and_then(|i| i.config.click_action),
        ),
        UiEvent::Scroll { item, .. } => (
            item.clone(),
            state.item(&item).and_then(|i| i.config.scroll_action),
        ),
        UiEvent::Enter { item } => (
            item.clone(),
            state.item(&item).and_then(|i| i.config.enter_action),
        ),
        UiEvent::Leave { item } => (
            item.clone(),
            state.item(&item).and_then(|i| i.config.leave_action),
        ),
    };

    let Some(action) = action else {
        return;
    };

    match action.action_type {
        fluxbar_core::config::MouseActionType::Shell => {
            if let Some(command) = action.command {
                info!(item = %item_name, %command, "running click action");
                let result = tokio::process::Command::new("/bin/sh")
                    .arg("-c")
                    .arg(&command)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                if let Err(error) = result {
                    warn!(%error, "failed to spawn click action");
                }
            }
        }
        fluxbar_core::config::MouseActionType::Trigger => {
            if let Some(name) = action.event {
                state.emit(
                    Event::custom(name, action.payload.unwrap_or(json!({})))
                        .with_source(format!("ui:{item_name}")),
                );
            }
        }
        fluxbar_core::config::MouseActionType::Plugin => {
            warn!(item = %item_name, "plugin mouse actions are not implemented yet");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn bootstrap_creates_config_and_plugin_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fluxbar.toml");

        bootstrap_default_layout(&path).unwrap();

        assert!(path.exists());
        assert!(dir.path().join("plugins").is_dir());
        assert!(dir.path().join("plugins.d").is_dir());
        let config = Config::load(&path).unwrap();
        assert!(!config.items.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn pid_file_is_private_locked_and_cleaned_up() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fluxbar.pid");

        let pid_file = PidFile::acquire(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap().trim(),
            std::process::id().to_string()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(PidFile::acquire(&path).is_err());

        drop(pid_file);
        assert!(!path.exists());
        let pid_file = PidFile::acquire(&path).unwrap();
        drop(pid_file);
    }

    #[test]
    fn stale_pid_file_is_reused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fluxbar.pid");
        fs::write(&path, "999999\n").unwrap();

        let _pid_file = PidFile::acquire(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap().trim(),
            std::process::id().to_string()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_pid_file_is_rejected() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "do not touch\n").unwrap();
        let path = dir.path().join("fluxbar.pid");
        std::os::unix::fs::symlink(&target, &path).unwrap();

        assert!(PidFile::acquire(&path).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "do not touch\n");
    }

    #[test]
    fn bootstrap_does_not_overwrite_existing_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fluxbar.toml");
        fs::write(&path, "[bar]\nheight = 41\n").unwrap();

        bootstrap_default_layout(&path).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "[bar]\nheight = 41\n");
        assert!(dir.path().join("plugins").is_dir());
        assert!(dir.path().join("plugins.d").is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn reconcile_items_removes_config_items_and_preserves_runtime_items() {
        use fluxbar_core::config::ItemConfig;
        use fluxbar_core::geometry::Position;
        use fluxbar_core::item::ItemKind;

        let mut initial = Config::default();
        initial.items.insert("a".into(), ItemConfig::default());
        initial.items.insert("b".into(), ItemConfig::default());
        let state = StateHandle::new(initial);
        state
            .add_item("runtime", ItemKind::Text, Position::Center)
            .unwrap();
        state
            .update_item(
                "a",
                fluxbar_core::item::ItemUpdate {
                    label: Some("runtime label".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        let mut next = Config::default();
        next.items.insert(
            "a".into(),
            ItemConfig {
                label: "configured".into(),
                kind: ItemKind::Progress,
                position: Position::Right,
                ..ItemConfig::default()
            },
        );
        next.items.insert("c".into(), ItemConfig::default());

        let report = reconcile_items(&state, &next);
        let snapshot = state.read();
        assert_eq!(report.added_items, 1);
        assert_eq!(report.removed_items, 1);
        assert!(!snapshot.items.contains_key("b"));
        assert!(snapshot.items.contains_key("runtime"));
        assert_eq!(
            snapshot.items.get("runtime").unwrap().origin,
            ItemOrigin::Runtime
        );
        let a = snapshot.items.get("a").unwrap();
        assert_eq!(a.state.label, "configured");
        assert_eq!(a.kind, ItemKind::Progress);
        assert_eq!(a.position, Position::Right);
        assert!(snapshot.items.contains_key("c"));
    }

    #[tokio::test]
    async fn runtime_reload_applies_items_timers_monitor_and_bar() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fluxbar.toml");
        fs::write(
            &path,
            r#"
[bar]
height = 30

[items.a]
label = "old"
"#,
        )
        .unwrap();
        let initial = Config::load(&path).unwrap();
        let state = StateHandle::new(initial.clone());
        let plugins = Arc::new(PluginHost::new(state.clone()));
        let (renderer, _commands, _events) = MacRenderer::new(800.0, 600.0);
        let renderer = Arc::new(renderer);
        let bar_visible = Arc::new(AtomicBool::new(true));
        let runtime = RuntimeReload::new(
            state.clone(),
            plugins,
            renderer,
            bar_visible.clone(),
            Some(path.clone()),
        );
        runtime.start_timers(&initial);
        runtime.start_monitor(&initial);

        fs::write(
            &path,
            r#"
[bar]
height = 30
hidden = true

[system]
poll_interval = 7

[items.a]
label = "new"
kind = "progress"
position = "right"

[items.c]
label = "added"

[events.timers.second]
interval = 1
event = "tick"
enabled = true
"#,
        )
        .unwrap();

        let report = runtime.reload().unwrap();
        assert_eq!(report.added_items, 1);
        assert_eq!(report.removed_items, 0);
        assert_eq!(report.timers, 1);
        assert!(report.monitor_restarted);
        assert!(!bar_visible.load(Ordering::Relaxed));
        let snapshot = state.read();
        assert_eq!(snapshot.items.get("a").unwrap().state.label, "new");
        assert_eq!(
            snapshot.items.get("a").unwrap().kind,
            fluxbar_core::item::ItemKind::Progress
        );
        assert!(snapshot.items.contains_key("c"));
        drop(snapshot);
        assert_eq!(state.config().bar.height, 30.0);
        runtime.shutdown();
    }

    #[test]
    fn reload_keeps_inherited_styles_and_runtime_text() {
        use fluxbar_core::config::ItemConfig;
        use fluxbar_core::geometry::EdgeInsets;

        let mut config = Config::default();
        config.defaults.background_corner_radius = 13.0;
        config.defaults.background_blur_radius = 14.0;
        config.defaults.background_height = Some(30.0);
        config.defaults.background_padding = EdgeInsets::new(0.0, 0.0, 7.0, 0.0);
        config.defaults.border_color = Some("0x26FFFFFF".into());
        config.defaults.border_width = 1.0;
        config.defaults.padding = EdgeInsets::horizontal(8.0);
        config.items.insert("cpu".into(), ItemConfig::default());

        let effective = config.item("cpu");
        let raw = config.items.get("cpu").unwrap();
        let runtime = ItemConfig {
            label: "runtime label".into(),
            icon: "cpu".into(),
            background: Some("0xFF112233".into()),
            ..ItemConfig::default()
        };

        let merged = merge_item_config(&runtime, &effective, raw);

        assert_eq!(merged.label, "runtime label");
        assert_eq!(merged.icon, "cpu");
        assert_eq!(merged.background.as_deref(), Some("0xFF112233"));
        assert_eq!(merged.background_corner_radius, 13.0);
        assert_eq!(merged.background_blur_radius, 14.0);
        assert_eq!(merged.background_height, Some(30.0));
        assert_eq!(
            merged.background_padding,
            EdgeInsets::new(0.0, 0.0, 7.0, 0.0)
        );
        assert_eq!(merged.border_color.as_deref(), Some("0x26FFFFFF"));
        assert_eq!(merged.border_width, 1.0);
        assert_eq!(merged.padding, EdgeInsets::horizontal(8.0));
    }
}
