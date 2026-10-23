//! Event bus and event types.
//!
//! The bus uses a `tokio::sync::broadcast` channel. Every subscriber gets its
//! own receiver, and slow subscribers cannot block the producer.

use crate::error::{FluxError, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Built-in and custom event kinds.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", untagged)]
pub enum EventKind {
    /// Workspace / space changed.
    SpaceChange,
    /// Focused application changed.
    AppFocus,
    /// Output volume changed.
    VolumeChange,
    /// Display brightness changed.
    BrightnessChange,
    /// Power source / battery state changed.
    PowerState,
    /// Media playback changed.
    MediaChange,
    /// CPU load sample.
    CpuChange,
    /// Memory usage sample.
    MemoryChange,
    /// User-defined event.
    Custom(String),
}

impl EventKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SpaceChange => "space_change",
            Self::AppFocus => "app_focus",
            Self::VolumeChange => "volume_change",
            Self::BrightnessChange => "brightness_change",
            Self::PowerState => "power_state",
            Self::MediaChange => "media_change",
            Self::CpuChange => "cpu_change",
            Self::MemoryChange => "memory_change",
            Self::Custom(name) => name,
        }
    }

    /// Parse a well-known event name, falling back to [`EventKind::Custom`].
    pub fn parse(name: &str) -> Self {
        match name {
            "space_change" => Self::SpaceChange,
            "app_focus" => Self::AppFocus,
            "volume_change" => Self::VolumeChange,
            "brightness_change" => Self::BrightnessChange,
            "power_state" => Self::PowerState,
            "media_change" => Self::MediaChange,
            "cpu_change" => Self::CpuChange,
            "memory_change" => Self::MemoryChange,
            other => Self::Custom(other.to_string()),
        }
    }

    /// Whether this event is built in.
    pub const fn is_builtin(&self) -> bool {
        !matches!(self, Self::Custom(_))
    }
}

/// An event flowing through the bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Event kind / name.
    pub kind: EventKind,
    /// Arbitrary structured payload.
    #[serde(default)]
    pub payload: serde_json::Value,
    /// UNIX timestamp in milliseconds.
    #[serde(default = "now_millis")]
    pub timestamp: i64,
    /// Origin: `"system"`, `"cli"`, plugin name, etc.
    #[serde(default)]
    pub source: String,
}

fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Event {
    pub fn new(kind: EventKind, payload: serde_json::Value) -> Self {
        Self {
            kind,
            payload,
            timestamp: now_millis(),
            source: "core".to_string(),
        }
    }

    pub fn custom(name: impl Into<String>, payload: serde_json::Value) -> Self {
        Self::new(EventKind::Custom(name.into()), payload)
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Event name as a string.
    pub fn name(&self) -> &str {
        self.kind.as_str()
    }
}

/// Cloneable handle to the event bus.
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: broadcast::Sender<Arc<Event>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl EventBus {
    /// Create a bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(16));
        Self { sender }
    }

    /// Subscribe to all events.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.sender.subscribe()
    }

    /// Publish an event. Returns the number of active receivers.
    pub fn emit(&self, event: Event) -> usize {
        // A send error only means there are no subscribers, which is fine.
        self.sender.send(Arc::new(event)).unwrap_or(0)
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// An event handler that can be registered with a [`Bus`](EventBus).
#[async_trait::async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: Arc<Event>) -> Result<()>;
    fn event_kinds(&self) -> Vec<String>;
}

/// Helper that spawns a task forwarding bus events into a callback.
pub fn spawn_handler<F, Fut>(
    mut receiver: broadcast::Receiver<Arc<Event>>,
    state: Arc<()>,
    handler: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn(Arc<()>, Arc<Event>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => handler(state.clone(), event).await,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "event subscriber lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// Subscribe a typed handler to the bus.
pub fn subscribe<H: EventHandler + 'static>(
    bus: &EventBus,
    handler: Arc<H>,
) -> tokio::task::JoinHandle<()> {
    let mut receiver = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let kinds = handler.event_kinds();
                    let matches = kinds.is_empty()
                        || kinds.iter().any(|kind| kind == "*" || kind == event.name());
                    if matches {
                        if let Err(error) = handler.handle(event).await {
                            tracing::error!(%error, "event handler failed");
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "event handler lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// Validate that an event name is non-empty and reasonably sized.
pub fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(FluxError::invalid("event name must not be empty"));
    }
    if name.len() > 128 {
        return Err(FluxError::invalid(
            "event name must be at most 128 characters",
        ));
    }
    if name.chars().any(char::is_whitespace) {
        return Err(FluxError::invalid("event name must not contain whitespace"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kinds() {
        assert_eq!(EventKind::parse("space_change"), EventKind::SpaceChange);
        assert_eq!(
            EventKind::parse("build_finished"),
            EventKind::Custom("build_finished".into())
        );
        assert!(EventKind::parse("cpu_change").is_builtin());
        assert!(!EventKind::parse("custom").is_builtin());
    }

    #[tokio::test]
    async fn broadcast_delivers() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        bus.emit(Event::custom("hello", serde_json::json!({"a": 1})));
        let event = rx.recv().await.unwrap();
        assert_eq!(event.name(), "hello");
        assert_eq!(event.payload["a"], 1);
        assert_eq!(bus.subscriber_count(), 1);
    }

    #[test]
    fn validates_names() {
        assert!(validate_name("build_finished").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("has space").is_err());
    }
}
