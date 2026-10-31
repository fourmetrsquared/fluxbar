//! Persistent plugin host.
//!
//! Plugins are long-lived processes speaking newline-delimited JSON-RPC over
//! stdin/stdout. The host owns the process lifecycle: it spawns each plugin
//! once, forwards matching events, applies returned updates, and restarts
//! crashed plugins with exponential backoff.

use crate::config::{PluginConfig, PluginRuntime, PluginType};
use crate::error::{FluxError, Result};
use crate::event::{Event, EventKind};
use crate::ipc::{
    JSONRPC_VERSION, MAX_MESSAGE_BYTES, RequestId, RpcRequest, RpcResponse, read_line_limited,
};
use crate::script::{
    ScriptCommand, ScriptContext, ScriptInvocation, run_script, sketchybar_sender,
};
use crate::state::StateHandle;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, trace, warn};

/// Stable run time after which the consecutive-crash counter is reset.
const STABLE_RUN: Duration = Duration::from_secs(60);
/// Maximum time to wait for a plugin to acknowledge shutdown.
const PLUGIN_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Runtime description of a plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        crate::script::subscription_matches(&self.config.subscribe, event.name())
    }

    /// Effective runtime, including extension-based inference.
    pub fn runtime(&self) -> PluginRuntime {
        self.config.effective_runtime()
    }

    /// Resolve the script file invoked by a script runtime.
    pub fn script_command(&self) -> Result<ScriptCommand> {
        let runtime = self.runtime();
        if !runtime.is_script() {
            return Err(FluxError::Plugin(format!(
                "plugin `{}` is not a script plugin",
                self.name
            )));
        }

        let command_path = self.config.command.as_deref().map(std::path::Path::new);
        let command_name = command_path
            .and_then(std::path::Path::file_name)
            .and_then(|name| name.to_str());
        let interpreter = matches!(command_name, Some("lua" | "luajit" | "sh" | "bash"));

        let mut args = self.config.args.clone();
        let path = if interpreter {
            let index = args
                .iter()
                .position(|arg| !arg.starts_with('-'))
                .ok_or_else(|| {
                    FluxError::Plugin(format!(
                        "script plugin `{}` has no script argument",
                        self.name
                    ))
                })?;
            let path = args.remove(index);
            std::path::PathBuf::from(path)
        } else if let Some(path) = &self.config.path {
            path.clone()
        } else if let Some(command) = &self.config.command {
            std::path::PathBuf::from(command)
        } else {
            return Err(FluxError::Plugin(format!(
                "script plugin `{}` has no command or path",
                self.name
            )));
        };

        Ok(ScriptCommand::File { path, args })
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
    pub async fn spawn(spec: &PluginSpec, context: &ScriptContext) -> Result<Self> {
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

        let command = crate::script::expand_home(std::path::Path::new(&command))
            .to_string_lossy()
            .to_string();
        let mut cmd = Command::new(&command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(dir) = &config.working_dir {
            cmd.current_dir(crate::script::expand_home(dir));
        }
        cmd.envs(crate::script::ambient_environment(context));
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

        // Forward stderr to the daemon log, preserving conventional levels.
        if let Some(stderr) = child.stderr.take() {
            let name = spec.name.clone();
            tokio::spawn(async move {
                let mut stderr = BufReader::new(stderr);
                while let Ok(Some(line)) = read_line_limited(&mut stderr, MAX_MESSAGE_BYTES).await {
                    let (level, message) = classify_stderr(&line);
                    log_plugin_stderr(&name, level, &message);
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
        tokio::time::timeout(Duration::from_secs(10), async {
            self.stdin.write_all(&encoded).await?;
            self.stdin.flush().await
        })
        .await
        .map_err(|_| FluxError::Plugin(format!("plugin `{}` stopped reading", self.name)))??;

        let line = tokio::time::timeout(
            Duration::from_secs(10),
            read_line_limited(&mut self.stdout, MAX_MESSAGE_BYTES),
        )
        .await
        .map_err(|_| FluxError::Plugin(format!("plugin `{}` timed out", self.name)))??;

        let line =
            line.ok_or_else(|| FluxError::Plugin(format!("plugin `{}` closed stdout", self.name)))?;

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
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                let _ = self.stdin.write_all(&encoded).await;
                let _ = self.stdin.flush().await;
            })
            .await;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StderrLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
    Unknown,
}

fn level_from_name(name: &str) -> Option<StderrLevel> {
    match name
        .trim_matches(|character: char| character == '[' || character == ']' || character == ':')
        .to_ascii_lowercase()
        .as_str()
    {
        "error" | "err" | "fatal" | "panic" => Some(StderrLevel::Error),
        "warn" | "warning" => Some(StderrLevel::Warn),
        "info" | "notice" => Some(StderrLevel::Info),
        "debug" => Some(StderrLevel::Debug),
        "trace" => Some(StderrLevel::Trace),
        _ => None,
    }
}

/// Parse a plugin stderr line into a level and message.
///
/// Plugins commonly emit `error: ...`, `[warn] ...`, or JSON lines such as
/// `{"level":"debug","message":"..."}`. Unstructured output remains a warning.
fn classify_stderr(line: &str) -> (StderrLevel, String) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return (StderrLevel::Unknown, String::new());
    }

    if trimmed.starts_with('{')
        && let Ok(value) = serde_json::from_str::<Value>(trimmed)
        && let Some(level) = value
            .get("level")
            .and_then(Value::as_str)
            .and_then(level_from_name)
    {
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or(trimmed)
            .to_string();
        return (level, message);
    }

    let split = trimmed
        .find(|character: char| character.is_whitespace())
        .unwrap_or(trimmed.len());
    let token = &trimmed[..split];
    let message = trimmed[split..].trim_start();
    if let Some(level) = level_from_name(token) {
        return (level, message.to_string());
    }
    (StderrLevel::Unknown, trimmed.to_string())
}

fn log_plugin_stderr(name: &str, level: StderrLevel, message: &str) {
    match level {
        StderrLevel::Error => error!(plugin = %name, stream = "stderr", "{message}"),
        StderrLevel::Warn => warn!(plugin = %name, stream = "stderr", "{message}"),
        StderrLevel::Info => info!(plugin = %name, stream = "stderr", "{message}"),
        StderrLevel::Debug => debug!(plugin = %name, stream = "stderr", "{message}"),
        StderrLevel::Trace => trace!(plugin = %name, stream = "stderr", "{message}"),
        StderrLevel::Unknown => warn!(plugin = %name, stream = "stderr", "{message}"),
    }
}

/// Explicit lifecycle state exposed by `plugin.list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginPhase {
    Starting,
    Running,
    Backoff,
    Stopped,
    Failed,
}

/// Handle for one managed plugin.
pub struct ManagedPlugin {
    pub spec: PluginSpec,
    pub state: Arc<AtomicBool>,
    pub restarts: Arc<std::sync::atomic::AtomicU32>,
    pub pid: Arc<parking_lot::Mutex<Option<u32>>>,
    pub last_error: Arc<parking_lot::Mutex<Option<String>>>,
    phase: Arc<parking_lot::Mutex<PluginPhase>>,
    direct_tx: mpsc::UnboundedSender<Event>,
    stop_tx: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct PluginLifecycle {
    running: Arc<AtomicBool>,
    restarts: Arc<std::sync::atomic::AtomicU32>,
    pid: Arc<parking_lot::Mutex<Option<u32>>>,
    last_error: Arc<parking_lot::Mutex<Option<String>>>,
    phase: Arc<parking_lot::Mutex<PluginPhase>>,
}

impl Default for PluginLifecycle {
    fn default() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            restarts: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            pid: Arc::new(parking_lot::Mutex::new(None)),
            last_error: Arc::new(parking_lot::Mutex::new(None)),
            phase: Arc::new(parking_lot::Mutex::new(PluginPhase::Starting)),
        }
    }
}

