//! JSON-RPC request handler for the daemon.

use fluxbar_core::error::{FluxError, Result};
use fluxbar_core::event::Event;
use fluxbar_core::geometry::Position;
use fluxbar_core::ipc::RequestHandler;
use fluxbar_core::item::{ItemKind, ItemUpdate};
use fluxbar_core::plugin::PluginHost;
use fluxbar_core::renderer::Renderer;
use fluxbar_core::state::StateHandle;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Shared daemon state exposed over JSON-RPC.
pub struct DaemonHandler {
    pub state: StateHandle,
    pub plugins: Arc<PluginHost>,
    pub renderer: Arc<dyn Renderer>,
    pub started: Instant,
    pub frames: Arc<AtomicU64>,
    pub requests: Arc<AtomicU64>,
    pub errors: Arc<AtomicU64>,
    pub shutdown: Arc<AtomicBool>,
    pub config_path: Option<PathBuf>,
    pub bar_visible: Arc<AtomicBool>,
    pub(crate) reload: Option<Arc<crate::daemon::RuntimeReload>>,
}

#[async_trait::async_trait]
impl RequestHandler for DaemonHandler {
    #[tracing::instrument(name = "ipc.request", skip(self, params), fields(method = %method))]
    async fn handle(&self, method: &str, params: Value) -> Result<Value> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let result = match method {
            "system.ping" => Ok(json!({"pong": true, "version": fluxbar_core::VERSION})),
            "system.status" => Ok(self.status()),
            "system.shutdown" => {
                self.shutdown.store(true, Ordering::Relaxed);
                Ok(json!({"ok": true}))
            }
            "item.add" => self.item_add(params),
            "item.set" => self.item_set(params),
            "item.configure" => self.item_configure(params),
            "item.subscribe" => self.item_subscribe(params),
            "item.rename" => {
                let old = required_str(&params, "old")?;
                let new = required_str(&params, "new")?;
                let item = self.state.rename_item(&old, &new)?;
                Ok(json!({"old": old, "new": item.name}))
            }
            "item.remove" => {
                let name = required_str(&params, "name")?;
                let item = self.state.remove_item(&name)?;
                Ok(json!({"name": item.name}))
            }
            "item.get" => {
                let name = required_str(&params, "name")?;
                let item = self
                    .state
                    .item(&name)
                    .ok_or_else(|| FluxError::ItemNotFound(name.clone()))?;
                Ok(serde_json::to_value(item)?)
            }
            "item.list" => {
                let items: Vec<Value> = self
                    .state
                    .items()
                    .into_iter()
                    .map(|item| {
                        json!({
                            "name": item.name,
                            "kind": item.kind.as_str(),
                            "position": item.position.as_str(),
                            "visible": item.state.visible,
                            "label": item.state.label,
                            "icon": item.state.icon,
                            "progress": item.state.progress,
                        })
                    })
                    .collect();
                Ok(Value::Array(items))
            }
            "event.add" => {
                let name = required_str(&params, "name")?;
                fluxbar_core::event::validate_name(&name)?;
                Ok(json!({"added": name}))
            }
            "event.trigger" => {
                let name = required_str(&params, "name")?;
                fluxbar_core::event::validate_name(&name)?;
                let payload = params.get("payload").cloned().unwrap_or(Value::Null);
                let environment = params
                    .get("environment")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|e| FluxError::invalid(format!("invalid environment: {e}")))?
                    .unwrap_or_default();
                self.state.emit(
                    Event::custom(name.clone(), payload)
                        .with_source("cli")
                        .with_environment(environment),
                );
                Ok(json!({"triggered": name}))
            }
            "bar.configure" => self.bar_configure(params),
            "defaults.configure" => {
                let patch = params.get("config").cloned().unwrap_or_else(|| json!({}));
                let defaults = self.state.configure_defaults(&patch)?;
                Ok(serde_json::to_value(defaults)?)
            }
            "config.reload" => self.config_reload(),
            "config.path" => Ok(json!({"path": self.config_path})),
            "plugin.list" => Ok(Value::Array(self.plugins.list())),
            "plugin.restart" => {
                let name = required_str(&params, "name")?;
                self.plugins.restart(&name).await?;
                Ok(json!({"restarted": name}))
            }
            "plugin.stop" => {
                let name = required_str(&params, "name")?;
                self.plugins.stop(&name).await?;
                Ok(json!({"stopped": name}))
            }
            "bar.show" => {
                self.bar_visible.store(true, Ordering::Relaxed);
                self.renderer.set_visible(true)?;
                Ok(json!({"visible": true}))
            }
            "bar.hide" => {
                self.bar_visible.store(false, Ordering::Relaxed);
                self.renderer.set_visible(false)?;
                Ok(json!({"visible": false}))
            }
            other => Err(FluxError::NotImplemented(other.to_string())),
        };
        if result.is_err() {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn methods(&self) -> Vec<&'static str> {
        vec![
            "system.ping",
            "system.status",
            "system.shutdown",
            "item.add",
            "item.set",
            "item.configure",
            "item.subscribe",
            "item.rename",
            "item.remove",
            "item.get",
            "item.list",
            "event.add",
            "event.trigger",
            "bar.configure",
            "defaults.configure",
            "config.reload",
            "config.path",
            "plugin.list",
            "plugin.restart",
            "plugin.stop",
            "bar.show",
            "bar.hide",
        ]
    }
}

