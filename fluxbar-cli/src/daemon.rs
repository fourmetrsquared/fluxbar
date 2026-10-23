//! Daemon orchestration: wires state, IPC, plugins, system events and the UI.

use crate::handler::DaemonHandler;
use anyhow::{Context, Result};
use fluxbar_core::builtin;
use fluxbar_core::config::Config;
use fluxbar_core::event::Event;
use fluxbar_core::ipc::IpcServer;
use fluxbar_core::plugin::PluginHost;
use fluxbar_core::renderer::{Renderer, layout};
use fluxbar_core::state::StateHandle;
use fluxbar_core::system::SystemMonitor;
use fluxbar_macos::{MacRenderer, UiCommand, UiEvent};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Start the daemon. Blocks the main thread while the native UI is running.
pub fn run(config_path: Option<PathBuf>, socket: PathBuf, headless: bool) -> Result<()> {
    let (config, used_path) = load_config(config_path)?;

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
            let path = Config::default_path();
            match &path {
                Some(path) if path.exists() => {
                    let config = Config::load(path)
                        .with_context(|| format!("failed to load {}", path.display()))?;
                    Ok((config, Some(path.clone())))
                }
                _ => {
                    warn!("no configuration file found, using built-in defaults");
                    Ok((Config::default(), path))
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn async_main(
    config: Config,
    config_path: Option<PathBuf>,
    socket: PathBuf,
    renderer: Arc<MacRenderer>,
    ui_events: std::sync::mpsc::Receiver<UiEvent>,
    shutdown: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
) -> Result<()> {
    let state = StateHandle::new(config.clone());
    let plugins = Arc::new(PluginHost::new(state.clone()));

    // Start timers defined in the config.
    let mut timer_tasks = Vec::new();
    for (name, timer) in &config.events.timers {
        if !timer.enabled {
            continue;
        }
        let state = state.clone();
        let event_name = timer.event.clone();
        let payload = timer.payload.clone().unwrap_or(serde_json::Value::Null);
        let interval = Duration::from_secs(timer.interval.max(1));
        let timer_name = name.clone();
        timer_tasks.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                state.emit(
                    Event::custom(event_name.clone(), payload.clone())
                        .with_source(format!("timer:{timer_name}")),
                );
            }
        }));
    }

    // Built-in system monitoring (CPU, memory, battery, volume, media).
    let monitor = SystemMonitor::new(state.clone(), config.system.clone());
    let monitor_task = monitor.spawn();

    // Start plugins.
    plugins.start_all();

    // IPC server.
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let handler = Arc::new(DaemonHandler {
        state: state.clone(),
        plugins: plugins.clone(),
        renderer: renderer.clone(),
        started: Instant::now(),
        frames: frames.clone(),
        shutdown: shutdown.clone(),
        config_path: config_path.clone(),
    });
    let server = IpcServer::bind(&socket, handler)
        .with_context(|| format!("failed to bind {}", socket.display()))?;
    let server_task = tokio::spawn(async move {
        if let Err(error) = server.run().await {
            tracing::error!(%error, "IPC server stopped");
        }
    });

    // Subscribe to the bus before emitting anything.
    let mut bus = state.bus().subscribe();

    let mut render_tick = tokio::time::interval(Duration::from_millis(16));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut clock_tick = tokio::time::interval(Duration::from_secs(1));
    clock_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut last_version = u64::MAX;
    let mut visible = !config.bar.hidden;
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
                        layout(&state_guard, &config_guard, renderer.screen_size())
                    };
                    // Keep the bar height/visibility in sync with config changes.
                    if visible {
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
                                if let Some(path) = &config_path {
                                    match reload_config(&state, &plugins, path) {
                                        Ok(added) => {
                                            info!(added, "configuration reloaded");
                                        }
                                        Err(error) => {
                                            warn!(%error, "configuration reload failed");
                                        }
                                    }
                                }
                            }
                            "bar_toggle" => {
                                visible = !visible;
                                renderer.set_visible(visible)?;
                            }
                            "bar_show" => {
                                visible = true;
                                renderer.set_visible(true)?;
                            }
                            "bar_hide" => {
                                visible = false;
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
    for task in timer_tasks {
        task.abort();
    }
    monitor_task.abort();
    server_task.abort();
    plugins.stop_all();
    let _ = renderer.send(UiCommand::Shutdown);
    Ok(())
}

/// Reload the configuration, adding new items and updating existing ones.
///
/// Returns the number of newly created items. Items are never deleted by a
/// reload; use `fluxbar remove` for explicit removal.
pub(crate) fn reload_config(
    state: &StateHandle,
    plugins: &PluginHost,
    path: &std::path::Path,
) -> fluxbar_core::error::Result<usize> {
    let config = Config::load(path)?;
    let added = {
        let mut guard = state.write();
        let mut added = 0;
        for (name, item_config) in &config.items {
            if !guard.items.contains_key(name) {
                guard.items.insert(
                    name.clone(),
                    fluxbar_core::Item::new(name.clone(), config.item(name)),
                );
                added += 1;
            } else if let Some(item) = guard.items.get_mut(name) {
                item.config = merge_item_config(&item.config, item_config);
            }
        }
        guard.next_version();
        added
    };
    state.replace_config(config);
    plugins.start_all();
    Ok(added)
}

/// Preserve runtime tweaks when a config reload only sets a subset of fields.
fn merge_item_config(
    base: &fluxbar_core::config::ItemConfig,
    over: &fluxbar_core::config::ItemConfig,
) -> fluxbar_core::config::ItemConfig {
    let mut merged = over.clone();
    if merged.label.is_empty() {
        merged.label = base.label.clone();
    }
    if merged.icon.is_empty() {
        merged.icon = base.icon.clone();
    }
    if merged.background.is_none() {
        merged.background = base.background.clone();
    }
    merged
}

async fn handle_ui_event(state: &StateHandle, event: UiEvent) {
    let (event_name, payload) = match &event {
        UiEvent::Click { item, button } => {
            ("mouse.clicked", json!({ "item": item, "button": button }))
        }
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