impl std::fmt::Debug for ManagedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedPlugin")
            .field("spec", &self.spec)
            .field("running", &self.is_running())
            .field("phase", &self.phase())
            .finish()
    }
}

impl ManagedPlugin {
    pub fn is_running(&self) -> bool {
        self.state.load(Ordering::Relaxed) && *self.phase.lock() == PluginPhase::Running
    }

    pub fn restart_count(&self) -> u32 {
        self.restarts.load(Ordering::Relaxed)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub fn phase(&self) -> PluginPhase {
        *self.phase.lock()
    }

    /// Ask the task to shut down. The task sends a JSON-RPC `shutdown` request
    /// and waits for the child process before exiting.
    fn request_stop(&self) {
        let _ = self.stop_tx.send(true);
        self.state.store(false, Ordering::Relaxed);
        *self.phase.lock() = PluginPhase::Stopped;
    }

    async fn shutdown(mut self) {
        self.request_stop();
        if tokio::time::timeout(PLUGIN_SHUTDOWN_TIMEOUT, &mut self.task)
            .await
            .is_err()
        {
            warn!(plugin = %self.spec.name, "plugin shutdown timed out; aborting");
            self.task.abort();
        }
    }
}

/// Plugin host owning every managed plugin.
pub struct PluginHost {
    state: StateHandle,
    plugins: parking_lot::Mutex<BTreeMap<String, ManagedPlugin>>,
    script_context: parking_lot::RwLock<ScriptContext>,
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
        Self {
            state,
            plugins: parking_lot::Mutex::new(BTreeMap::new()),
            script_context: parking_lot::RwLock::new(ScriptContext::default()),
        }
    }

