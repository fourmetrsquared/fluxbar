//! SketchyBar-compatible command translation.
//!
//! Interoperability note: the command names and event vocabulary follow
//! SketchyBar by Felix Kratz (https://github.com/FelixKratz/SketchyBar). This
//! is an independent implementation; no SketchyBar source code is copied.
//!
//! The real SketchyBar configuration and plugin ecosystem is a shell API:
//! scripts call `sketchybar --set NAME label=...`, subscribe items to events,
//! and use `$NAME`/`$SENDER`/`$INFO`. This module translates the commonly used
//! command grammar into FluxBar JSON-RPC calls. Unknown style properties are
//! ignored with a warning so existing plugins keep working when they use
//! properties FluxBar does not render yet.

use crate::client;
use anyhow::{Context, Result, bail};
use fluxbar_core::config::Config;
use fluxbar_core::geometry::Position;
use fluxbar_core::item::ItemUpdate;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A parsed `--command args...` group.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    command: String,
    args: Vec<String>,
}

/// Run the compatibility CLI.
pub fn run(socket: &Path, args: &[String]) -> Result<()> {
    if args.is_empty() {
        print_help();
        return Ok(());
    }

    let segments = parse_segments(args);
    if segments.is_empty() {
        print_help();
        return Ok(());
    }

    for segment in segments {
        dispatch(socket, &segment)?;
    }
    Ok(())
}

fn dispatch(socket: &Path, segment: &Segment) -> Result<()> {
    match segment.command.as_str() {
        "--help" | "-h" => print_help(),
        "--version" => println!("sketchybar-compatible FluxBar {}", fluxbar_core::VERSION),
        "--bar" => {
            let patch = bar_patch(&assignments(&segment.args))?;
            client::call(socket, "bar.configure", json!({ "config": patch }))?;
        }
        "--default" => {
            let (patch, _) = item_patch(&assignments(&segment.args));
            client::call(socket, "defaults.configure", json!({ "config": patch }))?;
        }
        "--add" => add(socket, &segment.args)?,
        "--set" => set(socket, &segment.args)?,
        "--remove" => {
            for name in &segment.args {
                client::call(socket, "item.remove", json!({ "name": name }))?;
            }
        }
        "--rename" => {
            let [old, new] = segment.args.as_slice() else {
                bail!("--rename expects <old> <new>");
            };
            client::call(socket, "item.rename", json!({ "old": old, "new": new }))?;
        }
        "--move" => {
            let Some(name) = segment.args.first() else {
                bail!("--move expects <name> <position>");
            };
            let position = segment.args.get(1).map(String::as_str).unwrap_or("left");
            client::call(
                socket,
                "item.configure",
                json!({
                    "name": name,
                    "config": { "position": position },
                    "update": { "position": position },
                }),
            )?;
        }
        "--subscribe" => subscribe(socket, segment, true)?,
        "--unsubscribe" => subscribe(socket, segment, false)?,
        "--trigger" => trigger(socket, segment)?,
        "--update" => {
            client::call(socket, "event.trigger", json!({ "name": "routine" }))?;
        }
        "--reload" | "--hotload" => {
            client::call(socket, "config.reload", json!({}))?;
            client::call(socket, "event.trigger", json!({ "name": "routine" }))?;
        }
        "--query" => query(socket, &segment.args)?,
        "--message" => {
            // SketchyBar shows a transient message; a log line keeps scripts
            // working without opening an additional window.
            eprintln!("{}", segment.args.join(" "));
        }
        "--config" => {
            let value = client::call(socket, "config.path", json!({}))?;
            if let Some(path) = value.get("path").and_then(Value::as_str) {
                println!("{path}");
            }
        }
        "--animate" | "--curve" | "--duration" | "--delay" => {
            // Animation commands are accepted for compatibility. FluxBar still
            // renders with its configured animation duration and easing.
        }
        other => {
            eprintln!("sketchybar: ignoring unsupported command `{other}`");
        }
    }
    Ok(())
}

