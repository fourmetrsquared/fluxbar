//! Error types for FluxBar.

use thiserror::Error;

/// Unified error type used across all FluxBar components.
#[derive(Error, Debug)]
pub enum FluxError {
    /// Configuration could not be parsed or is invalid.
    #[error("configuration error: {0}")]
    Config(String),

    /// Failed to read or write a configuration file.
    #[error("config file error: {0}")]
    ConfigFile(String),

    /// TOML deserialization failed.
    #[error("toml parse error: {0}")]
    Toml(#[from] toml::de::Error),

    /// TOML serialization failed.
    #[error("toml serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    /// Generic I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization failure.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// JSON-RPC protocol error.
    #[error("rpc error {code}: {message}")]
    Rpc { code: i32, message: String },

    /// The daemon could not be reached.
    #[error("{0}")]
    IpcUnavailable(String),

    /// A plugin failed or could not be found.
    #[error("plugin error: {0}")]
    Plugin(String),

    /// A plugin with the given name is unknown.
    #[error("plugin not found: {0}")]
    PluginNotFound(String),

    /// The renderer reported an error.
    #[error("renderer error: {0}")]
    Renderer(String),

    /// The event system reported an error.
    #[error("event error: {0}")]
    Event(String),

    /// An item with the given name is unknown.
    #[error("item not found: {0}")]
    ItemNotFound(String),

    /// The caller supplied an invalid argument.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// The requested feature is not implemented yet.
    #[error("not implemented: {0}")]
    NotImplemented(String),

    /// Unexpected internal failure.
    #[error("internal error: {0}")]
    Internal(String),
}

impl FluxError {
    /// Convenience constructor for [`FluxError::Internal`].
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// Convenience constructor for [`FluxError::InvalidArgument`].
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidArgument(message.into())
    }

    /// JSON-RPC error code associated with this error.
    pub const fn rpc_code(&self) -> i32 {
        match self {
            Self::Rpc { code, .. } => *code,
            Self::InvalidArgument(_) => -32602,
            Self::ItemNotFound(_) | Self::PluginNotFound(_) => -32004,
            Self::NotImplemented(_) => -32601,
            _ => -32603,
        }
    }
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, FluxError>;