impl DaemonHandler {
    fn status(&self) -> Value {
        let state = self.state.read();
        let plugins = self.plugins.list();
        let running_plugins = plugins
            .iter()
            .filter(|plugin| plugin["running"].as_bool().unwrap_or(false))
            .count();
        let plugin_restarts: u64 = plugins
            .iter()
            .map(|plugin| plugin["restarts"].as_u64().unwrap_or(0))
            .sum();
        let frames = self.frames.load(Ordering::Relaxed);
        let events = self.state.bus().stats();
        json!({
            "version": fluxbar_core::VERSION,
            "uptime_secs": self.started.elapsed().as_secs(),
            "items": state.items.len(),
            "plugins": running_plugins,
            "frames": frames,
            "config": self.config_path.as_ref().map(|p| p.display().to_string()),
            "space": state.current_space,
            "app": state.current_app,
            "counters": {
                "events": events.emitted,
                "dropped_events": events.dropped,
                "event_subscribers": events.subscribers,
                "frames": frames,
                "plugin_restarts": plugin_restarts,
                "ipc_requests": self.requests.load(Ordering::Relaxed),
                "ipc_errors": self.errors.load(Ordering::Relaxed),
            },
        })
    }

    fn item_add(&self, params: Value) -> Result<Value> {
        let name = required_str(&params, "name")?;
        let kind: ItemKind = params
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("text")
            .parse()?;
        let position: Position = params
            .get("position")
            .and_then(Value::as_str)
            .unwrap_or("left")
            .parse()?;

        let item = self.state.add_item(&name, kind, position)?;
        if let Some(label) = params.get("label").and_then(Value::as_str) {
            self.state.update_item(
                &name,
                ItemUpdate {
                    label: Some(label.to_string()),
                    ..Default::default()
                },
            )?;
        }
        if let Some(icon) = params.get("icon").and_then(Value::as_str) {
            self.state.update_item(
                &name,
                ItemUpdate {
                    icon: Some(icon.to_string()),
                    ..Default::default()
                },
            )?;
        }
        Ok(json!({
            "name": item.name,
            "kind": item.kind.as_str(),
            "position": item.position.as_str(),
        }))
    }

    fn item_set(&self, params: Value) -> Result<Value> {
        let name = required_str(&params, "name")?;
        let update: ItemUpdate = match params.get("update") {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|e| FluxError::invalid(format!("invalid update: {e}")))?,
            None => {
                let mut rest = params.clone();
                if let Some(object) = rest.as_object_mut() {
                    object.remove("name");
                }
                serde_json::from_value(rest)
                    .map_err(|e| FluxError::invalid(format!("invalid update: {e}")))?
            }
        };
        let item = self.state.update_item(&name, update)?;
        Ok(json!({
            "name": item.name,
            "state": {
                "label": item.state.label,
                "icon": item.state.icon,
                "progress": item.state.progress,
                "visible": item.state.visible,
            }
        }))
    }

    fn item_configure(&self, params: Value) -> Result<Value> {
        let name = required_str(&params, "name")?;
        let patch = params.get("config").cloned().unwrap_or_else(|| json!({}));
        let update: Option<ItemUpdate> = params
            .get("update")
            .filter(|value| !value.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| FluxError::invalid(format!("invalid update: {error}")))?;
        let item = self.state.configure_item(&name, &patch, update)?;
        Ok(serde_json::to_value(item)?)
    }

    fn item_subscribe(&self, params: Value) -> Result<Value> {
        let name = required_str(&params, "name")?;
        let events: Vec<String> = params
            .get("events")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| FluxError::invalid(format!("invalid events: {error}")))?
            .unwrap_or_default();
        let add = params.get("add").and_then(Value::as_bool).unwrap_or(true);
        let item = self.state.subscribe_item(&name, &events, add)?;
        Ok(json!({
            "name": item.name,
            "subscribe": item.config.subscribe,
        }))
    }

    fn bar_configure(&self, params: Value) -> Result<Value> {
        let patch = params.get("config").cloned().unwrap_or_else(|| json!({}));
        let bar = self.state.configure_bar(&patch)?;
        if let Some(hidden) = patch.get("hidden").and_then(Value::as_bool) {
            self.bar_visible.store(!hidden, Ordering::Relaxed);
            self.renderer.set_visible(!hidden)?;
        }
        Ok(serde_json::to_value(bar)?)
    }

    fn config_reload(&self) -> Result<Value> {
        let reload = self
            .reload
            .as_ref()
            .ok_or_else(|| FluxError::Config("configuration reloading is not available".into()))?;
        let report = reload.reload()?;
        let payload = serde_json::to_value(&report)?;
        self.state
            .emit(Event::custom("config_reload", payload.clone()));
        let mut response = payload;
        response["reloaded"] = json!(true);
        Ok(response)
    }
}

