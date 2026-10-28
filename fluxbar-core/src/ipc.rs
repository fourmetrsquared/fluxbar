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
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tracing::{debug, error, info, warn};

/// JSON-RPC version string.
pub const JSONRPC_VERSION: &str = "2.0";

/// Maximum size of one newline-delimited JSON-RPC message.
///
/// The limit applies in both directions so a malformed peer cannot make the
/// daemon or a client allocate an unbounded amount of memory.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Maximum number of concurrent IPC connections. Excess clients are closed
/// immediately instead of consuming an unbounded number of tasks.
const MAX_CONNECTIONS: usize = 64;

/// Close idle connections so abandoned clients cannot accumulate.
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

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
    connections: std::sync::Arc<Semaphore>,
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
                // Never unlink a live daemon's socket. A connect attempt is the
                // cheapest portable liveness check for a Unix socket.
                if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                    return Err(FluxError::invalid(format!(
                        "another FluxBar instance is already listening at {}",
                        path.display()
                    )));
                }
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
            connections: std::sync::Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run the accept loop until the shutdown signal fires.
    pub async fn run(mut self) -> Result<()> {
        let listener = UnixListener::bind(&self.path)?;
        if let Err(error) = restrict_socket_permissions(&self.path) {
            // Do not leave a world-accessible control socket behind if the
            // permission change failed.
            remove_socket_file(&self.path);
            return Err(error.into());
        }
        info!(path = %self.path.display(), "IPC server listening");

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        self.shutdown = Some(shutdown_tx);

        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _addr)) => {
                            let Ok(permit) = self.connections.clone().try_acquire_owned() else {
                                warn!("too many IPC connections; rejecting client");
                                drop(stream);
                                continue;
                            };
                            let handler = self.handler.clone();
                            tokio::spawn(async move {
                                let _permit = permit;
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
        remove_socket_file(&self.path);
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
        remove_socket_file(&self.path);
    }
}

fn remove_socket_file(path: &Path) {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_socket() {
            let _ = std::fs::remove_file(path);
        }
    }
}

async fn serve_connection(
    stream: UnixStream,
    handler: std::sync::Arc<dyn RequestHandler>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    loop {
        let line = match tokio::time::timeout(
            CONNECTION_IDLE_TIMEOUT,
            read_line_limited(&mut reader, MAX_MESSAGE_BYTES),
        )
        .await
        {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) | Err(_) => break,
            Ok(Err(error)) => {
                let response = RpcResponse::failure(
                    RequestId::Null,
                    codes::INVALID_REQUEST,
                    error.to_string(),
                );
                write_response(&mut writer, &response).await?;
                break;
            }
        };

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

        write_response(&mut writer, &response).await?;
    }

    Ok(())
}

async fn write_response(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    response: &RpcResponse,
) -> Result<()> {
    let mut encoded = serde_json::to_vec(response)?;
    if encoded.len() > MAX_MESSAGE_BYTES {
        encoded = serde_json::to_vec(&RpcResponse::failure(
            RequestId::Null,
            codes::INTERNAL_ERROR,
            format!("response exceeds {MAX_MESSAGE_BYTES} bytes"),
        ))?;
    }
    encoded.push(b'\n');
    writer.write_all(&encoded).await?;
    writer.flush().await?;
    Ok(())
}

/// Read one newline-delimited message without ever buffering more than `max`
/// bytes. The returned string has the trailing `\n` and optional `\r` removed.
pub(crate) async fn read_line_limited<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.unwrap_or(available.len());
        if line.len().saturating_add(take) > max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("message exceeds {max} bytes"),
            ));
        }
        line.extend_from_slice(&available[..take]);
        reader.consume(take + usize::from(newline.is_some()));
        if newline.is_some() {
            break;
        }
    }

    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line).map(Some).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "message is not valid UTF-8",
        )
    })
}

