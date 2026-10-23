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
                "cannot connect to {}: {error}. Is `fluxbar start` running?",
                socket.display()
            ))
        })?;
        client.call(method, params).await
    })
}