    /// Configure paths and shims exposed to script plugins.
    pub fn set_script_context(&self, context: ScriptContext) {
        *self.script_context.write() = context;
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

    /// Reconcile running plugins with a new configuration.
    ///
    /// Removed plugins are stopped, changed specs are restarted, and new
    /// specs are started. Plugins whose spec is unchanged keep their process.
    /// Returns `(started, stopped)` counts for diagnostics.
    pub fn reconcile(&self, specs: BTreeMap<String, PluginSpec>) -> (usize, usize) {
        let mut stopped = 0;
        {
            let mut plugins = self.plugins.lock();
            let names: Vec<String> = plugins.keys().cloned().collect();
            for name in names {
                let should_stop = match (plugins.get(&name), specs.get(&name)) {
                    (Some(plugin), Some(spec)) => plugin.spec != *spec,
                    (Some(_), None) => true,
                    _ => false,
                };
                if should_stop && let Some(plugin) = plugins.remove(&name) {
                    plugin.request_stop();
                    stopped += 1;
                }
            }
        }

        let mut started = 0;
        for (name, spec) in specs {
            let missing = !self.plugins.lock().contains_key(&name);
            if missing {
                self.start(spec);
                started += 1;
            }
        }
        (started, stopped)
    }

    /// Start a single plugin.
    #[tracing::instrument(name = "plugin.start", skip(self, spec), fields(plugin = %spec.name))]
    pub fn start(&self, spec: PluginSpec) {
        let name = spec.name.clone();
        let mut plugins = self.plugins.lock();
        if plugins.contains_key(&name) {
            warn!(plugin = %name, "plugin already running");
            return;
        }

        let state = self.state.clone();
        let lifecycle = PluginLifecycle::default();
        let lifecycle_task = lifecycle.clone();
        let (direct_tx, mut direct_rx) = mpsc::unbounded_channel();
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let spec_task = spec.clone();
        let script_context = self.script_context.read().clone();

        let task = if spec.runtime().is_script() {
            tokio::spawn(async move {
                run_script_plugin(
                    spec_task,
                    state,
                    script_context,
                    lifecycle_task,
                    &mut stop_rx,
                    &mut direct_rx,
                )
                .await;
            })
        } else {
            tokio::spawn(async move {
                run_plugin(
                    spec_task,
                    state,
                    script_context,
                    lifecycle_task,
                    &mut stop_rx,
                    &mut direct_rx,
                )
                .await;
            })
        };

        plugins.insert(
            name,
            ManagedPlugin {
                spec,
                state: lifecycle.running,
                restarts: lifecycle.restarts,
                pid: lifecycle.pid,
                last_error: lifecycle.last_error,
                phase: lifecycle.phase,
                direct_tx,
                stop_tx,
                task,
            },
        );
    }

    /// Stop a plugin by name and wait for its graceful shutdown handshake.
    #[tracing::instrument(name = "plugin.stop", skip(self), fields(plugin = %name))]
    pub async fn stop(&self, name: &str) -> Result<()> {
        let removed = self.plugins.lock().remove(name);
        match removed {
            Some(plugin) => {
                plugin.shutdown().await;
                info!(plugin = %name, "plugin stopped");
                Ok(())
            }
            None => Err(FluxError::PluginNotFound(name.to_string())),
        }
    }

    /// Signal every plugin to stop without waiting. Useful for `Drop` and for
    /// synchronous reconcile paths, where a new spec may replace the old task.
    pub fn stop_all(&self) {
        let plugins = std::mem::take(&mut *self.plugins.lock());
        for (_, plugin) in plugins {
            plugin.request_stop();
        }
    }

    /// Stop every plugin and wait for each graceful shutdown.
    pub async fn shutdown_all(&self) {
        let plugins = std::mem::take(&mut *self.plugins.lock());
        for (_, plugin) in plugins {
            plugin.shutdown().await;
        }
    }

    /// Restart a plugin using its current spec.
    pub async fn restart(&self, name: &str) -> Result<()> {
        let spec = self
            .plugins
            .lock()
            .get(name)
            .map(|plugin| plugin.spec.clone())
            .ok_or_else(|| FluxError::PluginNotFound(name.to_string()))?;
        self.stop(name).await?;
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
                    "runtime": plugin.spec.runtime(),
                    "state": plugin.phase(),
                    "running": plugin.is_running(),
                    "restarts": plugin.restart_count(),
                    "pid": *plugin.pid.lock(),
                    "last_error": plugin.last_error(),
                })
            })
            .collect()
    }

    /// Forward an event only to one plugin, bypassing its subscription list.
    pub fn notify(&self, name: &str, event: Event) -> Result<()> {
        let sender = self
            .plugins
            .lock()
            .get(name)
            .map(|plugin| plugin.direct_tx.clone())
            .ok_or_else(|| FluxError::PluginNotFound(name.to_string()))?;
        sender.send(event).map_err(|_| {
            FluxError::Plugin(format!("plugin `{name}` is not accepting direct events"))
        })
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        let plugins = std::mem::take(&mut *self.plugins.lock());
        for (_, plugin) in plugins {
            plugin.request_stop();
        }
    }
}