fn add(socket: &Path, args: &[String]) -> Result<()> {
    let Some(kind) = args.first() else {
        bail!("--add expects a component type");
    };
    match kind.as_str() {
        "item" | "space" => {
            let name = args.get(1).context("--add item expects a name")?;
            let position = args.get(2).map(String::as_str).unwrap_or("left");
            let item_kind = if kind == "space" { "custom" } else { "text" };
            client::call(
                socket,
                "item.add",
                json!({
                    "name": name,
                    "kind": item_kind,
                    "position": position,
                }),
            )?;
        }
        "event" => {
            let Some(name) = args.get(1) else {
                bail!("--add event expects a name");
            };
            client::call(socket, "event.add", json!({ "name": name }))?;
        }
        "alias" | "bracket" => {
            eprintln!("sketchybar: `{kind}` components are not supported yet");
        }
        other => eprintln!("sketchybar: ignoring unsupported component `{other}`"),
    }
    Ok(())
}

fn set(socket: &Path, args: &[String]) -> Result<()> {
    let Some(name) = args.first() else {
        bail!("--set expects an item name");
    };
    let (config, update) = item_patch(&assignments(&args[1..]));
    client::call(
        socket,
        "item.configure",
        json!({
            "name": name,
            "config": config,
            "update": update,
        }),
    )?;
    Ok(())
}

fn subscribe(socket: &Path, segment: &Segment, add: bool) -> Result<()> {
    let Some(name) = segment.args.first() else {
        bail!("{} expects an item name", segment.command);
    };
    let events = &segment.args[1..];
    if events.is_empty() {
        bail!("{} expects at least one event", segment.command);
    }
    client::call(
        socket,
        "item.subscribe",
        json!({ "name": name, "events": events, "add": add }),
    )?;
    Ok(())
}

fn trigger(socket: &Path, segment: &Segment) -> Result<()> {
    let Some(name) = segment.args.first() else {
        bail!("--trigger expects an event name");
    };
    let env = assignments(&segment.args[1..]);
    let environment: BTreeMap<_, _> = env.into_iter().collect();
    client::call(
        socket,
        "event.trigger",
        json!({ "name": name, "payload": {}, "environment": environment }),
    )?;
    Ok(())
}

fn query(socket: &Path, args: &[String]) -> Result<()> {
    if args.is_empty() || args[0] == "bar" {
        let status = client::call(socket, "system.status", json!({}))?;
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }
    let value = client::call(socket, "item.get", json!({ "name": args[0] }))?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Split argv into command segments. Every token starting with `--` begins a
/// new command; everything else belongs to the current command.
fn parse_segments(args: &[String]) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for token in args {
        if token.starts_with("--") {
            segments.push(Segment {
                command: token.clone(),
                args: Vec::new(),
            });
        } else if let Some(segment) = segments.last_mut() {
            segment.args.push(token.clone());
        }
    }
    segments
}

fn assignments(args: &[String]) -> Vec<(String, String)> {
    args.iter()
        .filter_map(|argument| {
            argument
                .split_once('=')
                .map(|(key, value)| (key.to_string(), value.to_string()))
        })
        .collect()
}

