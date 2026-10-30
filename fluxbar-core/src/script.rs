//! Event-driven script execution.
//!
//! FluxBar plugin scripts and SketchyBar-style item scripts are not long-lived
//! JSON-RPC processes. Instead they are executed once per matching event with
//! a stable environment and may either print a JSON update object or call the
//! `sketchybar`/`fluxbar` command-line interface.

use crate::config::PluginRuntime;
use crate::error::{FluxError, Result};
use crate::event::Event;
use crate::plugin::{PluginEmittedEvent, PluginResult};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Default hard limit for one script invocation (matches SketchyBar).
pub const SCRIPT_TIMEOUT: Duration = Duration::from_secs(60);

/// Maximum size we will read from each script stream. A script that prints
/// more is treated as misbehaving instead of exhausting daemon memory.
pub const MAX_SCRIPT_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Ambient information needed by script plugins.
#[derive(Debug, Clone, Default)]
pub struct ScriptContext {
    /// Directory containing `fluxbar.toml` / `plugins.d`.
    pub config_dir: Option<PathBuf>,
    /// Daemon socket; exposed as `FLUXBAR_SOCKET` when set.
    pub socket: Option<PathBuf>,
    /// Directory prepended to `PATH` that contains `sketchybar`/`fluxbar`
    /// compatibility shims.
    pub compat_dir: Option<PathBuf>,
}

impl ScriptContext {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: Some(config_dir.into()),
            ..Self::default()
        }
    }
}

/// How a script is invoked.
#[derive(Debug, Clone)]
pub enum ScriptCommand {
    /// A script file plus extra arguments.
    File { path: PathBuf, args: Vec<String> },
    /// A shell command line (the value of an item's `script` property).
    Shell(String),
}

/// A fully described script invocation.
#[derive(Debug, Clone)]
pub struct ScriptInvocation {
    /// Item or plugin name exposed as `NAME`.
    pub name: String,
    /// Raw event name (`cpu_change`, `mouse.clicked`, ...).
    pub event: String,
    /// Optional payload exposed as `INFO` and `FLUXBAR_PAYLOAD`.
    pub payload: Value,
    /// Additional environment variables.
    pub environment: BTreeMap<String, String>,
    /// Script command.
    pub command: ScriptCommand,
    /// Language/runtime. `None` auto-detects from the extension.
    pub runtime: Option<PluginRuntime>,
    /// Working directory; defaults to the script's parent or config dir.
    pub working_dir: Option<PathBuf>,
    /// Per-invocation timeout.
    pub timeout: Duration,
}

impl ScriptInvocation {
    pub fn new(name: impl Into<String>, event: impl Into<String>, command: ScriptCommand) -> Self {
        Self {
            name: name.into(),
            event: event.into(),
            payload: Value::Null,
            environment: BTreeMap::new(),
            command,
            runtime: None,
            working_dir: None,
            timeout: SCRIPT_TIMEOUT,
        }
    }

    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }

    pub fn with_environment(mut self, environment: BTreeMap<String, String>) -> Self {
        self.environment = environment;
        self
    }

    pub fn with_runtime(mut self, runtime: PluginRuntime) -> Self {
        self.runtime = Some(runtime);
        self
    }

    pub fn with_working_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(directory.into());
        self
    }
}

/// Result of one script invocation.
#[derive(Debug, Clone, Default)]
pub struct ScriptOutput {
    /// Updates parsed from stdout, if any.
    pub result: PluginResult,
    /// Raw stdout (trimmed).
    pub stdout: String,
    /// Raw stderr (trimmed).
    pub stderr: String,
}

pub(crate) async fn read_capped<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut limited = reader.take((limit as u64).saturating_add(1));
    limited.read_to_end(&mut output).await?;
    if output.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("script output exceeds {limit} bytes"),
        ));
    }
    Ok(output)
}

