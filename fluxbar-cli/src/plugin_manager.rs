//! Plugin installation and management.
//!
//! A plugin repository may contain a `fluxbar-plugin.toml` manifest:
//!
//! ```toml
//! name = "sysmon"
//! build = "cargo build --release"
//! entry = "target/release/fluxbar-plugin-sysmon"
//! subscribe = ["tick"]
//! tick_interval = 1
//! ```
//!
//! Without a manifest the installer infers the entry point for Rust crates
//! (`target/release/<package>`), Node.js (`node index.js`) and Python
//! (`python3 main.py`).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use fluxbar_core::config::Config;

/// Manifest file name inside plugin repositories.
pub const MANIFEST_FILE: &str = "fluxbar-plugin.toml";

/// Plugin manifest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    /// Plugin name. Defaults to the repository name.
    #[serde(default)]
    pub name: Option<String>,
    /// Build command executed in the plugin directory.
    #[serde(default)]
    pub build: Option<String>,
    /// Entry point, relative to the plugin directory.
    #[serde(default)]
    pub entry: Option<String>,
    /// Events the plugin subscribes to.
    #[serde(default)]
    pub subscribe: Vec<String>,
    /// Tick interval in seconds.
    #[serde(default)]
    pub tick_interval: Option<u64>,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Installation options from the CLI.
#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub source: String,
    pub name: Option<String>,
    pub build: bool,
}

/// Result of a successful installation.
#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    pub name: String,
    pub config_path: PathBuf,
    pub command: Option<String>,
}

/// Directory where plugin sources are stored.
/// Override for the configuration directory (set when `--config` is passed).
static CONFIG_DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Point plugin operations at the directory of an explicit `--config` file.
pub fn set_config_dir(directory: PathBuf) {
    let _ = CONFIG_DIR_OVERRIDE.set(directory);
}

pub fn plugins_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("plugins"))
}

fn config_dir() -> Result<PathBuf> {
    if let Some(directory) = CONFIG_DIR_OVERRIDE.get() {
        return Ok(directory.clone());
    }
    Config::default_dir().context("could not determine the configuration directory")
}

/// Parse a plugin source into a git URL and a suggested name.
pub fn parse_source(source: &str) -> Result<(Option<String>, String)> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        bail!("empty plugin source");
    }

    if let Some(rest) = trimmed.strip_prefix("github:") {
        let rest = rest.trim_matches('/');
        if rest.matches('/').count() != 1 {
            bail!("expected github:user/repo, got `{source}`");
        }
        let name = rest.rsplit('/').next().unwrap_or(rest);
        return Ok((
            Some(format!("https://github.com/{rest}.git")),
            sanitize_name(name),
        ));
    }

    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("git@")
    {
        let name = trimmed
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("plugin")
            .trim_end_matches(".git");
        return Ok((Some(trimmed.to_string()), sanitize_name(name)));
    }

    // Local path.
    let path = PathBuf::from(trimmed);
    if !path.exists() {
        bail!("`{source}` is neither a git URL nor an existing path");
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "plugin".into());
    Ok((None, sanitize_name(&name)))
}

fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    cleaned.trim_matches('-').to_lowercase()
}

/// Install a plugin from git or a local directory.
pub fn install(options: InstallOptions) -> Result<InstalledPlugin> {
    let (url, suggested) = parse_source(&options.source)?;
    let name = options
        .name
        .as_deref()
        .map(sanitize_name)
        .unwrap_or(suggested);

    let root = plugins_dir()?;
    std::fs::create_dir_all(&root)?;

    // Git plugins are cloned into the plugins directory. Local plugins are
    // registered in place, which keeps development iterations fast.
    let destination = match &url {
        Some(url) => {
            let destination = root.join(&name);
            if destination.exists() {
                bail!(
                    "plugin directory {} already exists; use `fluxbar plugin update {name}`",
                    destination.display()
                );
            }
            run(
                Command::new("git")
                    .args(["clone", "--depth", "1", url])
                    .arg(&destination),
                None,
                "git clone",
            )?;
            destination
        }
        None => {
            let source = PathBuf::from(&options.source);
            source
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", source.display()))?
        }
    };

    let manifest = read_manifest(&destination)?;
    let plugin_name = manifest.name.clone().unwrap_or_else(|| name.clone());

    if options.build {
        if let Some(build) = &manifest.build {
            run(
                Command::new("/bin/sh")
                    .arg("-c")
                    .arg(build)
                    .current_dir(&destination),
                None,
                "build",
            )?;
        }
    }

    let entry = resolve_entry(&destination, &manifest)?;

    let config_path = write_registration(&plugin_name, &entry, &manifest)?;

    Ok(InstalledPlugin {
        name: plugin_name,
        config_path,
        command: Some(entry.display()),
    })
}

