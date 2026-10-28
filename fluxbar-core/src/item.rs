//! Bar items: runtime state and operations.

use crate::config::ItemConfig;
use crate::error::{FluxError, Result};
use crate::geometry::{Position, Rect};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique item identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemId(String);

impl ItemId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ItemId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for ItemId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Logical item kind. Mostly used to pick sensible defaults and icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    #[default]
    Text,
    Icon,
    Progress,
    Graph,
    Cpu,
    Memory,
    Battery,
    Media,
    Volume,
    Brightness,
    Time,
    Custom,
}

impl ItemKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Icon => "icon",
            Self::Progress => "progress",
            Self::Graph => "graph",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Battery => "battery",
            Self::Media => "media",
            Self::Volume => "volume",
            Self::Brightness => "brightness",
            Self::Time => "time",
            Self::Custom => "custom",
        }
    }
}

impl std::str::FromStr for ItemKind {
    type Err = FluxError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "icon" => Ok(Self::Icon),
            "progress" => Ok(Self::Progress),
            "graph" => Ok(Self::Graph),
            "cpu" => Ok(Self::Cpu),
            "memory" | "ram" => Ok(Self::Memory),
            "battery" => Ok(Self::Battery),
            "media" => Ok(Self::Media),
            "volume" => Ok(Self::Volume),
            "brightness" => Ok(Self::Brightness),
            "time" => Ok(Self::Time),
            "custom" => Ok(Self::Custom),
            other => Err(FluxError::invalid(format!("unknown item kind `{other}`"))),
        }
    }
}

impl fmt::Display for ItemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Runtime state of an item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemState {
    /// Label text shown next to the icon.
    pub label: String,

    /// Icon glyph / SF Symbol.
    pub icon: String,

    /// Optional progress value in `0..=100`.
    pub progress: Option<f64>,

    /// Numerical data used by graph renderers (newest last).
    #[serde(default)]
    pub graph: Vec<f64>,

    /// Short status string (e.g. "charging").
    #[serde(default)]
    pub status: Option<String>,

    /// Whether the item is currently visible.
    #[serde(default = "default_true")]
    pub visible: bool,
}

impl Default for ItemState {
    fn default() -> Self {
        Self {
            label: String::new(),
            icon: String::new(),
            progress: None,
            graph: Vec::new(),
            status: None,
            visible: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Who owns an item's existence and static configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemOrigin {
    /// Created from `fluxbar.toml`; a config reload reconciles it.
    #[default]
    Config,
    /// Created at runtime through IPC/CLI; config reloads must preserve it.
    Runtime,
}

/// A fully described bar item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub name: String,
    pub kind: ItemKind,
    pub position: Position,
    pub config: ItemConfig,
    pub state: ItemState,
    /// Ownership source used by transactional config reloads.
    #[serde(default)]
    pub origin: ItemOrigin,
    #[serde(skip)]
    pub bounds: Option<Rect>,
}

impl Item {
    pub fn new(name: impl Into<String>, config: ItemConfig) -> Self {
        let name = name.into();
        Self {
            kind: config.kind,
            position: config.position,
            state: ItemState {
                label: config.label.clone(),
                icon: config.icon.clone(),
                ..ItemState::default()
            },
            config,
            origin: ItemOrigin::Config,
            bounds: None,
            name,
        }
    }

    /// Apply a partial update received over IPC.
    pub fn apply_update(&mut self, update: ItemUpdate) {
        if let Some(label) = update.label {
            self.state.label = label;
        }
        if let Some(icon) = update.icon {
            self.state.icon = icon;
        }
        if let Some(progress) = update.progress {
            self.state.progress = (!progress.is_nan()).then_some(progress.clamp(0.0, 100.0));
        }
        if let Some(graph) = update.graph {
            self.state.graph = graph;
        }
        if let Some(status) = update.status {
            self.state.status = Some(status);
        }
        if let Some(visible) = update.visible {
            self.state.visible = visible;
        }
        if let Some(position) = update.position {
            self.position = position;
        }
        if let Some(kind) = update.kind {
            self.kind = kind;
        }
    }

