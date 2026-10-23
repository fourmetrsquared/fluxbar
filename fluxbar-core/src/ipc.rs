//! JSON-RPC 2.0 over Unix domain sockets.
//!
//! The protocol is newline-delimited JSON (one message per line), which keeps
//! it trivially debuggable with `socat`/`nc` while remaining typed on both
//! ends.

use crate::error::{FluxError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

/// JSON-RPC version string.
pub const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC request id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
    #[default]
    Null,
}

impl From<i64> for RequestId {
    fn from(value: i64) -> Self {
        Self::Number(value)
    }
}

/// Incoming JSON-RPC request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    #[serde(default = "default_version")]
    pub jsonrpc: String,
    #[serde(default)]
    pub id: RequestId,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

fn default_version() -> String {
    JSONRPC_VERSION.to_string()
}

/// Outgoing JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    #[serde(default = "default_version")]
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcResponse {
    pub fn success(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: RequestId, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    pub fn from_error(id: RequestId, error: &FluxError) -> Self {
        Self::failure(id, error.rpc_code(), error.to_string())
    }
}

/// Standard JSON-RPC error codes.
pub mod codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

/// Server-side request handling.
#[async_trait::async_trait]
pub trait RequestHandler: Send + Sync + 'static {
    /// Handle a single RPC request.
    async fn handle(&self, method: &str, params: Value) -> Result<Value>;

    /// Methods exposed by this handler (used for `rpc.discover`).
    fn methods(&self) -> Vec<&'static str>;
}

/// Unix-socket JSON-RPC server.
pub struct IpcServer {
    path: PathBuf,
    handler: std::sync::Arc<dyn RequestHandler>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl IpcServer {
    /// Bind a new server to `path`. Removes a stale socket file if present.
    pub fn bind(
        path: impl Into<PathBuf>,
        handler: std::sync::Arc<dyn RequestHandler>,
    ) -> Result<Self> {
        let path = path.into();
        if path.exists() {
            // Only remove if it is actually a socket; refuse to clobber real files.
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.file_type().is_socket() {
                std::fs::remove_file(&path)?;
            } else {
                return Err(FluxError::invalid(format!(
                    "refusing to remove non-socket file at {}",
                    path.display()
                )));
            }
        }
        Ok(Self {
            path,
            handler,
            shutdown: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run the accept loop until the shutdown signal fires.
    pub async fn run(mut self) -> Result<()> {
        let listener = UnixListener::bind(&self.path)?;
        info!(path = %self.path.display(), "IPC server listening");

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        self.shutdown = Some(shutdown_tx);

        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _addr)) => {
                            let handler = self.handler.clone();
                            tokio::spawn(async move {
                                if let Err(error) = serve_connection(stream, handler).await {
                                    debug!(%error, "IPC connection closed");
                                }
                            });
                        }
                        Err(error) => {
                            error!(%error, "failed to accept IPC connection");
                        }
                    }
                }
                _ = &mut shutdown_rx => {
                    info!("IPC server shutting down");
                    break;
                }
            }
        }

        // Best-effort cleanup of the socket file.
        let _ = std::fs::remove_file(&self.path);
        Ok(())
    }

    /// Request shutdown from another task.
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn serve_connection(
    stream: UnixStream,
    handler: std::sync::Arc<dyn RequestHandler>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<RpcRequest>(line) {
            Ok(request) => {
                let id = request.id.clone();
                match handler.handle(&request.method, request.params).await {
                    Ok(result) => RpcResponse::success(id, result),
                    Err(error) => RpcResponse::from_error(id, &error),
                }
            }
            Err(error) => {
                RpcResponse::failure(RequestId::Null, codes::PARSE_ERROR, error.to_string())
            }
        };

        let mut encoded = serde_json::to_vec(&response)?;
        encoded.push(b'\n');
        writer.write_all(&encoded).await?;
        writer.flush().await?;
    }

    Ok(())
}

/// Client used by the CLI and plugins.
pub struct IpcClient {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    next_id: i64,
}

impl IpcClient {
    /// Connect to a running FluxBar instance.
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let stream = UnixStream::connect(path.as_ref()).await?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
        })
    }

    /// Call a method and wait for the response.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
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
        self.writer.write_all(&encoded).await?;
        self.writer.flush().await?;

        let mut line = String::new();
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.reader.read_line(&mut line),
        )
        .await;
        let read = read.map_err(|_| FluxError::Rpc {
            code: codes::INTERNAL_ERROR,
            message: "timed out waiting for response".into(),
        })??;

        if read == 0 {
            return Err(FluxError::Rpc {
                code: codes::INTERNAL_ERROR,
                message: "server closed the connection".into(),
            });
        }

        // Skip notifications / unrelated ids.
        let response: RpcResponse = serde_json::from_str(line.trim())?;
        if let Some(error) = response.error {
            return Err(FluxError::Rpc {
                code: error.code,
                message: error.message,
            });
        }
        Ok(response.result.unwrap_or(Value::Null))
    }

    /// Check that the daemon is alive.
    pub async fn ping(&mut self) -> Result<()> {
        self.call("system.ping", json!({})).await.map(|_| ())
    }
}

/// Compute the default socket path.
#[must_use]
pub fn default_socket_path() -> PathBuf {
    if let Some(runtime) = directories::ProjectDirs::from("dev", "fluxbar", "fluxbar") {
        let dir = runtime.runtime_dir().unwrap_or_else(|| runtime.cache_dir());
        return dir.join("fluxbar.sock");
    }
    std::env::temp_dir().join("fluxbar.sock")
}

/// Helper used by tests and tooling: run a mock client/server pair.
pub async fn spawn_test_server(
    handler: std::sync::Arc<dyn RequestHandler>,
    path: PathBuf,
) -> tokio::task::JoinHandle<()> {
    let server = IpcServer::bind(path, handler).expect("bind test server");
    tokio::spawn(async move {
        if let Err(error) = server.run().await {
            warn!(%error, "test server stopped");
        }
    })
}

/// Convenience for sending a one-shot command, used by the CLI.
pub async fn call_socket(path: impl AsRef<Path>, method: &str, params: Value) -> Result<Value> {
    let mut client = IpcClient::connect(path).await?;
    client.call(method, params).await
}

/// Channel-based helper for building request/response shims.
pub type PendingRequest = oneshot::Sender<Result<Value>>;

/// Sender type for queueing server-side commands.
pub type CommandSender = mpsc::Sender<(RpcRequest, PendingRequest)>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct EchoHandler;

    #[async_trait::async_trait]
    impl RequestHandler for EchoHandler {
        async fn handle(&self, method: &str, params: Value) -> Result<Value> {
            match method {
                "system.ping" => Ok(json!({"pong": true})),
                "echo" => Ok(params),
                _ => Err(FluxError::NotImplemented(method.into())),
            }
        }

        fn methods(&self) -> Vec<&'static str> {
            vec!["system.ping", "echo"]
        }
    }

    #[tokio::test]
    async fn roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.sock");
        let handle = spawn_test_server(std::sync::Arc::new(EchoHandler), path.clone()).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut client = IpcClient::connect(&path).await.unwrap();
        assert_eq!(
            client.call("system.ping", json!({})).await.unwrap()["pong"],
            true
        );
        assert_eq!(
            client.call("echo", json!({"x": 1})).await.unwrap(),
            json!({"x": 1})
        );

        let error = client.call("unknown", json!({})).await.unwrap_err();
        assert_eq!(error.rpc_code(), -32601);

        handle.abort();
    }
}
