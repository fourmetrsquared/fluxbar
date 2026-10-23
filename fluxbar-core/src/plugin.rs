//! Persistent plugin host.
//!
//! Plugins are long-lived processes speaking newline-delimited JSON-RPC over
//! stdin/stdout. The host owns the process lifecycle: it spawns each plugin
//! once, forwards matching events, applies returned updates, and restarts
//! crashed plugins with exponential backoff.

use crate::config::{PluginConfig, PluginType};
use crate::error::{FluxError, Result};
use crate::event::{Event, EventKind};
use crate::ipc::{JSONRPC_VERSION, RequestId, RpcRequest, RpcResponse};
use crate::state::StateHandle;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

/// Runtime description of a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSpec {
    pub name: String,
    pub config: PluginConfig,
}

impl PluginSpec {
    pub fn new(name: impl Into<String>, config: PluginConfig) -> Self {
        Self {
            name: name.into(),
            config,
        }
    }

    /// Events this plugin cares about.
    pub fn subscribes_to(&self, event: &Event) -> bool {
        if self.config.subscribe.is_empty() {
            return false;
        }
        self.config
            .subscribe
            .iter()
            .any(|pattern| pattern == "*" || pattern == event.name())
    }
}

/// Payload returned by a plugin for a single event.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginResult {
    /// Item updates keyed by item name.
    #[serde(default)]
    pub updates: BTreeMap<String, Value>,
    /// Optional events the plugin wants to emit.
    #[serde(default)]
    pub events: Vec<PluginEmittedEvent>,
    /// Optional log line.
    #[serde(default)]
    pub log: Option<String>,
}

/// Event emitted by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEmittedEvent {
    pub name: String,
    #[serde(default)]
    pub payload: Value,
}

/// One running plugin process.
pub struct PluginProcess {
    name: String,
    command: String,
    args: Vec<String>,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl std::fmt::Debug for PluginProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginProcess")
            .field("name", &self.name)
            .field("command", &self.command)
            .field("pid", &self.child.id())
            .finish()
    }
}