/// Execute a script invocation with a timeout.
pub async fn run_script(
    context: &ScriptContext,
    invocation: &ScriptInvocation,
) -> Result<ScriptOutput> {
    let mut command = build_command(context, invocation)?;
    let script_env = environment(context, invocation);
    command.envs(&script_env);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);

    if let Some(directory) = invocation
        .working_dir
        .clone()
        .or_else(|| context.config_dir.clone())
    {
        command.current_dir(expand_home(&directory));
    }

    let mut child = command.spawn().map_err(|error| {
        FluxError::Plugin(format!(
            "failed to run script `{}`: {error}",
            invocation.name
        ))
    })?;
    let mut child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| FluxError::Plugin("script stdout unavailable".into()))?;
    let mut child_stderr = child
        .stderr
        .take()
        .ok_or_else(|| FluxError::Plugin("script stderr unavailable".into()))?;

    let output = tokio::time::timeout(invocation.timeout, async {
        // `try_join!` cancels the other stream as soon as one exceeds its cap;
        // otherwise an endless writer on stderr would keep us waiting after
        // stdout has already failed.
        let result = tokio::try_join!(
            read_capped(&mut child_stdout, MAX_SCRIPT_OUTPUT_BYTES),
            read_capped(&mut child_stderr, MAX_SCRIPT_OUTPUT_BYTES)
        );
        // A script that floods a pipe will block once we stop reading. Kill it
        // before waiting instead of holding the daemon for the full timeout.
        if result.is_err() {
            let _ = child.kill().await;
        }
        // Close our read ends before waiting so a child blocked on a full
        // pipe exits instead of deadlocking the timeout path.
        drop(child_stdout);
        drop(child_stderr);
        let status = child.wait().await?;
        let (stdout, stderr) = result?;
        Ok::<_, io::Error>((status, stdout, stderr))
    })
    .await
    .map_err(|_| {
        FluxError::Plugin(format!(
            "script `{}` timed out after {:?}",
            invocation.name, invocation.timeout
        ))
    })?
    .map_err(|error| {
        FluxError::Plugin(format!(
            "failed to run script `{}`: {error}",
            invocation.name
        ))
    })?;

    let (status, stdout_bytes, stderr_bytes) = output;
    let stdout = String::from_utf8_lossy(&stdout_bytes).trim().to_string();
    let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();

    if !status.success() {
        let detail = if stderr.is_empty() {
            format!("exit status {status}")
        } else {
            stderr.clone()
        };
        return Err(FluxError::Plugin(format!(
            "script `{}` failed: {detail}",
            invocation.name
        )));
    }

    // Scripts frequently print debug text. Only structured stdout is treated
    // as updates; unparseable output is retained for diagnostics instead of
    // failing an otherwise successful invocation.
    let result = match parse_script_output(&stdout) {
        Ok(result) => result,
        Err(error) => {
            tracing::debug!(%error, "ignoring non-JSON script output");
            PluginResult::default()
        }
    };
    Ok(ScriptOutput {
        result,
        stdout,
        stderr,
    })
}

/// Parse stdout from a script. Accepts a full [`PluginResult`], a bare update
/// map, or nothing at all. When output is line-oriented the last valid JSON
/// line wins, which makes `echo '...'` inside shell scripts convenient.
pub fn parse_script_output(stdout: &str) -> Result<PluginResult> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(PluginResult::default());
    }

    let parsed = serde_json::from_str::<Value>(trimmed).or_else(|first_error| {
        trimmed
            .lines()
            .rev()
            .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
            .ok_or_else(|| {
                FluxError::Plugin(format!("script output is not valid JSON: {first_error}"))
            })
    })?;

    let Some(object) = parsed.as_object() else {
        return Err(FluxError::Plugin(
            "script output must be a JSON object".into(),
        ));
    };

    if object.contains_key("updates") || object.contains_key("events") || object.contains_key("log")
    {
        return serde_json::from_value(parsed)
            .map_err(|error| FluxError::Plugin(format!("invalid script result: {error}")));
    }

    // A bare map `{ "item": { "label": "..." } }` is treated as updates.
    let updates = serde_json::from_value(parsed)
        .map_err(|error| FluxError::Plugin(format!("invalid update map: {error}")))?;
    Ok(PluginResult {
        updates,
        events: Vec::new(),
        log: None,
    })
}

/// Ambient environment available to scripts and long-lived plugins.
pub fn ambient_environment(context: &ScriptContext) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if let Some(directory) = &context.config_dir {
        env.insert("CONFIG_DIR".into(), directory.display().to_string());
        env.insert("FLUXBAR_CONFIG_DIR".into(), directory.display().to_string());
    }
    if let Some(socket) = &context.socket {
        env.insert("FLUXBAR_SOCKET".into(), socket.display().to_string());
    }
    if let Ok(executable) = std::env::current_exe() {
        env.insert("FLUXBAR_BIN".into(), executable.display().to_string());
    }
    if let Some(directory) = &context.compat_dir {
        let mut paths = vec![directory.display().to_string()];
        if let Some(existing) = std::env::var_os("PATH") {
            paths.push(existing.to_string_lossy().to_string());
        }
        env.insert("PATH".into(), paths.join(":"));
        env.insert(
            "SKETCHYBAR".into(),
            directory.join("sketchybar").display().to_string(),
        );
    }
    env
}

