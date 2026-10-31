//! Execution of item scripts and SketchyBar-compatible event subscriptions.

use fluxbar_core::config::{Config, ItemConfig};
use fluxbar_core::event::Event;
use fluxbar_core::script::{
    ScriptCommand, ScriptContext, ScriptInvocation, run_script, sketchybar_sender,
};
use fluxbar_core::state::StateHandle;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

/// Periodically runs `[items.*].script` and dispatches subscribed events.
pub struct ScriptRunner {
    state: StateHandle,
    context: ScriptContext,
    last_run: HashMap<String, Instant>,
}

impl ScriptRunner {
    pub fn new(state: StateHandle, context: ScriptContext) -> Self {
        Self {
            state,
            context,
            last_run: HashMap::new(),
        }
    }

    /// Spawn the runner. The returned task is aborted on daemon shutdown.
    pub fn spawn(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut bus = self.state.bus().subscribe();
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            // Initial pass mirrors `sketchybar --update`: initialize every item
            // script once and let it paint the first value.
            self.run_all("routine", json!({})).await;

            loop {
                tokio::select! {
                    received = bus.recv() => {
                        match received {
                            Ok(event) => self.handle_event(&event).await,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                self.state.bus().record_lag(skipped);
                                warn!(skipped, "item script subscriber lagged");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    _ = ticker.tick() => {
                        self.run_due_scripts().await;
                    }
                }
            }
        })
    }

    async fn handle_event(&mut self, event: &Event) {
        if event.name() == "routine" {
            self.run_all("routine", event.payload.clone()).await;
            return;
        }

        let items = self.state.items();
        let mut tasks = JoinSet::new();
        for item in items {
            let config = item.config;
            if !event_targets_item(event, &item.name) {
                continue;
            }

            if event.name() == "mouse.clicked"
                && let Some(script) = config.click_script.clone()
            {
                spawn_item_script(
                    &mut tasks,
                    &self.state,
                    &self.context,
                    &item.name,
                    event,
                    script,
                );
            }

            let Some(script) = config.script.clone() else {
                continue;
            };
            if !subscribed(&config, event.name()) {
                continue;
            }
            spawn_item_script(
                &mut tasks,
                &self.state,
                &self.context,
                &item.name,
                event,
                script,
            );
        }
        while let Some(result) = tasks.join_next().await {
            if let Ok(Err(error)) = result {
                warn!(%error, "item script failed");
            }
        }
    }

    async fn run_all(&mut self, sender: &str, payload: Value) {
        let now = Instant::now();
        let items = self.state.items();
        let mut tasks = JoinSet::new();
        for item in items {
            let Some(script) = item.config.script.clone() else {
                continue;
            };
            self.last_run.insert(item.name.clone(), now);
            let event = Event::custom(sender, payload.clone());
            spawn_item_script(
                &mut tasks,
                &self.state,
                &self.context,
                &item.name,
                &event,
                script,
            );
        }
        while let Some(result) = tasks.join_next().await {
            if let Ok(Err(error)) = result {
                warn!(%error, "item script failed");
            }
        }
    }

    async fn run_due_scripts(&mut self) {
        let now = Instant::now();
        let items = self.state.items();
        let mut tasks = JoinSet::new();
        for item in items {
            let Some(interval) = item.config.update_interval.filter(|seconds| *seconds > 0) else {
                continue;
            };
            let due = self
                .last_run
                .get(&item.name)
                .is_none_or(|last| now.duration_since(*last) >= Duration::from_secs(interval));
            if !due {
                continue;
            }
            let Some(script) = item.config.script.clone() else {
                continue;
            };
            self.last_run.insert(item.name.clone(), now);
            let event = Event::custom("routine", json!({}));
            spawn_item_script(
                &mut tasks,
                &self.state,
                &self.context,
                &item.name,
                &event,
                script,
            );
        }
        while let Some(result) = tasks.join_next().await {
            if let Ok(Err(error)) = result {
                warn!(%error, "item script failed");
            }
        }
    }
}