    /// Push a new graph sample, keeping at most `limit` points.
    pub fn push_graph_sample(&mut self, value: f64, limit: usize) {
        self.state.graph.push(value);
        if self.state.graph.len() > limit {
            let excess = self.state.graph.len() - limit;
            self.state.graph.drain(0..excess);
        }
    }
}

/// Partial update used by CLI / IPC.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ItemUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
}

impl ItemUpdate {
    /// Parse `key=value` pairs as accepted by `fluxbar set`.
    pub fn from_assignments(pairs: &[String]) -> Result<Self> {
        let mut update = Self::default();
        for pair in pairs {
            let (key, raw) = pair
                .split_once('=')
                .ok_or_else(|| FluxError::invalid(format!("expected key=value, got `{pair}`")))?;
            let key = key.trim();
            let raw = raw.trim();

            match key {
                "label" => update.label = Some(raw.to_string()),
                "icon" => update.icon = Some(raw.to_string()),
                "progress" => {
                    update.progress =
                        Some(raw.parse::<f64>().map_err(|_| {
                            FluxError::invalid(format!("invalid progress `{raw}`"))
                        })?);
                }
                "graph" => {
                    let values = raw
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| {
                            s.parse::<f64>().map_err(|_| {
                                FluxError::invalid(format!("invalid graph value `{s}`"))
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    update.graph = Some(values);
                }
                "status" => update.status = Some(raw.to_string()),
                "visible" => {
                    update.visible = Some(match raw {
                        "true" | "1" | "yes" => true,
                        "false" | "0" | "no" => false,
                        other => {
                            return Err(FluxError::invalid(format!("invalid boolean `{other}`")));
                        }
                    });
                }
                "position" => update.position = Some(raw.parse()?),
                "kind" => update.kind = Some(raw.parse()?),
                other => {
                    return Err(FluxError::invalid(format!(
                        "unknown item property `{other}`"
                    )));
                }
            }
        }
        Ok(update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_updates() {
        let mut item = Item::new("cpu", ItemConfig::default());
        item.apply_update(ItemUpdate {
            label: Some("42%".into()),
            progress: Some(150.0),
            ..ItemUpdate::default()
        });
        assert_eq!(item.state.label, "42%");
        assert_eq!(item.state.progress, Some(100.0));

        item.apply_update(ItemUpdate {
            progress: Some(-5.0),
            ..ItemUpdate::default()
        });
        assert_eq!(item.state.progress, Some(0.0));

        item.apply_update(ItemUpdate {
            progress: Some(f64::NAN),
            ..ItemUpdate::default()
        });
        assert_eq!(item.state.progress, None);
    }

    #[test]
    fn parses_assignments() {
        let update = ItemUpdate::from_assignments(&[
            "label=Hello".into(),
            "icon=x".into(),
            "progress=12.5".into(),
            "graph=1,2,3".into(),
            "visible=false".into(),
        ])
        .unwrap();

        assert_eq!(update.label.as_deref(), Some("Hello"));
        assert_eq!(update.progress, Some(12.5));
        assert_eq!(update.graph, Some(vec![1.0, 2.0, 3.0]));
        assert_eq!(update.visible, Some(false));

        assert!(ItemUpdate::from_assignments(&["nope".into()]).is_err());
        assert!(ItemUpdate::from_assignments(&["progress=abc".into()]).is_err());
    }

    #[test]
    fn graph_is_bounded() {
        let mut item = Item::new("graph", ItemConfig::default());
        for value in 0..10 {
            item.push_graph_sample(f64::from(value), 5);
        }
        assert_eq!(item.state.graph, vec![5.0, 6.0, 7.0, 8.0, 9.0]);
    }
}