/// Restrict the control socket to the current user. The containing directory
/// is also expected to be private; the socket mode is the last line of defence.
fn restrict_socket_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
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
        let stream =
            tokio::time::timeout(Duration::from_secs(5), UnixStream::connect(path.as_ref()))
                .await
                .map_err(|_| FluxError::Rpc {
                    code: codes::INTERNAL_ERROR,
                    message: "timed out connecting to the FluxBar socket".into(),
                })??;
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
        tokio::time::timeout(Duration::from_secs(10), async {
            self.writer.write_all(&encoded).await?;
            self.writer.flush().await
        })
        .await
        .map_err(|_| FluxError::Rpc {
            code: codes::INTERNAL_ERROR,
            message: "timed out sending request to FluxBar".into(),
        })??;

        let line = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            read_line_limited(&mut self.reader, MAX_MESSAGE_BYTES),
        )
        .await
        .map_err(|_| FluxError::Rpc {
            code: codes::INTERNAL_ERROR,
            message: "timed out waiting for response".into(),
        })??
        .ok_or_else(|| FluxError::Rpc {
            code: codes::INTERNAL_ERROR,
            message: "server closed the connection".into(),
        })?;

        let response: RpcResponse = serde_json::from_str(line.trim())?;
        if let Some(error) = response.error {
            return Err(FluxError::Rpc {
                code: error.code,
                message: error.message,
            });
        }
        if response.id != RequestId::Number(id) {
            return Err(FluxError::Rpc {
                code: codes::INTERNAL_ERROR,
                message: "server returned a response with the wrong request id".into(),
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
    // Never fall back to a shared directory such as /tmp: another local user
    // could pre-create or replace the socket. HOME is private to the user and
    // is only used when the platform directory lookup is unavailable.
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".cache")
            .join("fluxbar")
            .join("fluxbar.sock");
    }
    // Last resort for environments without HOME and without platform
    // directories. The socket is still chmod 0600; set an explicit --socket
    // path if the temporary directory is shared.
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
    std::env::temp_dir().join(format!("fluxbar-{user}.sock"))
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
    use tokio::io::AsyncWriteExt;

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

    struct HugeHandler;

    #[async_trait::async_trait]
    impl RequestHandler for HugeHandler {
        async fn handle(&self, _method: &str, _params: Value) -> Result<Value> {
            Ok(json!({ "data": "x".repeat(MAX_MESSAGE_BYTES + 1) }))
        }

        fn methods(&self) -> Vec<&'static str> {
            vec!["huge"]
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

    #[tokio::test]
    async fn socket_is_owner_only() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("private.sock");
            let handle = spawn_test_server(std::sync::Arc::new(EchoHandler), path.clone()).await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            handle.abort();
        }
    }

    #[tokio::test]
    async fn refuses_to_replace_a_live_socket() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("live.sock");
        let handle = spawn_test_server(std::sync::Arc::new(EchoHandler), path.clone()).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let error = match IpcServer::bind(path.clone(), std::sync::Arc::new(EchoHandler)) {
            Ok(_) => panic!("a live socket must not be replaced"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("already listening"));
        assert!(path.exists());

        handle.abort();
    }

    #[tokio::test]
    async fn removes_a_stale_socket() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("stale.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        drop(listener);
        assert!(path.exists());

        let server = IpcServer::bind(path.clone(), std::sync::Arc::new(EchoHandler)).unwrap();
        assert_eq!(server.path(), path.as_path());
    }

    #[tokio::test]
    async fn rejects_oversized_messages() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("limit.sock");
        let handle = spawn_test_server(std::sync::Arc::new(EchoHandler), path.clone()).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut stream = UnixStream::connect(&path).await.unwrap();
        stream
            .write_all(&vec![b'a'; MAX_MESSAGE_BYTES + 1])
            .await
            .unwrap();

        let mut reader = BufReader::new(stream);
        let line = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            read_line_limited(&mut reader, MAX_MESSAGE_BYTES),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let response: RpcResponse = serde_json::from_str(&line).unwrap();
        assert_eq!(response.error.unwrap().code, codes::INVALID_REQUEST);

        handle.abort();
    }

    #[tokio::test]
    async fn rejects_oversized_responses() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("response.sock");
        let handle = spawn_test_server(std::sync::Arc::new(HugeHandler), path.clone()).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut client = IpcClient::connect(&path).await.unwrap();
        let error = client.call("huge", json!({})).await.unwrap_err();
        assert_eq!(error.rpc_code(), codes::INTERNAL_ERROR);

        handle.abort();
    }
}