fn subscribed(config: &ItemConfig, event: &str) -> bool {
    fluxbar_core::script::subscription_matches(&config.subscribe, event)
}

/// Mouse events carry the item that produced them. Do not invoke every item's
/// click handler for a single click.
fn event_targets_item(event: &Event, item: &str) -> bool {
    if !event.name().starts_with("mouse.") {
        return true;
    }
    event
        .payload
        .get("item")
        .and_then(Value::as_str)
        .is_none_or(|target| target == item)
}

fn spawn_item_script(
    tasks: &mut JoinSet<fluxbar_core::Result<()>>,
    state: &StateHandle,
    context: &ScriptContext,
    name: &str,
    event: &Event,
    script: String,
) {
    let state = state.clone();
    let context = context.clone();
    let name = name.to_string();
    let event = event.clone();
    tasks.spawn(async move { run_item_script(&state, &context, &name, &event, script).await });
}

async fn run_item_script(
    state: &StateHandle,
    context: &ScriptContext,
    name: &str,
    event: &Event,
    script: String,
) -> fluxbar_core::Result<()> {
    let mut invocation =
        ScriptInvocation::new(name.to_string(), event.name(), ScriptCommand::Shell(script))
            .with_payload(event.payload.clone());
    invocation.environment = event.environment.clone();
    invocation
        .environment
        .insert("NAME".into(), name.to_string());
    invocation
        .environment
        .insert("SENDER".into(), sketchybar_sender(event.name()));

    if event.name() == "mouse.clicked" {
        if let Some(button) = event.payload.get("button").and_then(Value::as_i64) {
            invocation.environment.insert(
                "BUTTON".into(),
                match button {
                    0 => "left",
                    1 => "right",
                    _ => "other",
                }
                .to_string(),
            );
        }
        if let Some(modifier) = event.payload.get("modifier").and_then(Value::as_str) {
            invocation
                .environment
                .insert("MODIFIER".into(), modifier.to_string());
        }
    }
    if event.name() == "mouse.scrolled"
        && let Some(delta) = event.payload.get("delta").and_then(Value::as_f64)
    {
        invocation
            .environment
            .insert("SCROLL_DELTA".into(), delta.to_string());
    }

    let output = run_script(context, &invocation).await?;
    if let Some(log) = &output.result.log {
        info!(item = %name, "{log}");
    }
    if !output.result.updates.is_empty() {
        state.apply_plugin_updates(&output.result.updates)?;
    }
    for emitted in &output.result.events {
        state.emit(
            Event::custom(emitted.name.clone(), emitted.payload.clone())
                .with_source(format!("item:{name}")),
        );
    }
    if !output.stderr.is_empty() {
        debug!(item = %name, stderr = %output.stderr, "item script stderr");
    }
    Ok(())
}

/// Build the context used by the daemon. The config directory is taken from
/// the loaded file so `--config` and `FLUXBAR_CONFIG_DIR` both work.
pub fn context_for(
    config_path: Option<&std::path::Path>,
    socket: &std::path::Path,
    compat_dir: Option<std::path::PathBuf>,
) -> ScriptContext {
    let config_dir = config_path
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .or_else(Config::default_dir);
    ScriptContext {
        config_dir,
        socket: Some(socket.to_path_buf()),
        compat_dir,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_events_are_targeted_at_one_item() {
        let event = Event::custom("mouse.clicked", json!({"item": "workspace.1", "button": 0}));
        assert!(event_targets_item(&event, "workspace.1"));
        assert!(!event_targets_item(&event, "clock"));
    }

    #[test]
    fn untargeted_mouse_events_fall_back_to_all_items() {
        let event = Event::custom("mouse.clicked", json!({"button": 0}));
        assert!(event_targets_item(&event, "clock"));
    }

    #[test]
    fn non_mouse_events_are_not_filtered() {
        let event = Event::custom("tick", json!({"item": "other"}));
        assert!(event_targets_item(&event, "clock"));
    }
}
