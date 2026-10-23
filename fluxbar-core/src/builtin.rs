//! Built-in item bindings: map system events to item updates.

use crate::event::Event;
use crate::item::ItemKind;
use crate::state::StateHandle;
use serde_json::Value;

/// Apply an event to every item whose `kind` matches the event.
pub fn apply_event(state: &StateHandle, event: &Event) {
    let payload = &event.payload;

    match event.name() {
        "cpu_change" => {
            let percent = payload
                .get("percent")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            update_kind(
                state,
                ItemKind::Cpu,
                |label| {
                    *label = format!("{percent:.0}%");
                },
                Some(percent),
                Some(percent),
            );
        }
        "memory_change" => {
            let percent = payload
                .get("percent")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let used = payload.get("used").and_then(Value::as_u64).unwrap_or(0);
            let total = payload.get("total").and_then(Value::as_u64).unwrap_or(0);
            let used_gb = used as f64 / 1024.0 / 1024.0 / 1024.0;
            let total_gb = total as f64 / 1024.0 / 1024.0 / 1024.0;
            let label = if total > 0 {
                format!("{used_gb:.1}/{total_gb:.0}G")
            } else {
                format!("{percent:.0}%")
            };
            update_kind(
                state,
                ItemKind::Memory,
                |existing| {
                    *existing = label.clone();
                },
                Some(percent),
                Some(percent),
            );
        }
        "volume_change" => {
            let volume = payload.get("volume").and_then(Value::as_u64).unwrap_or(0);
            update_kind(
                state,
                ItemKind::Volume,
                |label| {
                    *label = format!("{volume}%");
                },
                Some(volume as f64),
                Some(volume as f64),
            );
        }
        "power_state" => {
            let percent = payload.get("percent").and_then(Value::as_u64).unwrap_or(0);
            let charging = payload
                .get("charging")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            update_kind(
                state,
                ItemKind::Battery,
                |label| {
                    *label = format!("{percent}%");
                },
                Some(percent as f64),
                Some(percent as f64),
            );
            let _ = charging;
        }
        "media_change" => {
            let title = payload.get("title").and_then(Value::as_str).unwrap_or("");
            let artist = payload.get("artist").and_then(Value::as_str).unwrap_or("");
            let label = if artist.is_empty() {
                title.to_string()
            } else {
                format!("{artist} — {title}")
            };
            update_kind(
                state,
                ItemKind::Media,
                |existing| {
                    *existing = label.clone();
                },
                None,
                None,
            );
        }
        "app_focus" => {
            let app = payload.get("app").and_then(Value::as_str);
            if let Some(app) = app {
                state.write().current_app = Some(app.to_string());
            }
        }
        "space_change" => {
            let space = payload.get("space").and_then(Value::as_str);
            if let Some(space) = space {
                state.write().current_space = Some(space.to_string());
            }
        }
        _ => {}
    }
}

fn update_kind<F>(
    state: &StateHandle,
    kind: ItemKind,
    mut update_label: F,
    progress: Option<f64>,
    graph_sample: Option<f64>,
) where
    F: FnMut(&mut String),
{
    let mut guard = state.write();
    let mut changed = false;
    for item in guard.items.values_mut() {
        if item.kind != kind {
            continue;
        }
        update_label(&mut item.state.label);
        if let Some(progress) = progress {
            item.state.progress = Some(progress);
        }
        if let Some(sample) = graph_sample {
            item.push_graph_sample(sample, 60);
        }
        changed = true;
    }
    if changed {
        guard.next_version();
    }
}

/// Refresh clock-style items (`time` and custom date items).
pub fn refresh_clock(state: &StateHandle) {
    let now = chrono::Local::now();
    let time_label = now.format("%H:%M").to_string();

    let mut guard = state.write();
    let mut changed = false;
    for item in guard.items.values_mut() {
        let label = match item.kind {
            ItemKind::Time => time_label.clone(),
            // Custom items may format the clock via `properties.format`.
            ItemKind::Custom => {
                match item.config.properties.get("format").and_then(Value::as_str) {
                    Some(format) => now.format(format).to_string(),
                    None => continue,
                }
            }
            _ => continue,
        };
        if item.state.label != label {
            item.state.label = label;
            changed = true;
        }
    }
    if changed {
        guard.next_version();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::geometry::Position;
    use serde_json::json;

    fn setup() -> StateHandle {
        let mut config = Config::default();
        config.items.insert(
            "cpu".into(),
            crate::config::ItemConfig {
                kind: ItemKind::Cpu,
                position: Position::Left,
                ..crate::config::ItemConfig::default()
            },
        );
        let state = StateHandle::new(config);
        state
            .add_item("cpu2", ItemKind::Cpu, Position::Left)
            .unwrap();
        state
            .add_item("disk", ItemKind::Text, Position::Right)
            .unwrap();
        state
    }

    #[test]
    fn cpu_event_updates_only_cpu_items() {
        let state = setup();
        apply_event(
            &state,
            &Event::new(crate::event::EventKind::CpuChange, json!({"percent": 42.0})),
        );
        assert_eq!(state.item("cpu").unwrap().state.label, "42%");
        assert_eq!(state.item("cpu2").unwrap().state.label, "42%");
        assert_eq!(state.item("disk").unwrap().state.label, "");
    }

    #[test]
    fn volume_event_formats_label() {
        let state = StateHandle::new(Config::default());
        state
            .add_item("vol", ItemKind::Volume, Position::Right)
            .unwrap();
        apply_event(
            &state,
            &Event::new(crate::event::EventKind::VolumeChange, json!({"volume": 30})),
        );
        assert_eq!(state.item("vol").unwrap().state.label, "30%");
    }

    #[test]
    fn media_event_formats_label() {
        let state = StateHandle::new(Config::default());
        state
            .add_item("media", ItemKind::Media, Position::Center)
            .unwrap();
        apply_event(
            &state,
            &Event::new(
                crate::event::EventKind::MediaChange,
                json!({"artist": "A", "title": "T"}),
            ),
        );
        assert_eq!(state.item("media").unwrap().state.label, "A — T");
    }

    #[test]
    fn clock_updates_time_items() {
        let state = StateHandle::new(Config::default());
        state
            .add_item("clock", ItemKind::Time, Position::Right)
            .unwrap();
        refresh_clock(&state);
        let label = state.item("clock").unwrap().state.label;
        assert!(!label.is_empty());
    }
}