fn item_patch(assignments: &[(String, String)]) -> (Value, ItemUpdate) {
    let mut config = Map::new();
    let mut update = ItemUpdate::default();

    for (key, value) in assignments {
        match key.as_str() {
            "label" => update.label = Some(value.clone()),
            "icon" => update.icon = Some(value.clone()),
            "progress" => {
                if let Ok(progress) = value.parse() {
                    update.progress = Some(progress);
                }
            }
            "progress.color" => set_string(&mut config, "progress_color", value),
            "progress.track_color" => set_string(&mut config, "progress_track_color", value),
            "progress.height" => set_number(&mut config, "progress_height", value),
            "progress.position" => set_string(&mut config, "progress_position", value),
            "drawing" | "visible" => {
                let visible = parse_bool(value);
                update.visible = Some(visible);
                config.insert("visible".into(), json!(visible));
            }
            "position" => {
                if let Ok(position) = value.parse::<Position>() {
                    update.position = Some(position);
                    config.insert("position".into(), json!(position.as_str()));
                }
            }
            "label.drawing" => set_bool(&mut config, "label_drawing", value),
            "icon.drawing" => set_bool(&mut config, "icon_drawing", value),
            "background.drawing" => set_bool(&mut config, "background_drawing", value),
            "background.color" => set_string(&mut config, "background", value),
            "background.border_color" => set_string(&mut config, "border_color", value),
            "background.border_width" => set_number(&mut config, "border_width", value),
            "background.corner_radius" => {
                set_number(&mut config, "background_corner_radius", value)
            }
            "background.shape" => set_string(&mut config, "background_shape", value),
            "background.blur_radius" => set_number(&mut config, "background_blur_radius", value),
            "opacity" => set_number(&mut config, "opacity", value),
            "background.height" => set_number(&mut config, "background_height", value),
            "background.padding_left" => {
                set_inset(&mut config, "background_padding", "left", value)
            }
            "background.padding_right" => {
                set_inset(&mut config, "background_padding", "right", value)
            }
            "label.color" => set_string(&mut config, "label_color", value),
            "icon.color" => set_string(&mut config, "icon_color", value),
            "label.font" => {
                let (family, size) = parse_font(value);
                config.insert("label_font".into(), json!(family));
                if let Some(size) = size {
                    config.insert("label_font_size".into(), json!(size));
                }
            }
            "icon.font" => {
                let (family, size) = parse_font(value);
                config.insert("icon_font".into(), json!(family));
                if let Some(size) = size {
                    config.insert("icon_font_size".into(), json!(size));
                }
            }
            "label.font_size" => set_number(&mut config, "label_font_size", value),
            "icon.font_size" => set_number(&mut config, "icon_font_size", value),
            "label.padding_left" => set_inset(&mut config, "label_padding", "left", value),
            "label.padding_right" => set_inset(&mut config, "label_padding", "right", value),
            "icon.padding_left" => set_inset(&mut config, "icon_padding", "left", value),
            "icon.padding_right" => set_inset(&mut config, "icon_padding", "right", value),
            "padding_left" => set_inset(&mut config, "padding", "left", value),
            "padding_right" => set_inset(&mut config, "padding", "right", value),
            "width" => set_optional_number(&mut config, "width", value),
            "min_width" => set_number(&mut config, "min_width", value),
            "max_width" => set_number(&mut config, "max_width", value),
            "drawing_order" => set_integer(&mut config, "drawing_order", value),
            "script" => set_command(&mut config, "script", value),
            "click_script" => set_command(&mut config, "click_script", value),
            "update_freq" => {
                if let Ok(seconds) = value.parse::<u64>() {
                    config.insert("update_interval".into(), json!(seconds));
                }
            }
            // Accepted SketchyBar properties that FluxBar does not render yet.
            "associated_display"
            | "display"
            | "updates"
            | "mach_helper"
            | "scroll_texts"
            | "label.max_chars"
            | "label.align"
            | "label.y_offset"
            | "icon.y_offset"
            | "background.y_offset"
            | "background.x_offset"
            | "background.image"
            | "background.clip"
            | "label.highlight"
            | "icon.highlight"
            | "image"
            | "shadow.drawing" => {}
            other if other.starts_with("popup.") => {}
            other if other.starts_with("shadow.") => {}
            other if other.starts_with("label.background.") => {}
            other if other.starts_with("label.shadow.") => {}
            other if other.starts_with("icon.background.") => {}
            other if other.starts_with("icon.shadow.") => {}
            other => eprintln!("sketchybar: ignoring unsupported item property `{other}`"),
        }
    }

    (Value::Object(config), update)
}

fn bar_patch(assignments: &[(String, String)]) -> Result<Value> {
    let mut config = Map::new();
    for (key, value) in assignments {
        match key.as_str() {
            "height" => set_number(&mut config, "height", value),
            "position" => set_string(&mut config, "position", value),
            "color" | "background" => set_string(&mut config, "background", value),
            "opacity" => set_number(&mut config, "opacity", value),
            "blur_radius" => set_number(&mut config, "blur_radius", value),
            "corner_radius" => set_number(&mut config, "corner_radius", value),
            "padding_left" => set_inset(&mut config, "padding", "left", value),
            "padding_right" => set_inset(&mut config, "padding", "right", value),
            "y_offset" => set_number(&mut config, "top_margin", value),
            "margin" => set_number(&mut config, "horizontal_margin", value),
            "hidden" => set_bool(&mut config, "hidden", value),
            "font" => set_string(&mut config, "font", value),
            "font_size" => set_number(&mut config, "font_size", value),
            "icon_font" => set_string(&mut config, "icon_font", value),
            "icon_font_size" => set_number(&mut config, "icon_font_size", value),
            "foreground" => set_string(&mut config, "foreground", value),
            "display" => {
                if let Ok(display) = value.parse::<usize>() {
                    config.insert("display".into(), json!(display));
                }
            }
            // Native window-level properties without a FluxBar equivalent.
            "topmost" | "sticky" | "shadow" | "notch_width" | "notch_offset" | "text_font"
            | "text_font_size" => {}
            other => eprintln!("sketchybar: ignoring unsupported bar property `{other}`"),
        }
    }
    // Validate through the real config type before sending the patch.
    let mut current = serde_json::to_value(Config::default().bar)?;
    merge(&mut current, Value::Object(config.clone()));
    serde_json::from_value::<fluxbar_core::BarConfig>(current).context("invalid bar properties")?;
    Ok(Value::Object(config))
}

