//! Blocking helpers for talking to a running daemon.

use fluxbar_core::error::{FluxError, Result};
use fluxbar_core::ipc::{IpcClient, default_socket_path};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Default socket path.
pub fn default_socket() -> PathBuf {
    default_socket_path()
}

/// Call a JSON-RPC method using a temporary current-thread runtime.
pub fn call(socket: &Path, method: &str, params: Value) -> Result<Value> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| FluxError::internal(format!("failed to start runtime: {e}")))?;

    runtime.block_on(async {
        let mut client = IpcClient::connect(socket).await.map_err(|error| {
            FluxError::IpcUnavailable(format!(
                "cannot connect to FluxBar at `{}`: {error}. Start it with `fluxbar start`, or check `--socket` / `fluxbar paths`.",
                socket.display()
            ))
        })?;
        client
            .call(method, params)
            .await
            .map_err(|error| with_method_context(method, error))
    })
}

/// Add the failing RPC method and a concrete next step to common errors.
fn with_method_context(method: &str, error: FluxError) -> FluxError {
    match error {
        FluxError::ItemNotFound(name) => FluxError::ItemNotFound(format!(
            "`{name}` while handling `{method}`; run `fluxbar list` to see configured items"
        )),
        FluxError::PluginNotFound(name) => FluxError::PluginNotFound(format!(
            "`{name}` while handling `{method}`; run `fluxbar plugin list` to see installed plugins"
        )),
        FluxError::InvalidArgument(message) => {
            FluxError::InvalidArgument(format!("{message} (while handling `{method}`)"))
        }
        FluxError::Config(message) => {
            FluxError::Config(format!("{message} (while handling `{method}`)"))
        }
        FluxError::Rpc { code, message } => {
            let remedy = if message.starts_with("item not found:") {
                "; run `fluxbar list` to see configured items"
            } else if message.starts_with("plugin not found:") {
                "; run `fluxbar plugin list` to see installed plugins"
            } else {
                ""
            };
            FluxError::Rpc {
                code,
                message: format!("{message}{remedy} (while handling `{method}`)"),
            }
        }
        other => other,
    }
}
