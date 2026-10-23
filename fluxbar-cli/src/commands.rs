//! CLI command implementations.

use crate::client;
use crate::default_config;
use crate::plugin_manager;
use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use fluxbar_core::config::Config;
use serde_json::{Value, json};
use std::path::Path;
use std::path::PathBuf;

/// Print the status of the running daemon.
pub fn status(socket: &Path) -> Result<()> {
    let value = client::call(socket, "system.status", json!({}))?;
    println!("FluxBar {}", value["version"].as_str().unwrap_or("unknown"));
    println!("socket:   {}", socket.display());
    if let Some(config) = value["config"].as_str() {
        println!("config:   {config}");
    }
    println!("uptime:   {}s", value["uptime_secs"].as_u64().unwrap_or(0));
    println!("items:    {}", value["items"].as_u64().unwrap_or(0));
    println!("plugins:  {}", value["plugins"].as_u64().unwrap_or(0));
    println!("frames:   {}", value["frames"].as_u64().unwrap_or(0));
    Ok(())
}

pub fn stop(socket: &Path) -> Result<()> {
    match client::call(socket, "system.shutdown", json!({})) {
        Ok(_) => {
            println!("FluxBar stopped");
            Ok(())
        }
        Err(error) => bail!("{error}"),
    }
}

pub fn add(socket: &Path, args: crate::AddArgs) -> Result<()> {
    let value = client::call(
        socket,
        "item.add",
        json!({
            "name": args.name,
            "kind": args.kind,
            "position": args.position,
            "label": args.label,
            "icon": args.icon,
        }),
    )?;
    println!(
        "added item `{}` at {}",
        value["name"].as_str().unwrap_or(&args.name),
        value["position"].as_str().unwrap_or("left")
    );
    Ok(())
}

pub fn set(socket: &Path, args: crate::SetArgs) -> Result<()> {
    let update = fluxbar_core::item::ItemUpdate::from_assignments(&args.assignments)
        .with_context(|| "invalid assignment")?;
    let value = client::call(
        socket,
        "item.set",
        json!({ "name": args.name, "update": update }),
    )?;
    println!(
        "updated item `{}`: {}",
        value["name"].as_str().unwrap_or(&args.name),
        serde_json::to_string(&value["state"]).unwrap_or_default()
    );
    Ok(())
}

pub fn remove(socket: &Path, name: &str) -> Result<()> {
    client::call(socket, "item.remove", json!({ "name": name }))?;
    println!("removed item `{name}`");
    Ok(())
}

pub fn list(socket: &Path, plugins: bool) -> Result<()> {
    if plugins {
        let values = client::call(socket, "plugin.list", json!({}))?;
        let plugins = values.as_array().cloned().unwrap_or_default();
        if plugins.is_empty() {
            println!("no plugins");
            return Ok(());
        }
        println!("{:<20} {:<8} {:<8} PID", "NAME", "RUNNING", "RESTARTS");
        for plugin in plugins {
            println!(
                "{:<20} {:<8} {:<8} {}",
                plugin["name"].as_str().unwrap_or("?"),
                plugin["running"].as_bool().unwrap_or(false),
                plugin["restarts"].as_u64().unwrap_or(0),
                plugin["pid"]
                    .as_u64()
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "-".into()),
            );
        }
        return Ok(());
    }

    let values = client::call(socket, "item.list", json!({}))?;
    let items = values.as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        println!("no items");
        return Ok(());
    }
    println!(
        "{:<16} {:<10} {:<8} {:<10} LABEL",
        "NAME", "KIND", "POSITION", "VISIBLE"
    );
    for item in items {
        println!(
            "{:<16} {:<10} {:<8} {:<10} {}",
            item["name"].as_str().unwrap_or("?"),
            item["kind"].as_str().unwrap_or("?"),
            item["position"].as_str().unwrap_or("?"),
            item["visible"].as_bool().unwrap_or(true),
            item["label"].as_str().unwrap_or(""),
        );
    }
    Ok(())
}