fn set_bool(config: &mut Map<String, Value>, key: &str, value: &str) {
    config.insert(key.into(), json!(parse_bool(value)));
}

fn set_string(config: &mut Map<String, Value>, key: &str, value: &str) {
    config.insert(key.into(), json!(value));
}

fn set_number(config: &mut Map<String, Value>, key: &str, value: &str) {
    if let Ok(number) = value.parse::<f64>() {
        config.insert(key.into(), json!(number));
    }
}

fn set_integer(config: &mut Map<String, Value>, key: &str, value: &str) {
    if let Ok(number) = value.parse::<i32>() {
        config.insert(key.into(), json!(number));
    }
}

fn set_optional_number(config: &mut Map<String, Value>, key: &str, value: &str) {
    if value == "dynamic" {
        config.insert(key.into(), json!(0.0));
    } else if let Ok(number) = value.parse::<f64>() {
        config.insert(key.into(), json!(number));
    }
}

fn set_command(config: &mut Map<String, Value>, key: &str, value: &str) {
    if value.is_empty() || value == "off" || value == "none" {
        config.insert(key.into(), Value::Null);
    } else {
        config.insert(key.into(), json!(value));
    }
}

fn set_inset(config: &mut Map<String, Value>, key: &str, side: &str, value: &str) {
    if let Ok(number) = value.parse::<f64>() {
        config.insert(key.into(), json!({ side: number }));
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "on" | "true" | "yes" | "1")
}

fn parse_font(value: &str) -> (String, Option<f64>) {
    let mut parts = value.split(':');
    let family = parts.next().unwrap_or(value).to_string();
    let mut size = None;
    for part in parts {
        if let Ok(parsed) = part.parse::<f64>() {
            size = Some(parsed);
        }
    }
    (family, size)
}

fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                if value.is_null() {
                    base.remove(&key);
                } else if let Some(existing) = base.get_mut(&key) {
                    merge(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, patch) => *base = patch,
    }
}

/// Create `bin/sketchybar` and `bin/fluxbar` shims inside the configuration
/// directory. Scripts started by FluxBar get this directory prepended to
/// `PATH`, so existing SketchyBar plugins can call `sketchybar` unchanged.
pub fn ensure_shims(config_dir: &Path) -> Result<PathBuf> {
    let directory = config_dir.join("bin");
    fs::create_dir_all(&directory)?;
    let executable = std::env::current_exe().context("cannot determine executable path")?;
    let quoted = shell_quote(&executable.display().to_string());

    write_shim(
        &directory.join("sketchybar"),
        format!("#!/bin/sh\nexec {quoted} sketchybar \"$@\"\n"),
    )?;
    write_shim(
        &directory.join("fluxbar"),
        format!("#!/bin/sh\nexec {quoted} \"$@\"\n"),
    )?;

    Ok(directory)
}