/// Environment shared by plugin and item scripts.
pub fn environment(
    context: &ScriptContext,
    invocation: &ScriptInvocation,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("NAME".into(), invocation.name.clone());
    env.insert("SENDER".into(), sketchybar_sender(&invocation.event));
    env.insert("INFO".into(), event_info(&invocation.payload));
    env.insert("FLUXBAR_EVENT".into(), invocation.event.clone());
    env.insert("FLUXBAR_PAYLOAD".into(), invocation.payload.to_string());
    env.insert(
        "FLUXBAR_EVENT_JSON".into(),
        json!({
            "name": invocation.event,
            "sender": sketchybar_sender(&invocation.event),
            "payload": invocation.payload,
        })
        .to_string(),
    );
    env.extend(ambient_environment(context));
    env.extend(invocation.environment.clone());
    env
}

/// Whether an event is an internal state-change notification. A wildcard
/// subscription deliberately does not receive these, otherwise a script that
/// updates an item with `sketchybar --set` would trigger itself in a loop.
pub fn is_internal_event(event: &str) -> bool {
    matches!(
        event,
        "item_added" | "item_updated" | "item_configured" | "item_removed" | "item_renamed"
    )
}

/// Check an item/plugin subscription list. Exact names always match; `*`
/// matches every non-internal event.
pub fn subscription_matches(patterns: &[String], event: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| pattern == event || (pattern == "*" && !is_internal_event(event)))
}

/// Map FluxBar event names to their SketchyBar spelling.
pub fn sketchybar_sender(event: &str) -> String {
    match event {
        "app_focus" => "front_app_switched",
        "power_state" => "power_source_change",
        "mouse.left" => "mouse.exited",
        "tick" => "routine",
        other => other,
    }
    .to_string()
}

/// Render the event payload as SketchyBar's `$INFO`.
pub fn event_info(payload: &Value) -> String {
    match payload {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => {
            if let Some(value) = payload.get("app").and_then(Value::as_str) {
                return value.to_string();
            }
            if let Some(value) = payload.get("space").and_then(Value::as_str) {
                return value.to_string();
            }
            if let Some(value) = payload.get("volume").and_then(Value::as_u64) {
                return value.to_string();
            }
            if let Some(value) = payload.get("brightness").and_then(Value::as_u64) {
                return value.to_string();
            }
            if let Some(charging) = payload.get("charging").and_then(Value::as_bool) {
                return if charging { "AC" } else { "BATTERY" }.to_string();
            }
            if let Some(value) = payload.get("info").and_then(Value::as_str) {
                return value.to_string();
            }
            payload.to_string()
        }
    }
}

/// Build a command from a [`ScriptInvocation`].
fn build_command(context: &ScriptContext, invocation: &ScriptInvocation) -> Result<Command> {
    match &invocation.command {
        ScriptCommand::File { path, args } => {
            let path = resolve_path(context, path);
            let runtime = invocation
                .runtime
                .unwrap_or_else(|| PluginRuntime::from_entry(&path.to_string_lossy()));
            let (program, mut command_args) = interpreter(&runtime, &path);
            command_args.extend(args.clone());
            let mut command = Command::new(program);
            command.args(command_args);
            Ok(command)
        }
        ScriptCommand::Shell(script) => build_shell_command(context, invocation, script),
    }
}

fn build_shell_command(
    context: &ScriptContext,
    invocation: &ScriptInvocation,
    script: &str,
) -> Result<Command> {
    let trimmed = script.trim();
    let path = resolve_path(context, Path::new(trimmed));
    let runtime = invocation.runtime.unwrap_or_else(|| {
        if path.is_file() {
            PluginRuntime::from_entry(&path.to_string_lossy())
        } else {
            PluginRuntime::JsonRpc
        }
    });

    match runtime {
        PluginRuntime::Lua if path.is_file() => {
            let mut command = Command::new("lua");
            command.arg(path);
            Ok(command)
        }
        PluginRuntime::Lua => {
            let mut command = Command::new("lua");
            command.arg("-e").arg(trimmed);
            Ok(command)
        }
        PluginRuntime::Bash if path.is_file() => {
            let mut command = Command::new("/bin/bash");
            command.arg(path);
            Ok(command)
        }
        PluginRuntime::Bash => {
            let mut command = Command::new("/bin/bash");
            command.arg("-c").arg(trimmed);
            Ok(command)
        }
        PluginRuntime::Sh | PluginRuntime::JsonRpc if path.is_file() => {
            let mut command = Command::new("/bin/sh");
            command.arg(path);
            Ok(command)
        }
        PluginRuntime::Sh | PluginRuntime::JsonRpc => {
            let mut command = Command::new("/bin/sh");
            command.arg("-c").arg(trimmed);
            Ok(command)
        }
    }
}