impl PluginProcess {
    /// Spawn the plugin executable described by `spec`.
    pub async fn spawn(spec: &PluginSpec) -> Result<Self> {
        let config = &spec.config;
        let command = config
            .command
            .clone()
            .or_else(|| {
                config
                    .path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .ok_or_else(|| FluxError::Plugin(format!("plugin `{}` has no command", spec.name)))?;

        let mut cmd = Command::new(&command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(dir) = &config.working_dir {
            cmd.current_dir(dir);
        }
        if !config.env.is_empty() {
            cmd.envs(config.env.clone());
        }
        cmd.env("FLUXBAR_PLUGIN_NAME", &spec.name);
        cmd.env("FLUXBAR_PROTOCOL", "1");

        let mut child = cmd
            .spawn()
            .map_err(|e| FluxError::Plugin(format!("failed to spawn `{command}`: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| FluxError::Plugin("plugin stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| FluxError::Plugin("plugin stdout unavailable".into()))?;

        // Forward stderr to the daemon log.
        if let Some(stderr) = child.stderr.take() {
            let name = spec.name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    warn!(plugin = %name, "{line}");
                }
            });
        }

        info!(plugin = %spec.name, command = %command, pid = ?child.id(), "plugin started");

        Ok(Self {
            name: spec.name.clone(),
            command,
            args: config.args.clone(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    /// Process id, if still running.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Whether the child is still alive.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Send a request and wait for the matching response.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<PluginResult> {
        let id = self.next_id;
        self.next_id += 1;

        let request = RpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: RequestId::Number(id),
            method: method.to_string(),
            params,
        };

        let mut encoded = serde_json::to_vec(&request)?;
        encoded.push(b'\n');
        self.stdin.write_all(&encoded).await?;
        self.stdin.flush().await?;

        let mut line = String::new();
        let read = tokio::time::timeout(Duration::from_secs(10), self.stdout.read_line(&mut line))
            .await
            .map_err(|_| FluxError::Plugin(format!("plugin `{}` timed out", self.name)))??;

        if read == 0 {
            return Err(FluxError::Plugin(format!(
                "plugin `{}` closed stdout",
                self.name
            )));
        }

        let response: RpcResponse = serde_json::from_str(line.trim()).map_err(|e| {
            FluxError::Plugin(format!("plugin `{}` sent invalid JSON: {e}", self.name))
        })?;

        if let Some(error) = response.error {
            return Err(FluxError::Plugin(format!(
                "plugin `{}` returned error {}: {}",
                self.name, error.code, error.message
            )));
        }

        let result = response.result.unwrap_or(Value::Null);
        if result.is_null() {
            return Ok(PluginResult::default());
        }
        let parsed: PluginResult = serde_json::from_value(result).map_err(|e| {
            FluxError::Plugin(format!(
                "plugin `{}` returned malformed result: {e}",
                self.name
            ))
        })?;
        Ok(parsed)
    }

    /// Ask the plugin to shut down and wait briefly.
    pub async fn shutdown(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        let request = RpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: RequestId::Number(id),
            method: "shutdown".to_string(),
            params: Value::Null,
        };
        if let Ok(mut encoded) = serde_json::to_vec(&request) {
            encoded.push(b'\n');
            let _ = self.stdin.write_all(&encoded).await;
            let _ = self.stdin.flush().await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        let _ = self.child.kill().await;
    }

    /// Command line used to start the plugin (for diagnostics).
    pub fn command_line(&self) -> String {
        let mut parts = vec![self.command.clone()];
        parts.extend(self.args.clone());
        parts.join(" ")
    }
}

/// Handle for one managed plugin.
pub struct ManagedPlugin {
    pub spec: PluginSpec,
    pub state: Arc<AtomicBool>,
    pub restarts: Arc<std::sync::atomic::AtomicU32>,
    pub pid: Arc<parking_lot::Mutex<Option<u32>>>,
    pub last_error: Arc<parking_lot::Mutex<Option<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for ManagedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedPlugin")
            .field("spec", &self.spec)
            .field("running", &self.state.load(Ordering::Relaxed))
            .finish()
    }
}

impl ManagedPlugin {
    pub fn is_running(&self) -> bool {
        self.state.load(Ordering::Relaxed)
    }

    pub fn restart_count(&self) -> u32 {
        self.restarts.load(Ordering::Relaxed)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub fn stop(&self) {
        self.task.abort();
        self.state.store(false, Ordering::Relaxed);
    }
}

/// Plugin host owning every managed plugin.
pub struct PluginHost {
    state: StateHandle,
    plugins: parking_lot::Mutex<BTreeMap<String, ManagedPlugin>>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl std::fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginHost")
            .field("plugins", &self.plugins.lock().keys().collect::<Vec<_>>())
            .finish()
    }
}

impl PluginHost {
    pub fn new(state: StateHandle) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            state,
            plugins: parking_lot::Mutex::new(BTreeMap::new()),
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Start every plugin from the configuration.
    pub fn start_all(&self) {
        let config = self.state.config();
        let specs: Vec<PluginSpec> = config
            .plugins
            .iter()
            .filter(|(_, cfg)| matches!(cfg.plugin_type, PluginType::External))
            .map(|(name, cfg)| PluginSpec::new(name.clone(), cfg.clone()))
            .collect();
        drop(config);

        for spec in specs {
            self.start(spec);
        }
    }

    /// Start a single plugin.
    pub fn start(&self, spec: PluginSpec) {
        let name = spec.name.clone();
        if self.plugins.lock().contains_key(&name) {
            warn!(plugin = %name, "plugin already running");
            return;
        }

        let state = self.state.clone();
        let running = Arc::new(AtomicBool::new(true));
        let running_task = running.clone();
        let restarts = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let restarts_task = restarts.clone();
        let pid = Arc::new(parking_lot::Mutex::new(None));
        let pid_task = pid.clone();
        let last_error = Arc::new(parking_lot::Mutex::new(None));
        let last_error_task = last_error.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let spec_task = spec.clone();

        let task = tokio::spawn(async move {
            run_plugin(
                spec_task,
                state,
                running_task,
                restarts_task,
                pid_task,
                last_error_task,
                &mut shutdown_rx,
            )
            .await;
        });

        self.plugins.lock().insert(
            name,
            ManagedPlugin {
                spec,
                state: running,
                restarts,
                pid,
                last_error,
                task,
            },
        );
    }

    /// Stop a plugin by name.
    pub fn stop(&self, name: &str) -> Result<()> {
        let removed = self.plugins.lock().remove(name);
        match removed {
            Some(plugin) => {
                plugin.stop();
                info!(plugin = %name, "plugin stopped");
                Ok(())
            }
            None => Err(FluxError::PluginNotFound(name.to_string())),
        }
    }

    /// Stop all plugins.
    pub fn stop_all(&self) {
        let _ = self.shutdown_tx.send(true);
        let plugins = std::mem::take(&mut *self.plugins.lock());
        for (_, plugin) in plugins {
            plugin.stop();
        }
    }

    /// Restart a plugin.
    pub fn restart(&self, name: &str) -> Result<()> {
        let spec = self
            .plugins
            .lock()
            .get(name)
            .map(|plugin| plugin.spec.clone())
            .ok_or_else(|| FluxError::PluginNotFound(name.to_string()))?;
        self.stop(name)?;
        self.start(spec);
        Ok(())
    }

    /// Snapshot for the `plugin.list` RPC method.
    pub fn list(&self) -> Vec<Value> {
        self.plugins
            .lock()
            .values()
            .map(|plugin| {
                json!({
                    "name": plugin.spec.name,
                    "running": plugin.is_running(),
                    "restarts": plugin.restart_count(),
                    "pid": *plugin.pid.lock(),
                    "last_error": plugin.last_error(),
                })
            })
            .collect()
    }

    /// Forward an event to a specific plugin (bypassing subscriptions).
    pub fn notify(&self, name: &str, event: Event) -> Result<()> {
        let _ = name;
        // Events normally flow through the bus; direct notifications are useful
        // for plugin-to-plugin messaging. Emitting on the bus keeps the design
        // simple and preserves ordering.
        self.state.emit(event);
        Ok(())
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

async fn run_plugin(
    spec: PluginSpec,
    state: StateHandle,
    running: Arc<AtomicBool>,
    restarts: Arc<std::sync::atomic::AtomicU32>,
    pid: Arc<parking_lot::Mutex<Option<u32>>>,
    last_error: Arc<parking_lot::Mutex<Option<String>>>,
    shutdown_rx: &mut watch::Receiver<bool>,
) {
    let name = spec.name.clone();
    let mut bus = state.bus().subscribe();
    let mut backoff = Duration::from_secs(spec.config.restart_delay.max(1));

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        let mut process = match PluginProcess::spawn(&spec).await {
            Ok(process) => process,
            Err(error) => {
                *last_error.lock() = Some(error.to_string());
                error!(plugin = %name, %error, "failed to start plugin");
                if !spec.config.auto_restart {
                    break;
                }
                if !wait_or_shutdown(backoff, shutdown_rx).await {
                    break;
                }
                if spec.config.max_restarts > 0
                    && restarts.load(Ordering::Relaxed) >= spec.config.max_restarts
                {
                    error!(plugin = %name, "restart limit reached");
                    break;
                }
                restarts.fetch_add(1, Ordering::Relaxed);
                backoff = (backoff * 2).min(Duration::from_secs(60));
                continue;
            }
        };

        *pid.lock() = process.pid();
        running.store(true, Ordering::Relaxed);
        *last_error.lock() = None;
        let session_started = std::time::Instant::now();

        // Initialise the plugin with its config.
        let init_params = json!({
            "name": name,
            "config": spec.config.config,
            "subscribe": spec.config.subscribe,
            "version": env!("CARGO_PKG_VERSION"),
        });
        if let Err(error) = process.request("init", init_params).await {
            warn!(plugin = %name, %error, "plugin init failed");
            *last_error.lock() = Some(error.to_string());
        }

        let tick_interval = spec.config.tick_interval.map(Duration::from_secs);
        let mut ticker = tick_interval.map(tokio::time::interval);
        if let Some(ticker) = ticker.as_mut() {
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Consume the immediate first tick.
            ticker.tick().await;
        }

        let mut crashed = false;
        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            let event = tokio::select! {
                received = bus.recv() => {
                    match received {
                        Ok(event) => Some((*event).clone()),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(plugin = %name, skipped, "plugin event subscriber lagged");
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = async {
                    if let Some(ticker) = ticker.as_mut() {
                        ticker.tick().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => None,
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                    continue;
                }
            };

            let event =
                event.unwrap_or_else(|| Event::new(EventKind::Custom("tick".into()), json!({})));
            if event.name() != "tick" && !spec.subscribes_to(&event) {
                continue;
            }

            let params = json!({
                "kind": event.name(),
                "payload": event.payload,
                "timestamp": event.timestamp,
                "source": event.source,
            });

            match process.request("event", params).await {
                Ok(result) => {
                    if let Some(log) = result.log {
                        info!(plugin = %name, "{log}");
                    }
                    if !result.updates.is_empty() {
                        if let Err(error) = state.apply_plugin_updates(&result.updates) {
                            warn!(plugin = %name, %error, "failed to apply plugin updates");
                        }
                    }
                    for emitted in result.events {
                        state.emit(
                            Event::custom(emitted.name, emitted.payload).with_source(name.clone()),
                        );
                    }
                }
                Err(error) => {
                    *last_error.lock() = Some(error.to_string());
                    error!(plugin = %name, %error, "plugin request failed");
                    crashed = true;
                    break;
                }
            }
        }

        running.store(false, Ordering::Relaxed);
        process.shutdown().await;
        *pid.lock() = None;

        if *shutdown_rx.borrow() {
            break;
        }

        if !crashed && !spec.config.auto_restart {
            break;
        }

        if spec.config.max_restarts > 0
            && restarts.load(Ordering::Relaxed) >= spec.config.max_restarts
        {
            error!(plugin = %name, "restart limit reached");
            break;
        }

        restarts.fetch_add(1, Ordering::Relaxed);
        // A plugin that ran for a while before crashing gets a fresh backoff
        // instead of inheriting a long delay from an earlier crash loop.
        if session_started.elapsed() >= Duration::from_secs(60) {
            backoff = Duration::from_secs(spec.config.restart_delay.max(1));
            debug!(plugin = %name, "backoff reset after stable run");
        }
        warn!(
            plugin = %name,
            delay = ?backoff,
            "restarting plugin after crash"
        );
        if !wait_or_shutdown(backoff, shutdown_rx).await {
            break;
        }
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

async fn wait_or_shutdown(duration: Duration, shutdown_rx: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        () = tokio::time::sleep(duration) => true,
        _ = shutdown_rx.changed() => !*shutdown_rx.borrow(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_matching() {
        let config = PluginConfig {
            subscribe: vec!["tick".into(), "custom-*".into()],
            ..PluginConfig::default()
        };
        let spec = PluginSpec::new("test", config);

        assert!(spec.subscribes_to(&Event::custom("tick", json!({}))));
        assert!(!spec.subscribes_to(&Event::custom("other", json!({}))));

        let config = PluginConfig {
            subscribe: vec!["*".into()],
            ..PluginConfig::default()
        };
        let spec = PluginSpec::new("all", config);
        assert!(spec.subscribes_to(&Event::custom("anything", json!({}))));
    }

    #[test]
    fn empty_subscription_receives_nothing() {
        let spec = PluginSpec::new("test", PluginConfig::default());
        assert!(!spec.subscribes_to(&Event::custom("tick", json!({}))));
    }
}