fn required_str(params: &Value, key: &str) -> Result<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| FluxError::invalid(format!("missing `{key}`")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxbar_core::config::{Config, ItemConfig};
    use fluxbar_core::geometry::Size;
    use fluxbar_core::renderer::RenderFrame;

    struct TestRenderer;

    impl Renderer for TestRenderer {
        fn render(&self, _frame: &RenderFrame) -> Result<()> {
            Ok(())
        }

        fn set_visible(&self, _visible: bool) -> Result<()> {
            Ok(())
        }

        fn bar_height(&self) -> f64 {
            30.0
        }

        fn screen_size(&self) -> Size {
            Size::new(1440.0, 900.0)
        }
    }

    fn handler() -> DaemonHandler {
        let mut config = Config::default();
        config.items.insert(
            "configured".into(),
            ItemConfig {
                kind: ItemKind::Text,
                position: Position::Left,
                label: "static".into(),
                ..Default::default()
            },
        );
        let state = StateHandle::new(config);
        let plugins = Arc::new(PluginHost::new(state.clone()));
        DaemonHandler {
            state,
            plugins,
            renderer: Arc::new(TestRenderer),
            started: Instant::now(),
            frames: Arc::new(AtomicU64::new(0)),
            requests: Arc::new(AtomicU64::new(0)),
            errors: Arc::new(AtomicU64::new(0)),
            shutdown: Arc::new(AtomicBool::new(false)),
            config_path: None,
            bar_visible: Arc::new(AtomicBool::new(true)),
            reload: None,
        }
    }

    #[tokio::test]
    async fn item_lifecycle_over_rpc() {
        let handler = handler();

        let added = handler
            .handle(
                "item.add",
                json!({"name": "dynamic", "kind": "cpu", "position": "right", "label": "0%"}),
            )
            .await
            .unwrap();
        assert_eq!(added["position"], "right");

        let updated = handler
            .handle(
                "item.set",
                json!({"name": "dynamic", "label": "42%", "progress": 42.0}),
            )
            .await
            .unwrap();
        assert_eq!(updated["state"]["label"], "42%");
        assert_eq!(updated["state"]["progress"], 42.0);

        let fetched = handler
            .handle("item.get", json!({"name": "dynamic"}))
            .await
            .unwrap();
        assert_eq!(fetched["state"]["label"], "42%");

        let list = handler.handle("item.list", json!({})).await.unwrap();
        let names: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["name"].as_str())
            .collect();
        assert!(names.contains(&"configured"));
        assert!(names.contains(&"dynamic"));

        handler
            .handle("item.remove", json!({"name": "dynamic"}))
            .await
            .unwrap();
        let missing = handler
            .handle("item.get", json!({"name": "dynamic"}))
            .await
            .unwrap_err();
        assert_eq!(missing.rpc_code(), -32004);
    }

    #[tokio::test]
    async fn rejects_invalid_requests() {
        let handler = handler();

        let unknown = handler
            .handle("does.not.exist", json!({}))
            .await
            .unwrap_err();
        assert_eq!(unknown.rpc_code(), -32601);

        let missing_name = handler.handle("item.add", json!({})).await.unwrap_err();
        assert_eq!(missing_name.rpc_code(), -32602);

        let invalid_position = handler
            .handle("item.add", json!({"name": "bad", "position": "diagonal"}))
            .await
            .unwrap_err();
        assert_eq!(invalid_position.rpc_code(), -32602);
    }

    #[tokio::test]
    async fn shutdown_sets_flag() {
        let handler = handler();
        let response = handler.handle("system.shutdown", json!({})).await.unwrap();
        assert_eq!(response["ok"], true);
        assert!(handler.shutdown.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn trigger_validates_event_names() {
        let handler = handler();
        let ok = handler
            .handle(
                "event.trigger",
                json!({"name": "build_finished", "payload": {"job": 3}}),
            )
            .await
            .unwrap();
        assert_eq!(ok["triggered"], "build_finished");

        let invalid = handler
            .handle("event.trigger", json!({"name": "not valid!"}))
            .await
            .unwrap_err();
        assert_eq!(invalid.rpc_code(), -32602);
    }
}
