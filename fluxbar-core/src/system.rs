//! Built-in system event sources.
//!
//! On macOS, values are obtained through cheap shell queries (`pmset`,
//! `osascript`) and `sysinfo`. All polling happens in one task with
//! configurable intervals so the idle cost stays negligible.

use crate::event::{Event, EventKind};
use crate::state::StateHandle;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use sysinfo::{MemoryRefreshKind, System};
use tokio::process::Command;
use tracing::debug;

/// Configuration for the built-in monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemConfig {
    /// CPU / memory sampling interval in seconds.
    #[serde(default = "default_poll")]
    pub poll_interval: u64,
    /// Battery sampling interval in seconds.
    #[serde(default = "default_slow_poll")]
    pub battery_interval: u64,
    /// Volume sampling interval in seconds.
    #[serde(default = "default_slow_poll")]
    pub volume_interval: u64,
    /// Emit `cpu_change` / `memory_change` events.
    #[serde(default = "default_true")]
    pub emit_system_events: bool,
    /// Optional command printing the current space id (e.g. aerospace/yabai).
    #[serde(default)]
    pub space_command: Option<String>,
    /// Interval for `space_command` in seconds.
    #[serde(default = "default_poll")]
    pub space_interval: u64,
    /// Enable media queries through AppleScript.
    #[serde(default)]
    pub media: bool,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            poll_interval: 2,
            battery_interval: 30,
            volume_interval: 5,
            emit_system_events: true,
            space_command: None,
            space_interval: 2,
            media: false,
        }
    }
}

fn default_poll() -> u64 {
    2
}

fn default_slow_poll() -> u64 {
    30
}

fn default_true() -> bool {
    true
}

/// Snapshot of system metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub cpu_percent: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub battery_percent: Option<u8>,
    pub battery_charging: Option<bool>,
    pub volume: Option<u8>,
}

impl SystemSnapshot {
    pub fn memory_percent(&self) -> f64 {
        if self.memory_total == 0 {
            return 0.0;
        }
        (self.memory_used as f64 / self.memory_total as f64) * 100.0
    }
}

/// Long-running monitor task.
pub struct SystemMonitor {
    state: StateHandle,
    config: SystemConfig,
}

impl SystemMonitor {
    pub fn new(state: StateHandle, config: SystemConfig) -> Self {
        Self { state, config }
    }

    /// Spawn the monitor and return its task handle.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    async fn run(self) {
        let mut system = System::new();
        let mut poll = tokio::time::interval(Duration::from_secs(self.config.poll_interval.max(1)));
        let mut battery =
            tokio::time::interval(Duration::from_secs(self.config.battery_interval.max(5)));
        let mut volume =
            tokio::time::interval(Duration::from_secs(self.config.volume_interval.max(1)));
        let mut space =
            tokio::time::interval(Duration::from_secs(self.config.space_interval.max(1)));

        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        battery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        volume.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        space.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut last_space: Option<String> = None;
        let mut last_app: Option<String> = None;

