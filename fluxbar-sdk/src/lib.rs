//! # FluxBar SDK
//!
//! Writing a FluxBar plugin is a matter of implementing [`Plugin`] and calling
//! [`run`]:
//!
//! ```no_run
//! use fluxbar_sdk::{Plugin, PluginContext, PluginOutput, PluginEvent, run};
//!
//! struct Clock;
//!
//! #[fluxbar_sdk::async_trait]
//! impl Plugin for Clock {
//!     async fn on_init(&mut self, _ctx: &PluginContext) -> fluxbar_sdk::Result<()> {
//!         Ok(())
//!     }
//!
//!     async fn on_event(&mut self, event: &PluginEvent, _ctx: &PluginContext) -> fluxbar_sdk::Result<PluginOutput> {
//!         let mut out = PluginOutput::new();
//!         if event.kind() == "tick" {
//!             out.set_label("clock", chrono_like_now());
//!         }
//!         Ok(out)
//!     }
//! }
//!
//! fn chrono_like_now() -> String { "12:00".into() }
//!
//! #[tokio::main]
//! async fn main() -> fluxbar_sdk::Result<()> {
//!     run(Clock).await
//! }
//! ```
//!
//! The SDK takes care of the JSON-RPC framing over stdin/stdout, so plugins
//! stay small and typed.

use fluxbar_core::item::ItemUpdate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Re-exported so plugins can use `#[fluxbar_sdk::async_trait]` without
/// declaring their own dependency on `async-trait`.
pub use async_trait::async_trait;

/// SDK result type.
pub type Result<T> = std::result::Result<T, SdkError>;

/// SDK error type.
#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("plugin error: {0}")]
    Plugin(String),
}

/// Context passed to every plugin callback.
#[derive(Debug, Clone, Default)]
pub struct PluginContext {
    /// Plugin name, as registered in the FluxBar config.
    pub name: String,
    /// Free-form configuration from `[plugins.<name>.config]`.
    pub config: Value,
    /// Event names this plugin subscribed to.
    pub subscribe: Vec<String>,
    /// FluxBar version.
    pub version: String,
}

impl PluginContext {
    /// Look up a configuration string.
    pub fn config_str(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(Value::as_str)
    }

    /// Look up a configuration number.
    pub fn config_f64(&self, key: &str) -> Option<f64> {
        self.config.get(key).and_then(Value::as_f64)
    }
}

/// Event delivered to a plugin.
#[derive(Debug, Clone, Default)]
pub struct PluginEvent {
    kind: String,
    payload: Value,
    timestamp: i64,
    source: String,
}

impl PluginEvent {
    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn payload(&self) -> &Value {
        &self.payload
    }

    pub fn timestamp(&self) -> i64 {
        self.timestamp
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Deserialize the payload into a typed value.
    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.payload.clone()).map_err(Into::into)
    }
}

/// Data a plugin returns from a callback.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginOutput {
    /// Item updates keyed by item name.
    #[serde(default)]
    pub updates: BTreeMap<String, Value>,
    /// Events to emit on the bus.
    #[serde(default)]
    pub events: Vec<EmittedEvent>,
    /// Optional log message.
    #[serde(default)]
    pub log: Option<String>,
}

impl PluginOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace all fields of an item.
    pub fn set_update(&mut self, item: impl Into<String>, update: ItemUpdate) -> &mut Self {
        if let Ok(value) = serde_json::to_value(update) {
            self.updates.insert(item.into(), value);
        }
        self
    }

    /// Convenience: set an item's label.
    pub fn set_label(&mut self, item: impl Into<String>, label: impl Into<String>) -> &mut Self {
        self.merge_update(item.into(), json!({ "label": label.into() }));
        self
    }

    /// Convenience: set label and progress.
    pub fn set_progress(
        &mut self,
        item: impl Into<String>,
        label: impl Into<String>,
        progress: f64,
    ) -> &mut Self {
        self.merge_update(
            item.into(),
            json!({ "label": label.into(), "progress": progress }),
        );
        self
    }

    /// Convenience: set graph data (bar chart / line graph items).
    pub fn set_graph(&mut self, item: impl Into<String>, data: Vec<f64>) -> &mut Self {
        self.merge_update(item.into(), json!({ "graph": data }));
        self
    }

    /// Merge fields into an item's update object.
    fn merge_update(&mut self, item: String, fields: Value) {
        let entry = self
            .updates
            .entry(item)
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let (Value::Object(existing), Value::Object(fields)) = (entry, fields) {
            existing.extend(fields);
        }
    }

    /// Emit a custom event.
    pub fn emit(&mut self, name: impl Into<String>, payload: Value) -> &mut Self {
        self.events.push(EmittedEvent {
            name: name.into(),
            payload,
        });
        self
    }

    /// Attach a log line (printed by the daemon with the plugin name).
    pub fn log(&mut self, message: impl Into<String>) -> &mut Self {
        self.log = Some(message.into());
        self
    }
}

/// Event emitted by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmittedEvent {
    pub name: String,
    #[serde(default)]
    pub payload: Value,
}