fn write_shim(path: &Path, contents: String) -> Result<()> {
    // A symlink in the user's config directory must not redirect the shim
    // write to an unrelated file.
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            fs::remove_file(path)?;
        }
    }
    fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn print_help() {
    println!(
        "sketchybar-compatible command interface\n\n\
         Usage: fluxbar sketchybar <command> [arguments...]\n\n\
         Supported commands:\n\
           --bar key=value...          Bar properties\n\
           --default key=value...      Defaults for future items\n\
           --add item|space|event ...  Add a component\n\
           --set NAME key=value...     Update an item\n\
           --remove NAME...            Remove items\n\
           --rename OLD NEW            Rename an item\n\
           --move NAME POSITION        Move an item\n\
           --subscribe NAME EVENT...   Subscribe to events\n\
           --unsubscribe NAME EVENT... Remove subscriptions\n\
           --trigger EVENT [K=V...]    Trigger a custom event\n\
           --update                    Run every item script\n\
           --reload                    Reload the FluxBar configuration\n\
           --query [NAME|bar]          Print JSON state\n\
           --version                   Print the version"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxbar_core::config::ItemConfig;

    fn patched_item(values: &[(String, String)]) -> ItemConfig {
        let (patch, _) = item_patch(values);
        let mut current = serde_json::to_value(ItemConfig::default()).unwrap();
        merge(&mut current, patch);
        serde_json::from_value(current).unwrap()
    }

    #[test]
    fn splits_chained_commands() {
        let args: Vec<String> = [
            "--add",
            "item",
            "clock",
            "right",
            "--set",
            "clock",
            "label=12:00",
            "--subscribe",
            "clock",
            "routine",
            "system_woke",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let segments = parse_segments(&args);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].command, "--add");
        assert_eq!(segments[0].args, ["item", "clock", "right"]);
        assert_eq!(segments[1].args, ["clock", "label=12:00"]);
        assert_eq!(segments[2].args[2], "system_woke");
    }

    #[test]
    fn translates_item_properties() {
        let values = vec![
            ("label".into(), "CPU".into()),
            ("label.color".into(), "0xFFFF0000".into()),
            ("background.drawing".into(), "on".into()),
            ("background.color".into(), "0x44000000".into()),
            ("background.corner_radius".into(), "6".into()),
            ("background.shape".into(), "circle".into()),
            ("background.blur_radius".into(), "18".into()),
            ("opacity".into(), "0.75".into()),
            ("progress.color".into(), "0xFF00FF00".into()),
            ("progress.track_color".into(), "0x3300FF00".into()),
            ("progress.height".into(), "5".into()),
            ("progress.position".into(), "top".into()),
            ("drawing_order".into(), "100".into()),
            ("label.font".into(), "Hack Nerd Font:Bold:17.0".into()),
            ("width".into(), "dynamic".into()),
            ("update_freq".into(), "5".into()),
        ];
        let patched = patched_item(&values);
        assert_eq!(patched.label_color, "0xFFFF0000");
        assert!(patched.background_drawing);
        assert_eq!(patched.background.as_deref(), Some("0x44000000"));
        assert_eq!(patched.background_corner_radius, 6.0);
        assert_eq!(
            patched.background_shape,
            fluxbar_core::config::ItemShape::Circle
        );
        assert_eq!(patched.background_blur_radius, 18.0);
        assert_eq!(patched.opacity, 0.75);
        assert_eq!(patched.progress_color.as_deref(), Some("0xFF00FF00"));
        assert_eq!(patched.progress_track_color.as_deref(), Some("0x3300FF00"));
        assert_eq!(patched.progress_height, Some(5.0));
        assert_eq!(
            patched.progress_position,
            Some(fluxbar_core::config::ProgressPosition::Top)
        );
        assert_eq!(patched.drawing_order, 100);
        assert_eq!(patched.label_font.as_deref(), Some("Hack Nerd Font"));
        assert_eq!(patched.label_font_size, 17.0);
        assert_eq!(patched.width, 0.0);
        assert_eq!(patched.update_interval, Some(5));
    }

    #[test]
    fn parses_boolean_spellings() {
        assert!(parse_bool("on"));
        assert!(parse_bool("true"));
        assert!(!parse_bool("off"));
    }

    #[test]
    fn builds_bar_patch() {
        let values = vec![
            ("height".into(), "40".into()),
            ("color".into(), "0x40000000".into()),
            ("blur_radius".into(), "24".into()),
            ("y_offset".into(), "2".into()),
            ("margin".into(), "12".into()),
        ];
        let patch = bar_patch(&values).unwrap();
        assert_eq!(patch["height"], 40.0);
        assert_eq!(patch["background"], "0x40000000");
        assert_eq!(patch["blur_radius"], 24.0);
        assert_eq!(patch["top_margin"], 2.0);
        assert_eq!(patch["horizontal_margin"], 12.0);
    }

    #[test]
    fn creates_shell_quoted_shims() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin = ensure_shims(dir.path()).unwrap();
        assert!(bin.join("sketchybar").is_file());
        assert!(bin.join("fluxbar").is_file());
        let contents = fs::read_to_string(bin.join("sketchybar")).unwrap();
        assert!(contents.contains("sketchybar \"$@\""));
    }
}
