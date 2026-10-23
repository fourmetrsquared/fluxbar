//! Shared bar state.

use crate::config::Config;
use crate::error::{FluxError, Result};
use crate::event::{Event, EventBus};
use crate::geometry::Position;
use crate::item::{Item, ItemState, ItemUpdate};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Full bar state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BarState {
    /// Items keyed by name. `BTreeMap` keeps ordering stable for rendering.
    pub items: BTreeMap<String, Item>,
    /// Current workspace, if known.
    pub current_space: Option<String>,
    /// Focused application, if known.
    pub current_app: Option<String>,
    /// Monotonic version bumped on every change.
    pub version: u64,
}

impl BarState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_version(&mut self) -> u64 {
        self.version = self.version.wrapping_add(1);
        self.version
    }

    /// Items belonging to a position, sorted by drawing order then name.
    pub fn items_at(&self, position: Position) -> Vec<&Item> {
        let mut items: Vec<&Item> = self
            .items
            .values()
            .filter(|item| item.position == position)
            .collect();
        items.sort_by(|a, b| {
            a.config
                .drawing_order
                .cmp(&b.config.drawing_order)
                .then_with(|| a.name.cmp(&b.name))
        });
        items
    }
}

/// Thread-safe handle to the bar state and event bus.
#[derive(Debug, Clone)]
pub struct StateHandle {
    inner: Arc<RwLock<BarState>>,
    bus: EventBus,
    config: Arc<RwLock<Config>>,
}

impl StateHandle {
    pub fn new(config: Config) -> Self {
        let mut state = BarState::new();
        state.items = config
            .items
            .keys()
            .map(|name| (name.clone(), Item::new(name.clone(), config.item(name))))
            .collect();

        Self {
            inner: Arc::new(RwLock::new(state)),
            bus: EventBus::default(),
            config: Arc::new(RwLock::new(config)),
        }
    }

    /// Read access to the state.
    pub fn read(&self) -> parking_lot::RwLockReadGuard<'_, BarState> {
        self.inner.read()
    }

    /// Write access to the state.
    pub fn write(&self) -> parking_lot::RwLockWriteGuard<'_, BarState> {
        self.inner.write()
    }

    /// Access the event bus.
    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    /// Access the configuration.
    pub fn config(&self) -> parking_lot::RwLockReadGuard<'_, Config> {
        self.config.read()
    }

    /// Replace the configuration (e.g. after a hot reload).
    pub fn replace_config(&self, config: Config) {
        *self.config.write() = config;
    }

    /// Emit an event.
    pub fn emit(&self, event: Event) -> usize {
        self.bus.emit(event)
    }

    /// Add a new item.
    pub fn add_item(
        &self,
        name: &str,
        kind: crate::item::ItemKind,
        position: Position,
    ) -> Result<Item> {
        validate_item_name(name)?;
        let mut guard = self.write();
        if guard.items.contains_key(name) {
            return Err(FluxError::invalid(format!("item `{name}` already exists")));
        }
        let mut config = self.config.read().item(name);
        config.kind = kind;
        config.position = position;
        let item = Item::new(name, config);
        guard.items.insert(name.to_string(), item.clone());
        guard.next_version();
        drop(guard);
        self.emit(Event::custom(
            "item_added",
            serde_json::json!({"name": name}),
        ));
        Ok(item)
    }

    /// Update an existing item.
    pub fn update_item(&self, name: &str, update: ItemUpdate) -> Result<Item> {
        let mut guard = self.write();
        let item = guard
            .items
            .get_mut(name)
            .ok_or_else(|| FluxError::ItemNotFound(name.to_string()))?;
        item.apply_update(update);
        let snapshot = item.clone();
        guard.next_version();
        drop(guard);
        self.emit(Event::custom(
            "item_updated",
            serde_json::json!({"name": name}),
        ));
        Ok(snapshot)
    }

    /// Replace the full state of an item (used by plugins).
    pub fn set_item_state(&self, name: &str, state: ItemState) -> Result<()> {
        let mut guard = self.write();
        let item = guard
            .items
            .get_mut(name)
            .ok_or_else(|| FluxError::ItemNotFound(name.to_string()))?;
        item.state = state;
        guard.next_version();
        Ok(())
    }

    /// Remove an item.
    pub fn remove_item(&self, name: &str) -> Result<Item> {
        let mut guard = self.write();
        let item = guard
            .items
            .remove(name)
            .ok_or_else(|| FluxError::ItemNotFound(name.to_string()))?;
        guard.next_version();
        drop(guard);
        self.emit(Event::custom(
            "item_removed",
            serde_json::json!({"name": name}),
        ));
        Ok(item)
    }

    /// Fetch a single item.
    pub fn item(&self, name: &str) -> Option<Item> {
        self.read().items.get(name).cloned()
    }

    /// List all items.
    pub fn items(&self) -> Vec<Item> {
        self.read().items.values().cloned().collect()
    }

    /// Apply a plugin-provided update map: `item -> partial fields`.
    pub fn apply_plugin_updates(
        &self,
        updates: &BTreeMap<String, serde_json::Value>,
    ) -> Result<usize> {
        let mut applied = 0;
        for (name, value) in updates {
            let update: ItemUpdate = serde_json::from_value(value.clone())
                .map_err(|e| FluxError::Plugin(format!("invalid update for `{name}`: {e}")))?;
            let mut guard = self.write();
            if let Some(item) = guard.items.get_mut(name) {
                item.apply_update(update);
                guard.next_version();
                applied += 1;
            }
        }
        Ok(applied)
    }

    /// Update the focused space / app.
    pub fn set_focus(&self, space: Option<String>, app: Option<String>) {
        let mut guard = self.write();
        guard.current_space = space;
        guard.current_app = app;
        guard.next_version();
    }
}