/// The trait plugin authors implement.
///
/// Only [`Plugin::on_event`] is required; everything else has sensible
/// defaults.
#[async_trait::async_trait]
pub trait Plugin: Send + 'static {
    /// Called once when the plugin starts.
    async fn on_init(&mut self, _context: &PluginContext) -> Result<()> {
        Ok(())
    }

    /// Called for every event the plugin subscribed to.
    async fn on_event(
        &mut self,
        event: &PluginEvent,
        context: &PluginContext,
    ) -> Result<PluginOutput>;

    /// Called on the interval configured via `tick_interval`.
    async fn on_tick(&mut self, _context: &PluginContext) -> Result<PluginOutput> {
        Ok(PluginOutput::new())
    }

    /// Called before the process exits.
    async fn on_shutdown(&mut self, _context: &PluginContext) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct RpcIncoming {
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

/// Run a plugin until the host closes stdin or sends `shutdown`.
pub async fn run<P: Plugin>(mut plugin: P) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();
    let mut context = PluginContext::default();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let request: RpcIncoming = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": error.to_string() },
                });
                write_line(&mut stdout, &response).await?;
                continue;
            }
        };

        match request.method.as_str() {
            "init" => {
                context = PluginContext {
                    name: request
                        .params
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    config: request.params.get("config").cloned().unwrap_or(Value::Null),
                    subscribe: request
                        .params
                        .get("subscribe")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default(),
                    version: request
                        .params
                        .get("version")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                };
                let result = plugin.on_init(&context).await;
                respond(&mut stdout, &request.id, &result).await?;
            }
            "event" => {
                let event = PluginEvent {
                    kind: request
                        .params
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("event")
                        .to_string(),
                    payload: request
                        .params
                        .get("payload")
                        .cloned()
                        .unwrap_or(Value::Null),
                    timestamp: request
                        .params
                        .get("timestamp")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    source: request
                        .params
                        .get("source")
                        .and_then(Value::as_str)
                        .unwrap_or("core")
                        .to_string(),
                };

                let output = if event.kind() == "tick" {
                    plugin.on_tick(&context).await
                } else {
                    plugin.on_event(&event, &context).await
                };

                match output {
                    Ok(output) => {
                        let response = json!({
                            "jsonrpc": "2.0",
                            "id": request.id,
                            "result": output,
                        });
                        write_line(&mut stdout, &response).await?;
                    }
                    Err(error) => {
                        let response = json!({
                            "jsonrpc": "2.0",
                            "id": request.id,
                            "error": { "code": -32000, "message": error.to_string() },
                        });
                        write_line(&mut stdout, &response).await?;
                    }
                }
            }
            "shutdown" => {
                let _ = plugin.on_shutdown(&context).await;
                let response = json!({ "jsonrpc": "2.0", "id": request.id, "result": null });
                write_line(&mut stdout, &response).await?;
                break;
            }
            other => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": request.id,
                    "error": { "code": -32601, "message": format!("unknown method `{other}`") },
                });
                write_line(&mut stdout, &response).await?;
            }
        }
    }

    Ok(())
}

async fn respond<W: AsyncWriteExt + Unpin>(
    stdout: &mut W,
    id: &Value,
    result: &Result<()>,
) -> Result<()> {
    let response = match result {
        Ok(()) => json!({ "jsonrpc": "2.0", "id": id, "result": null }),
        Err(error) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32000, "message": error.to_string() },
        }),
    };
    write_line(stdout, &response).await
}

async fn write_line<W: AsyncWriteExt + Unpin>(stdout: &mut W, value: &Value) -> Result<()> {
    let mut encoded = serde_json::to_vec(value)?;
    encoded.push(b'\n');
    stdout.write_all(&encoded).await?;
    stdout.flush().await?;
    Ok(())
}

/// Synchronous fallback runner for blocking plugins and tests.
pub fn run_blocking<P: Plugin>(plugin: P) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| SdkError::Plugin(e.to_string()))?;
    runtime.block_on(run(plugin))
}

/// Flush stderr (handy when embedding the SDK in scripts).
pub fn flush_stderr() {
    let _ = std::io::stderr().flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlugin {
        ticks: usize,
    }

    #[async_trait::async_trait]
    impl Plugin for TestPlugin {
        async fn on_event(
            &mut self,
            event: &PluginEvent,
            _context: &PluginContext,
        ) -> Result<PluginOutput> {
            let mut output = PluginOutput::new();
            output.set_label("test", event.kind());
            self.ticks += 1;
            Ok(output)
        }
    }

    #[tokio::test]
    async fn plugin_receives_events() {
        let mut plugin = TestPlugin { ticks: 0 };
        let event = PluginEvent {
            kind: "build_finished".into(),
            ..PluginEvent::default()
        };
        let output = plugin
            .on_event(&event, &PluginContext::default())
            .await
            .unwrap();
        assert_eq!(output.updates["test"]["label"], "build_finished");
        assert_eq!(plugin.ticks, 1);
    }

    #[tokio::test]
    async fn output_builder() {
        let mut output = PluginOutput::new();
        output
            .set_progress("cpu", "42%", 42.0)
            .set_graph("cpu", vec![1.0, 2.0, 3.0])
            .emit("custom", json!({"ok": true}))
            .log("hello");

        assert_eq!(output.updates["cpu"]["label"], "42%");
        assert_eq!(output.updates["cpu"]["progress"], 42.0);
        assert_eq!(output.events.len(), 1);
        assert_eq!(output.log.as_deref(), Some("hello"));
    }

    #[test]
    fn context_accessors() {
        let context = PluginContext {
            name: "demo".into(),
            config: json!({"interval": 5, "path": "/tmp"}),
            ..PluginContext::default()
        };
        assert_eq!(context.config_f64("interval"), Some(5.0));
        assert_eq!(context.config_str("path"), Some("/tmp"));
        assert!(context.config_str("missing").is_none());
    }
}