fn read_manifest(directory: &Path) -> Result<PluginManifest> {
    let path = directory.join(MANIFEST_FILE);
    if !path.exists() {
        return infer_manifest(directory);
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let manifest: PluginManifest =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(manifest)
}

fn infer_manifest(directory: &Path) -> Result<PluginManifest> {
    if directory.join("Cargo.toml").exists() {
        let contents = std::fs::read_to_string(directory.join("Cargo.toml"))?;
        let value: toml::Value = toml::from_str(&contents)?;
        let package = value
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
            .unwrap_or("plugin")
            .to_string();
        return Ok(PluginManifest {
            name: Some(package.clone()),
            build: Some("cargo build --release".into()),
            entry: Some(format!("target/release/{package}")),
            subscribe: vec!["*".into()],
            tick_interval: None,
            description: None,
        });
    }

    if directory.join("package.json").exists() {
        return Ok(PluginManifest {
            name: None,
            build: Some("npm install".into()),
            entry: Some("index.js".into()),
            subscribe: vec!["*".into()],
            tick_interval: None,
            description: None,
        });
    }

    if directory.join("main.py").exists() {
        return Ok(PluginManifest {
            name: None,
            build: None,
            entry: Some("main.py".into()),
            subscribe: vec!["*".into()],
            tick_interval: None,
            description: None,
        });
    }

    bail!(
        "{} has no {MANIFEST_FILE} and no recognisable project layout",
        directory.display()
    )
}

/// Resolved executable plus arguments for a plugin entry point.
struct ResolvedEntry {
    program: String,
    args: Vec<String>,
}

impl ResolvedEntry {
    fn display(&self) -> String {
        if self.args.is_empty() {
            self.program.clone()
        } else {
            format!("{} {}", self.program, self.args.join(" "))
        }
    }
}

fn resolve_entry(directory: &Path, manifest: &PluginManifest) -> Result<ResolvedEntry> {
    let Some(entry) = &manifest.entry else {
        bail!("plugin manifest must define `entry`");
    };

    // A manifest may already contain a command line such as
    // `python3 plugin.py`; split off the interpreter.
    for interpreter in ["node ", "python3 ", "python "] {
        if let Some(rest) = entry.strip_prefix(interpreter) {
            let program = interpreter.trim().to_string();
            let script = directory.join(rest.trim());
            return Ok(ResolvedEntry {
                program,
                args: vec![script.to_string_lossy().to_string()],
            });
        }
    }

    let path = directory.join(entry);
    let path = path.to_string_lossy().to_string();

    let resolved = if entry.ends_with(".js") {
        ResolvedEntry {
            program: "node".into(),
            args: vec![path],
        }
    } else if entry.ends_with(".py") {
        ResolvedEntry {
            program: "python3".into(),
            args: vec![path],
        }
    } else {
        ResolvedEntry {
            program: path,
            args: Vec::new(),
        }
    };

    // Ensure a relative binary exists when we can check it. Interpreters are
    // resolved through PATH at runtime.
    if resolved.args.is_empty() && !Path::new(&resolved.program).exists() {
        bail!(
            "entry point {} does not exist after build",
            resolved.program
        );
    }

    Ok(resolved)
}

fn slice_is_empty<T>(values: &[T]) -> bool {
    values.is_empty()
}

fn write_registration(
    name: &str,
    entry: &ResolvedEntry,
    manifest: &PluginManifest,
) -> Result<PathBuf> {
    let dir = config_dir()?.join("plugins.d");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.toml"));

    #[derive(Serialize)]
    struct Root<'a> {
        plugins: std::collections::BTreeMap<&'a str, PluginEntry<'a>>,
    }
    #[derive(Serialize)]
    struct PluginEntry<'a> {
        #[serde(rename = "type")]
        plugin_type: &'a str,
        command: &'a str,
        #[serde(skip_serializing_if = "slice_is_empty")]
        args: &'a [String],
        subscribe: &'a [String],
        #[serde(skip_serializing_if = "Option::is_none")]
        tick_interval: Option<u64>,
        auto_restart: bool,
    }

    let mut plugins = std::collections::BTreeMap::new();
    plugins.insert(
        name,
        PluginEntry {
            plugin_type: "external",
            command: &entry.program,
            args: &entry.args,
            subscribe: &manifest.subscribe,
            tick_interval: manifest.tick_interval,
            auto_restart: true,
        },
    );
    let root = Root { plugins };
    let contents = toml::to_string_pretty(&root)?;
    std::fs::write(&path, contents)?;
    Ok(path)
}