async fn run_script_plugin(
    spec: PluginSpec,
    state: StateHandle,
    context: ScriptContext,
    lifecycle: PluginLifecycle,
    shutdown_rx: &mut watch::Receiver<bool>,
    direct_rx: &mut mpsc::UnboundedReceiver<Event>,
) {
    let PluginLifecycle {
        running,
        last_error,
        phase,
        ..
    } = lifecycle;
    let name = spec.name.clone();
    let mut bus = state.bus().subscribe();
    running.store(true, Ordering::Relaxed);
    *phase.lock() = PluginPhase::Running;

    let started = Event::custom("plugin_started", json!({"name": name.clone()}));
    invoke_script_plugin(&spec, &state, &context, &started)
        .await
        .map_or_else(
            |error| {
                *last_error.lock() = Some(error.to_string());
                warn!(plugin = %name, %error, "script plugin initial run failed");
            },
            |_| *last_error.lock() = None,
        );

    let tick_interval = spec.config.tick_interval.map(Duration::from_secs);
    let mut ticker = tick_interval.map(tokio::time::interval);
    if let Some(ticker) = ticker.as_mut() {
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
    }

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        let (event, direct) = tokio::select! {
            received = bus.recv() => {
                match received {
                    Ok(event) => ((*event).clone(), false),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        state.bus().record_lag(skipped);
                        warn!(plugin = %name, skipped, "script plugin event subscriber lagged");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            direct = direct_rx.recv() => {
                match direct {
                    Some(event) => (event, true),
                    None => break,
                }
            }
            _ = async {
                if let Some(ticker) = ticker.as_mut() {
                    ticker.tick().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => (Event::new(EventKind::Custom("tick".into()), json!({})), false),
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
                continue;
            }
        };

        if !direct && event.name() != "tick" && !spec.subscribes_to(&event) {
            continue;
        }

        match invoke_script_plugin(&spec, &state, &context, &event).await {
            Ok(()) => *last_error.lock() = None,
            Err(error) => {
                *last_error.lock() = Some(error.to_string());
                warn!(
                    plugin = %name,
                    event = event.name(),
                    %error,
                    "script plugin event failed"
                );
            }
        }
    }

    running.store(false, Ordering::Relaxed);
    *phase.lock() = PluginPhase::Stopped;
    info!(plugin = %name, "script plugin stopped");
}

async fn invoke_script_plugin(
    spec: &PluginSpec,
    state: &StateHandle,
    context: &ScriptContext,
    event: &Event,
) -> Result<()> {
    let mut invocation =
        ScriptInvocation::new(spec.name.clone(), event.name(), spec.script_command()?)
            .with_payload(event.payload.clone())
            .with_runtime(spec.runtime());
    invocation.environment = spec.config.env.clone();
    invocation.environment.extend(event.environment.clone());
    invocation
        .environment
        .insert("FLUXBAR_PLUGIN".into(), spec.name.clone());
    if let Some(directory) = &spec.config.working_dir {
        invocation.working_dir = Some(directory.clone());
    }

    let output = run_script(context, &invocation).await?;
    if let Some(log) = &output.result.log {
        info!(plugin = %spec.name, "{log}");
    }
    if !output.result.updates.is_empty() {
        state.apply_plugin_updates(&output.result.updates)?;
    }
    for emitted in &output.result.events {
        state.emit(
            Event::custom(emitted.name.clone(), emitted.payload.clone())
                .with_source(spec.name.clone()),
        );
    }
    debug!(
        plugin = %spec.name,
        event = event.name(),
        sender = %sketchybar_sender(event.name()),
        "script plugin executed"
    );
    Ok(())
}

async fn run_plugin(
    spec: PluginSpec,
    state: StateHandle,
    context: ScriptContext,
    lifecycle: PluginLifecycle,
    shutdown_rx: &mut watch::Receiver<bool>,
    direct_rx: &mut mpsc::UnboundedReceiver<Event>,
) {
    let PluginLifecycle {
        running,
        restarts,
        pid,
        last_error,
        phase,
    } = lifecycle;
    let name = spec.name.clone();
    let mut bus = state.bus().subscribe();
    let base_backoff = Duration::from_secs(spec.config.restart_delay.max(1));
    let mut backoff = base_backoff;

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        *phase.lock() = PluginPhase::Starting;
        let mut process = match PluginProcess::spawn(&spec, &context).await {
            Ok(process) => process,
            Err(error) => {
                *last_error.lock() = Some(error.to_string());
                error!(plugin = %name, %error, "failed to start plugin");
                if !spec.config.auto_restart
                    || (spec.config.max_restarts > 0
                        && restarts.load(Ordering::Relaxed) >= spec.config.max_restarts)
                {
                    *phase.lock() = PluginPhase::Failed;
                    break;
                }
                *phase.lock() = PluginPhase::Backoff;
                if !wait_or_shutdown(backoff, shutdown_rx).await {
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
        *phase.lock() = PluginPhase::Running;
        let session_started = std::time::Instant::now();

        // A failed init is a startup failure, not a usable running plugin.
        let init_params = json!({
            "name": name,
            "config": spec.config.config,
            "subscribe": spec.config.subscribe,
            "version": env!("CARGO_PKG_VERSION"),
        });
        let init_failed = match process.request("init", init_params).await {
            Ok(_) => false,
            Err(error) => {
                *last_error.lock() = Some(error.to_string());
                warn!(plugin = %name, %error, "plugin init failed");
                true
            }
        };

        if !init_failed {
            let tick_interval = spec.config.tick_interval.map(Duration::from_secs);
            let mut ticker = tick_interval.map(tokio::time::interval);
            if let Some(ticker) = ticker.as_mut() {
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                ticker.tick().await;
            }

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let (event, direct) = tokio::select! {
                    received = bus.recv() => {
                        match received {
                            Ok(event) => ((*event).clone(), false),
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                state.bus().record_lag(skipped);
                                warn!(plugin = %name, skipped, "plugin event subscriber lagged");
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    direct = direct_rx.recv() => {
                        match direct {
                            Some(event) => (event, true),
                            None => break,
                        }
                    }
                    _ = async {
                        if let Some(ticker) = ticker.as_mut() {
                            ticker.tick().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => (Event::new(EventKind::Custom("tick".into()), json!({})), false),
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                        continue;
                    }
                };

                if !direct && event.name() != "tick" && !spec.subscribes_to(&event) {
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
                        if !result.updates.is_empty()
                            && let Err(error) = state.apply_plugin_updates(&result.updates)
                        {
                            warn!(plugin = %name, %error, "failed to apply plugin updates");
                        }
                        for emitted in result.events {
                            state.emit(
                                Event::custom(emitted.name, emitted.payload)
                                    .with_source(name.clone()),
                            );
                        }
                    }
                    Err(error) => {
                        *last_error.lock() = Some(error.to_string());
                        error!(plugin = %name, %error, "plugin request failed");
                        break;
                    }
                }
            }
        }

        running.store(false, Ordering::Relaxed);
        process.shutdown().await;
        *pid.lock() = None;

        if *shutdown_rx.borrow() {
            break;
        }

        // A session that survived the stable window is no longer part of a
        // crash loop. Reset both the consecutive counter and the backoff.
        if session_started.elapsed() >= STABLE_RUN {
            restarts.store(0, Ordering::Relaxed);
            backoff = base_backoff;
            debug!(plugin = %name, "restart counter reset after stable run");
        }

        if !spec.config.auto_restart
            || (spec.config.max_restarts > 0
                && restarts.load(Ordering::Relaxed) >= spec.config.max_restarts)
        {
            if !spec.config.auto_restart {
                debug!(plugin = %name, "auto_restart is disabled");
            } else {
                error!(plugin = %name, "restart limit reached");
            }
            *phase.lock() = PluginPhase::Failed;
            break;
        }

        restarts.fetch_add(1, Ordering::Relaxed);
        *phase.lock() = PluginPhase::Backoff;
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

    running.store(false, Ordering::Relaxed);
    *pid.lock() = None;
    if *phase.lock() != PluginPhase::Failed {
        *phase.lock() = PluginPhase::Stopped;
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
    fn classifies_plugin_stderr_levels() {
        assert_eq!(
            classify_stderr("error: boom"),
            (StderrLevel::Error, "boom".into())
        );
        assert_eq!(
            classify_stderr("[warning] slow down"),
            (StderrLevel::Warn, "slow down".into())
        );
        assert_eq!(
            classify_stderr("INFO ready"),
            (StderrLevel::Info, "ready".into())
        );
        assert_eq!(
            classify_stderr(r#"{"level":"debug","message":"state"}"#),
            (StderrLevel::Debug, "state".into())
        );
        assert_eq!(
            classify_stderr("plain output"),
            (StderrLevel::Unknown, "plain output".into())
        );
    }

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
        assert!(!spec.subscribes_to(&Event::custom("item_updated", json!({}))));
        assert!(!spec.subscribes_to(&Event::custom("item_added", json!({}))));
        assert!(spec.subscribes_to(&Event::custom("mouse.clicked", json!({}))));
    }

    #[test]
    fn empty_subscription_receives_nothing() {
        let spec = PluginSpec::new("test", PluginConfig::default());
        assert!(!spec.subscribes_to(&Event::custom("tick", json!({}))));
    }

    #[tokio::test]
    async fn reconcile_adds_removes_and_restarts_plugins() {
        let state = StateHandle::new(crate::config::Config::default());
        let host = PluginHost::new(state);
        let spec = PluginSpec::new(
            "reconcile",
            PluginConfig {
                command: Some("/bin/true".into()),
                auto_restart: false,
                ..PluginConfig::default()
            },
        );
        let specs = BTreeMap::from([("reconcile".into(), spec.clone())]);

        assert_eq!(host.reconcile(specs.clone()), (1, 0));
        assert_eq!(host.list().len(), 1);
        assert_eq!(host.reconcile(specs.clone()), (0, 0));

        let mut changed = specs.clone();
        changed.get_mut("reconcile").unwrap().config.args = vec!["--changed".into()];
        assert_eq!(host.reconcile(changed), (1, 1));

        assert_eq!(host.reconcile(BTreeMap::new()), (0, 1));
        assert!(host.list().is_empty());
        assert_eq!(host.reconcile(specs), (1, 0));
        host.shutdown_all().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_notifications_are_targeted_and_host_can_restart() {
        use std::os::unix::fs::PermissionsExt;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let script = dir.path().join("plugin.bin");
        std::fs::write(
            &script,
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"init"'*) printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}" ;;
    *'"method":"event"'*) printf '%s\n' "$line" >> "$EVENT_LOG"; printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}" ;;
    *'"method":"shutdown"'*) exit 0 ;;
  esac
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let spec = |name: &str, log: &std::path::Path| {
            let mut config = PluginConfig {
                command: Some(script.to_string_lossy().into_owned()),
                ..PluginConfig::default()
            };
            config
                .env
                .insert("EVENT_LOG".into(), log.to_string_lossy().into_owned());
            PluginSpec::new(name, config)
        };
        let log_one = dir.path().join("one.log");
        let log_two = dir.path().join("two.log");
        let state = StateHandle::new(crate::config::Config::default());
        let host = PluginHost::new(state);
        host.start(spec("one", &log_one));
        host.start(spec("two", &log_two));

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let list = host.list();
                if list.len() == 2
                    && list
                        .iter()
                        .all(|plugin| plugin["state"].as_str() == Some("running"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        host.notify(
            "one",
            Event::custom("direct-event", json!({"target": "one"})),
        )
        .unwrap();
        assert!(
            host.notify("missing", Event::custom("direct-event", json!({})))
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let one = std::fs::read_to_string(&log_one).unwrap_or_default();
                if one.contains("direct-event") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            !std::fs::read_to_string(&log_two)
                .unwrap_or_default()
                .contains("direct-event")
        );

        host.stop_all();
        assert!(host.list().is_empty());
        let log_three = dir.path().join("three.log");
        host.start(spec("three", &log_three));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if host
                    .list()
                    .iter()
                    .any(|plugin| plugin["state"].as_str() == Some("running"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        host.shutdown_all().await;
    }
}