        loop {
            tokio::select! {
                _ = poll.tick() => {
                    system.refresh_cpu_usage();
                    system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
                    let snapshot = SystemSnapshot {
                        cpu_percent: system.global_cpu_usage(),
                        memory_used: system.used_memory(),
                        memory_total: system.total_memory(),
                        ..SystemSnapshot::default()
                    };

                    if self.config.emit_system_events {
                        self.state.emit(Event::new(
                            EventKind::CpuChange,
                            json!({"percent": snapshot.cpu_percent}),
                        ).with_source("system"));
                        self.state.emit(Event::new(
                            EventKind::MemoryChange,
                            json!({
                                "percent": snapshot.memory_percent(),
                                "used": snapshot.memory_used,
                                "total": snapshot.memory_total,
                            }),
                        ).with_source("system"));
                    }

                    // App focus polling (cheap enough at a few seconds).
                    if let Some(app) = frontmost_app().await {
                        if last_app.as_deref() != Some(app.as_str()) {
                            last_app = Some(app.clone());
                            self.state.emit(Event::new(EventKind::AppFocus, json!({"app": app})).with_source("system"));
                        }
                    }
                }
                _ = battery.tick() => {
                    if let Some((percent, charging)) = battery_state().await {
                        self.state.emit(Event::new(
                            EventKind::PowerState,
                            json!({"percent": percent, "charging": charging}),
                        ).with_source("system"));
                    }
                }
                _ = volume.tick() => {
                    if let Some(volume) = output_volume().await {
                        self.state.emit(Event::new(
                            EventKind::VolumeChange,
                            json!({"volume": volume}),
                        ).with_source("system"));
                    }
                    if self.config.media {
                        if let Some(media) = now_playing().await {
                            self.state.emit(Event::new(EventKind::MediaChange, media).with_source("system"));
                        }
                    }
                }
                _ = space.tick() => {
                    if let Some(command) = &self.config.space_command {
                        match run_command(command).await {
                            Ok(output) => {
                                let output = output.trim().to_string();
                                if !output.is_empty() && last_space.as_deref() != Some(output.as_str()) {
                                    last_space = Some(output.clone());
                                    self.state.emit(Event::new(
                                        EventKind::SpaceChange,
                                        json!({"space": output}),
                                    ).with_source("system"));
                                }
                            }
                            Err(error) => debug!(%error, "space command failed"),
                        }
                    }
                }
            }
        }
    }
}

async fn run_command(command: &str) -> Result<String, std::io::Error> {
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .output()
        .await?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "command exited with {}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn run_osascript(script: &str) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .await
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Frontmost application name.
pub async fn frontmost_app() -> Option<String> {
    run_osascript(r#"tell application "System Events" to get name of first application process whose frontmost is true"#).await
}

/// Battery percentage and charging flag.
pub async fn battery_state() -> Option<(u8, bool)> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = Command::new("pmset")
        .args(["-g", "batt"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // Example: `-InternalBattery-0 (id=...)  85%; discharging; 4:12 remaining present: true`
    let percent = text
        .split('%')
        .next()?
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()?
        .parse::<u8>()
        .ok()?;
    let charging = text.contains("charging") && !text.contains("discharging");
    Some((percent, charging))
}

/// Output volume (0-100).
pub async fn output_volume() -> Option<u8> {
    run_osascript("output volume of (get volume settings)")
        .await?
        .parse::<u8>()
        .ok()
}

/// Now-playing information from Music or Spotify.
pub async fn now_playing() -> Option<serde_json::Value> {
    let spotify = r#"if application "Spotify" is running then tell application "Spotify" to return (artist of current track) & "|" & (name of current track)"#;
    if let Some(raw) = run_osascript(spotify).await {
        if let Some((artist, title)) = raw.split_once('|') {
            return Some(json!({
                "player": "Spotify",
                "artist": artist,
                "title": title,
                "playing": true,
            }));
        }
    }

    let music = r#"if application "Music" is running then tell application "Music" to return (artist of current track) & "|" & (name of current track)"#;
    if let Some(raw) = run_osascript(music).await {
        if let Some((artist, title)) = raw.split_once('|') {
            return Some(json!({
                "player": "Music",
                "artist": artist,
                "title": title,
                "playing": true,
            }));
        }
    }

    None
}

/// One-shot sample useful for tests and the `system.info` RPC method.
pub fn sample_system() -> SystemSnapshot {
    let mut system = System::new();
    system.refresh_cpu_usage();
    system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
    SystemSnapshot {
        cpu_percent: system.global_cpu_usage(),
        memory_used: system.used_memory(),
        memory_total: system.total_memory(),
        ..SystemSnapshot::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_percent() {
        let snapshot = SystemSnapshot {
            memory_used: 512,
            memory_total: 1024,
            ..SystemSnapshot::default()
        };
        assert!((snapshot.memory_percent() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn sample_returns_sane_values() {
        let snapshot = sample_system();
        assert!(snapshot.memory_total > 0);
        assert!(snapshot.cpu_percent >= 0.0);
    }
}