fn interpreter(runtime: &PluginRuntime, path: &Path) -> (String, Vec<String>) {
    match runtime {
        PluginRuntime::Lua => ("lua".to_string(), vec![path.to_string_lossy().to_string()]),
        PluginRuntime::Bash => (
            "/bin/bash".to_string(),
            vec![path.to_string_lossy().to_string()],
        ),
        PluginRuntime::Sh | PluginRuntime::JsonRpc => (
            "/bin/sh".to_string(),
            vec![path.to_string_lossy().to_string()],
        ),
    }
}

pub(crate) fn expand_home(path: &Path) -> PathBuf {
    if let Ok(relative) = path.strip_prefix("~")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(relative);
    }
    path.to_path_buf()
}

fn resolve_path(context: &ScriptContext, path: &Path) -> PathBuf {
    let path = expand_home(path);
    if path.is_absolute() {
        return path;
    }
    context
        .config_dir
        .as_ref()
        .map(|directory| directory.join(&path))
        .filter(|candidate| candidate.exists())
        .unwrap_or(path)
}

/// Convert a parsed result event list into real events. Kept here so both
/// plugin and item script runners use identical semantics.
pub fn emitted_event(event: &PluginEmittedEvent) -> Event {
    Event::custom(event.name.clone(), event.payload.clone()).with_source("script")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn caps_script_output() {
        let (mut writer, mut reader) = tokio::io::duplex(32);
        writer.write_all(b"123456789").await.unwrap();
        drop(writer);
        let error = read_capped(&mut reader, 8).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn parses_full_result() {
        let result = parse_script_output(
            r#"{"updates":{"cpu":{"label":"42%"}},"events":[{"name":"demo"}],"log":"ok"}"#,
        )
        .unwrap();
        assert_eq!(result.updates["cpu"]["label"], "42%");
        assert_eq!(result.events[0].name, "demo");
        assert_eq!(result.log.as_deref(), Some("ok"));
    }

    #[test]
    fn parses_bare_update_map() {
        let result = parse_script_output(r#"{"cpu":{"label":"12%","progress":12.0}}"#).unwrap();
        assert_eq!(result.updates["cpu"]["label"], "12%");
    }

    #[test]
    fn parses_last_json_line() {
        let result = parse_script_output("debug\n{\"cpu\":{\"label\":\"1%\"}}\n").unwrap();
        assert_eq!(result.updates["cpu"]["label"], "1%");
    }

    #[test]
    fn maps_sketchybar_names() {
        assert_eq!(sketchybar_sender("app_focus"), "front_app_switched");
        assert_eq!(sketchybar_sender("mouse.left"), "mouse.exited");
        assert_eq!(sketchybar_sender("volume_change"), "volume_change");
        assert_eq!(sketchybar_sender("tick"), "routine");
    }

    #[test]
    fn info_uses_event_specific_fields() {
        assert_eq!(event_info(&json!({"app": "Safari"})), "Safari");
        assert_eq!(event_info(&json!({"volume": 32})), "32");
        assert_eq!(event_info(&json!({"charging": true})), "AC");
        assert_eq!(event_info(&json!("raw")), "raw");
    }

    #[tokio::test]
    async fn flooding_script_is_killed_promptly() {
        let context = ScriptContext::default();
        let mut invocation =
            ScriptInvocation::new("flood", "tick", ScriptCommand::Shell("yes x".into()));
        invocation.timeout = Duration::from_secs(30);
        let started = std::time::Instant::now();
        let error = run_script(&context, &invocation).await.unwrap_err();
        assert!(error.to_string().contains("output exceeds"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn runs_shell_script_and_parses_updates() {
        let context = ScriptContext::default();
        let invocation = ScriptInvocation::new(
            "demo",
            "tick",
            ScriptCommand::Shell(r#"printf '{"demo":{"label":"%s"}}' "$SENDER""#.to_string()),
        );
        let output = run_script(&context, &invocation).await.unwrap();
        assert_eq!(output.result.updates["demo"]["label"], "routine");
    }

    #[test]
    fn expands_home_paths() {
        let home = std::env::var_os("HOME").expect("HOME is set");
        assert_eq!(
            expand_home(Path::new("~/plugins/demo.sh")),
            PathBuf::from(home).join("plugins/demo.sh")
        );
        assert_eq!(
            expand_home(Path::new("relative/demo.sh")),
            PathBuf::from("relative/demo.sh")
        );
    }
}