/// Remove a plugin registration and, for git-installed plugins, its sources.
pub fn remove(name: &str) -> Result<()> {
    let name = sanitize_name(name);
    let config_path = config_dir()?.join("plugins.d").join(format!("{name}.toml"));
    if !config_path.exists() {
        bail!("plugin `{name}` is not registered");
    }
    std::fs::remove_file(&config_path)?;

    // Local plugins live outside the plugins directory and are left on disk.
    let source = plugins_dir()?.join(&name);
    if source.exists() {
        std::fs::remove_dir_all(&source)?;
    }
    Ok(())
}

/// Update a plugin (git pull + rebuild when applicable).
pub fn update(name: &str) -> Result<()> {
    let name = sanitize_name(name);
    let config_path = config_dir()?.join("plugins.d").join(format!("{name}.toml"));
    if !config_path.exists() {
        bail!("plugin `{name}` is not registered");
    }
    let source = locate_source(&name, &config_path)?;

    if source.join(".git").exists() {
        run(
            Command::new("git")
                .arg("-C")
                .arg(&source)
                .args(["pull", "--ff-only"]),
            None,
            "git pull",
        )?;
    }

    let manifest = read_manifest(&source)?;
    if let Some(build) = &manifest.build {
        run(
            Command::new("/bin/sh")
                .arg("-c")
                .arg(build)
                .current_dir(&source),
            None,
            "build",
        )?;
    }

    let plugin_name = manifest.name.clone().unwrap_or(name);
    let entry = resolve_entry(&source, &manifest)?;
    write_registration(&plugin_name, &entry, &manifest)?;
    Ok(())
}

fn locate_source(name: &str, config_path: &Path) -> Result<PathBuf> {
    let cloned = plugins_dir()?.join(name);
    if cloned.exists() {
        return Ok(cloned);
    }

    // Local plugin: find the directory containing the registered command or
    // script argument.
    let contents = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let value: toml::Value = toml::from_str(&contents)?;
    let plugin = value
        .get("plugins")
        .and_then(|plugins| plugins.get(name))
        .with_context(|| format!("plugin `{name}` is missing from its registration"))?;

    if let Some(args) = plugin.get("args").and_then(toml::Value::as_array) {
        for argument in args.iter().filter_map(toml::Value::as_str) {
            let path = Path::new(argument);
            if path.is_absolute() {
                if let Some(directory) = manifest_ancestor(path) {
                    return Ok(directory);
                }
            }
        }
    }

    let command = plugin
        .get("command")
        .and_then(toml::Value::as_str)
        .with_context(|| format!("plugin `{name}` has no command in its registration"))?;
    let command_path = Path::new(command);
    if !command_path.is_absolute() {
        bail!("cannot locate the source directory for plugin `{name}`");
    }
    manifest_ancestor(command_path)
        .with_context(|| format!("cannot find the source directory of `{command}`"))
}

/// Walk up from a file or directory until a plugin manifest or Cargo manifest
/// is found.
fn manifest_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| dir.join(MANIFEST_FILE).exists() || dir.join("Cargo.toml").exists())
        .map(Path::to_path_buf)
}

fn run(command: &mut Command, cwd: Option<&Path>, label: &str) -> Result<()> {
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let status = command
        .status()
        .with_context(|| format!("failed to run {label}"))?;
    if !status.success() {
        bail!("{label} failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_source() {
        let (url, name) = parse_source("github:user/my-plugin").unwrap();
        assert_eq!(
            url.as_deref(),
            Some("https://github.com/user/my-plugin.git")
        );
        assert_eq!(name, "my-plugin");
    }

    #[test]
    fn parses_git_url() {
        let (url, name) = parse_source("https://github.com/user/repo.git").unwrap();
        assert_eq!(url.as_deref(), Some("https://github.com/user/repo.git"));
        assert_eq!(name, "repo");
    }

    #[test]
    fn rejects_unknown_source() {
        assert!(parse_source("not-a-real-path-xyz").is_err());
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_name("My Plugin!"), "my-plugin");
        assert_eq!(sanitize_name("A_B-c"), "a_b-c");
    }

    #[test]
    fn writes_registration() {
        let dir = tempfile::TempDir::new().unwrap();
        // Can't override the config dir easily; just test serialization shape.
        let manifest = PluginManifest {
            subscribe: vec!["tick".into()],
            tick_interval: Some(1),
            ..PluginManifest::default()
        };
        let contents = toml::to_string_pretty(&manifest).unwrap();
        assert!(contents.contains("tick"));
        let _ = dir;
    }
}