pub fn trigger(socket: &Path, name: &str, payload: &str) -> Result<()> {
    fluxbar_core::event::validate_name(name)?;
    let payload: Value =
        serde_json::from_str(payload).with_context(|| "payload is not valid JSON")?;
    client::call(
        socket,
        "event.trigger",
        json!({ "name": name, "payload": payload }),
    )?;
    println!("triggered `{name}`");
    Ok(())
}

pub fn config(config_path: Option<PathBuf>, command: crate::ConfigCommand) -> Result<()> {
    let path = config_path.clone().or_else(Config::default_path);
    match command {
        crate::ConfigCommand::Path => {
            match path {
                Some(path) => println!("{}", path.display()),
                None => bail!("could not determine the configuration directory"),
            }
            Ok(())
        }
        crate::ConfigCommand::Init { force } => {
            let Some(path) = path else {
                bail!("could not determine the configuration directory");
            };
            if path.exists() && !force {
                bail!(
                    "{} already exists (use --force to overwrite)",
                    path.display()
                );
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, default_config::DEFAULT_CONFIG)?;
            println!("wrote {}", path.display());
            Ok(())
        }
        crate::ConfigCommand::Validate => {
            let Some(path) = path else {
                bail!("could not determine the configuration directory");
            };
            let config = Config::load(&path)?;
            println!(
                "configuration is valid: {} items, {} plugins, {} timers",
                config.items.len(),
                config.plugins.len(),
                config.events.timers.len()
            );
            Ok(())
        }
        crate::ConfigCommand::Show => {
            let config = match path {
                Some(path) if path.exists() => Config::load(&path)?,
                _ => Config::default(),
            };
            print!("{}", config.to_toml()?);
            Ok(())
        }
    }
}

pub fn plugin(
    socket: &Path,
    config_path: Option<PathBuf>,
    command: crate::PluginCommand,
) -> Result<()> {
    if let Some(directory) = config_path.as_ref().and_then(|path| path.parent()) {
        plugin_manager::set_config_dir(directory.to_path_buf());
    }
    match command {
        crate::PluginCommand::List => list(socket, true),
        crate::PluginCommand::Install(args) => {
            let options = plugin_manager::InstallOptions {
                source: args.source,
                name: args.name,
                build: !args.no_build,
            };
            let installed = plugin_manager::install(options)?;
            println!("installed plugin `{}`", installed.name);
            println!("config: {}", installed.config_path.display());
            if let Some(command) = &installed.command {
                println!("command: {command}");
            }
            if client::call(socket, "config.reload", json!({})).is_ok() {
                println!("loaded into the running daemon");
            } else {
                println!("Start FluxBar or run `fluxbar trigger config_reload` to load it.");
            }
            Ok(())
        }
        crate::PluginCommand::Remove { name } => {
            plugin_manager::remove(&name)?;
            println!("removed plugin `{name}`");
            Ok(())
        }
        crate::PluginCommand::Restart { name } => {
            client::call(socket, "plugin.restart", json!({ "name": name }))?;
            println!("restarted plugin `{name}`");
            Ok(())
        }
        crate::PluginCommand::Update { name } => {
            plugin_manager::update(&name)?;
            println!("updated plugin `{name}`");
            Ok(())
        }
    }
}

pub fn paths(config_path: Option<PathBuf>, socket: &Path) -> Result<()> {
    println!(
        "config:     {}",
        config_path
            .or_else(Config::default_path)
            .map_or_else(|| "<none>".into(), |p| p.display().to_string())
    );
    println!(
        "config dir: {}",
        Config::default_dir().map_or_else(|| "<none>".into(), |p| p.display().to_string())
    );
    println!("socket:     {}", socket.display());
    println!("plugins:    {}", plugin_manager::plugins_dir()?.display());
    Ok(())
}

/// Marker to keep clap's ValueEnum import meaningful for future enums.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}