/// Validate an item name.
pub fn validate_item_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(FluxError::invalid("item name must not be empty"));
    }
    if name.len() > 64 {
        return Err(FluxError::invalid(
            "item name must be at most 64 characters",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(FluxError::invalid(
            "item name may only contain letters, digits, `-`, `_` and `.`",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ItemKind;

    fn handle() -> StateHandle {
        StateHandle::new(Config::default())
    }

    #[test]
    fn add_update_remove() {
        let handle = handle();
        handle
            .add_item("cpu", ItemKind::Cpu, Position::Left)
            .unwrap();
        assert!(handle.item("cpu").is_some());

        let update = ItemUpdate {
            label: Some("12%".into()),
            progress: Some(12.0),
            ..ItemUpdate::default()
        };
        handle.update_item("cpu", update).unwrap();
        assert_eq!(handle.item("cpu").unwrap().state.label, "12%");

        handle.remove_item("cpu").unwrap();
        assert!(handle.item("cpu").is_none());
    }

    #[test]
    fn duplicate_item_is_rejected() {
        let handle = handle();
        handle
            .add_item("a", ItemKind::Text, Position::Left)
            .unwrap();
        assert!(
            handle
                .add_item("a", ItemKind::Text, Position::Left)
                .is_err()
        );
    }

    #[test]
    fn invalid_names_are_rejected() {
        assert!(validate_item_name("ok-name_1").is_ok());
        assert!(validate_item_name("bad name").is_err());
        assert!(validate_item_name("").is_err());
    }

    #[test]
    fn items_are_positioned() {
        let handle = handle();
        handle
            .add_item("a", ItemKind::Text, Position::Left)
            .unwrap();
        handle
            .add_item("b", ItemKind::Text, Position::Right)
            .unwrap();
        assert_eq!(handle.read().items_at(Position::Left).len(), 1);
        assert_eq!(handle.read().items_at(Position::Right).len(), 1);
    }

    #[test]
    fn apply_plugin_updates() {
        let handle = handle();
        handle
            .add_item("cpu", ItemKind::Cpu, Position::Left)
            .unwrap();
        let mut updates = BTreeMap::new();
        updates.insert("cpu".to_string(), serde_json::json!({"label": "42%"}));
        assert_eq!(handle.apply_plugin_updates(&updates).unwrap(), 1);
        assert_eq!(handle.item("cpu").unwrap().state.label, "42%");
    }
}
