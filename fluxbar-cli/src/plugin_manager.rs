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
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use fluxbar_core::config::{Config, PluginRuntime};

/// Manifest file name inside plugin repositories.
pub const MANIFEST_FILE: &str = "fluxbar-plugin.toml";

/// Metadata written next to a copied local plugin, pointing at its source.
const LOCAL_INSTALL_METADATA: &str = ".fluxbar-install.toml";

/// Manifests are small; cap them so a hostile repository cannot make the
/// installer allocate an arbitrary amount of memory.
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

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
    /// Script runtime (`sh`, `bash`, `lua`) for event-driven plugins.
    #[serde(default)]
    pub runtime: Option<PluginRuntime>,
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
    /// Register a local plugin in place instead of copying it into the
    /// configuration directory.
    pub link: bool,
}

/// Result of a successful installation.
#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    pub name: String,
    pub config_path: PathBuf,
    pub location: PathBuf,
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

/// Create the plugin source and registration directories under `config_dir`.
pub fn ensure_layout(config_dir: &Path) -> Result<()> {
    create_private_dir(config_dir)?;
    create_private_dir(&config_dir.join("plugins"))?;
    create_private_dir(&config_dir.join("plugins.d"))?;
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(path)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)?;
        Ok(())
    }
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
            validated_name(name)?,
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
        return Ok((Some(trimmed.to_string()), validated_name(name)?));
    }

    // Local path.
    let path = PathBuf::from(trimmed);
    if !path.exists() {
        bail!("`{source}` is neither a git URL nor an existing path");
    }
    let name = if path.is_file() {
        path.file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "plugin".into())
    } else {
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "plugin".into())
    };
    Ok((None, validated_name(&name)?))
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

/// Validate a plugin name before it is used as a path component.
///
/// Empty names are dangerous: joining them with `plugins` yields the plugins
/// root itself, and removing such a plugin would recursively delete every
/// installed plugin. Keep the accepted alphabet aligned with
/// [`sanitize_name`].
fn validated_name(name: &str) -> Result<String> {
    if name.contains('/') || name.contains('\\') {
        bail!("plugin name must not contain path separators");
    }
    let name = sanitize_name(name);
    if name.is_empty() {
        bail!("plugin name must contain at least one ASCII letter or digit");
    }
    Ok(name)
}

fn manifest_name(manifest: &PluginManifest, fallback: &str) -> Result<String> {
    manifest
        .name
        .as_deref()
        .map(validated_name)
        .unwrap_or_else(|| validated_name(fallback))
}

/// Validate an entry path from a manifest. Entry points must stay inside the
/// plugin directory even when the repository or local source is untrusted.
fn validated_entry_path(entry: &str) -> Result<PathBuf> {
    let entry = entry.trim();
    if entry.is_empty() {
        bail!("plugin entry must not be empty");
    }
    let path = Path::new(entry);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("plugin entry `{entry}` must be relative to the plugin directory");
    }
    Ok(path.to_path_buf())
}

/// Install a plugin from git or a local directory.
///
/// Git plugins are cloned into the configuration directory. Local plugins are
/// copied there as well, so plugin sources and registrations live together.
/// Pass [`InstallOptions::link`] to register a local plugin in place instead.
pub fn install(options: InstallOptions) -> Result<InstalledPlugin> {
    install_at(options, &config_dir()?)
}

fn install_at(options: InstallOptions, config_dir: &Path) -> Result<InstalledPlugin> {
    let (url, suggested) = parse_source(&options.source)?;
    let name = options
        .name
        .as_deref()
        .map(validated_name)
        .transpose()?
        .unwrap_or(suggested);

    ensure_layout(config_dir)?;
    let root = config_dir.join("plugins");
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", root.display()))?;

    if let Some(url) = url {
        if options.link {
            bail!("--link is only supported for local plugin sources");
        }
        let destination = root.join(&name);
        if destination.exists() {
            bail!(
                "plugin directory {} already exists; use `fluxbar plugin update {name}`",
                destination.display()
            );
        }
        run(
            Command::new("git")
                .args(["clone", "--depth", "1", &url])
                .arg(&destination),
            None,
            "git clone",
        )?;

        let manifest = read_manifest(&destination)?;
        let plugin_name = manifest_name(&manifest, &name)?;
        if options.build {
            build_plugin(&destination, &manifest)?;
        }
        let entry = resolve_entry(&destination, &manifest)?;
        let config_path = write_registration(config_dir, &plugin_name, &entry, &manifest)?;

        return Ok(InstalledPlugin {
            name: plugin_name,
            config_path,
            location: destination,
            command: Some(entry.display()),
        });
    }

    let source = PathBuf::from(&options.source);
    let source = source
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", source.display()))?;
    if source.is_file() {
        return install_script_file(options, config_dir, &source, &name);
    }
    if !source.is_dir() {
        bail!(
            "local plugin source must be a directory or script: {}",
            source.display()
        );
    }

    let manifest = read_manifest(&source)?;
    let plugin_name = manifest_name(&manifest, &name)?;

    if options.build {
        build_plugin(&source, &manifest)?;
    }
    let source_entry = resolve_entry(&source, &manifest)?;

    if options.link {
        let config_path = write_registration(config_dir, &plugin_name, &source_entry, &manifest)?;
        return Ok(InstalledPlugin {
            name: plugin_name,
            config_path,
            location: source,
            command: Some(source_entry.display()),
        });
    }

    let destination = root.join(&name);
    if destination.exists() {
        bail!(
            "plugin directory {} already exists; use `fluxbar plugin update {name}`",
            destination.display()
        );
    }
    ensure_disjoint(&source, &destination)?;
    if let Err(error) = copy_local_install(&source, &destination, &manifest) {
        // Do not leave a half-installed plugin behind.
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }

    let (entry, config_path) = match (|| {
        let entry = resolve_entry(&destination, &manifest)?;
        write_local_metadata(&destination, &source)?;
        let config_path = write_registration(config_dir, &plugin_name, &entry, &manifest)?;
        Ok((entry, config_path))
    })() {
        Ok(result) => result,
        Err(error) => {
            let _ = fs::remove_dir_all(&destination);
            return Err(error);
        }
    };

    Ok(InstalledPlugin {
        name: plugin_name,
        config_path,
        location: destination,
        command: Some(entry.display()),
    })
}

fn install_script_file(
    options: InstallOptions,
    config_dir: &Path,
    source: &Path,
    name: &str,
) -> Result<InstalledPlugin> {
    let runtime = PluginRuntime::from_entry(&source.to_string_lossy());
    if !runtime.is_script() {
        bail!(
            "single-file plugins must be .lua, .sh or .bash scripts: {}",
            source.display()
        );
    }

    let file_name = source
        .file_name()
        .context("plugin script has no file name")?;
    let manifest = PluginManifest {
        name: Some(name.to_string()),
        entry: Some(file_name.to_string_lossy().to_string()),
        runtime: Some(runtime),
        subscribe: vec!["*".into()],
        ..PluginManifest::default()
    };

    if options.link {
        let entry = ResolvedEntry {
            program: match runtime {
                PluginRuntime::Lua => "lua".into(),
                PluginRuntime::Bash => "/bin/bash".into(),
                PluginRuntime::Sh | PluginRuntime::JsonRpc => "/bin/sh".into(),
            },
            args: vec![source.to_string_lossy().to_string()],
        };
        let config_path = write_registration(config_dir, name, &entry, &manifest)?;
        return Ok(InstalledPlugin {
            name: name.to_string(),
            config_path,
            location: source.parent().unwrap_or(source).to_path_buf(),
            command: Some(entry.display()),
        });
    }

    let destination = config_dir.join("plugins").join(name);
    if destination.exists() {
        bail!(
            "plugin directory {} already exists; use `fluxbar plugin update {name}`",
            destination.display()
        );
    }
    fs::create_dir_all(&destination)?;
    let destination_file = destination.join(file_name);
    fs::copy(source, &destination_file).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            destination_file.display()
        )
    })?;
    write_local_metadata(&destination, source)?;
    let entry = resolve_entry(&destination, &manifest)?;
    let config_path = write_registration(config_dir, name, &entry, &manifest)?;

    Ok(InstalledPlugin {
        name: name.to_string(),
        config_path,
        location: destination,
        command: Some(entry.display()),
    })
}

fn build_plugin(directory: &Path, manifest: &PluginManifest) -> Result<()> {
    if let Some(build) = &manifest.build {
        run(
            Command::new("/bin/sh")
                .arg("-c")
                .arg(build)
                .current_dir(directory),
            None,
            "build",
        )?;
    }
    Ok(())
}

fn ensure_disjoint(source: &Path, destination: &Path) -> Result<()> {
    if source == destination || destination.starts_with(source) || source.starts_with(destination) {
        bail!(
            "cannot copy plugin source {} into {}",
            source.display(),
            destination.display()
        );
    }
    Ok(())
}

fn should_skip_root_entry(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git" | "target" | LOCAL_INSTALL_METADATA)
    )
}

fn copy_local_install(source: &Path, destination: &Path, manifest: &PluginManifest) -> Result<()> {
    copy_plugin_source(source, destination)?;
    copy_entry_artifact(source, destination, manifest)?;
    Ok(())
}

fn copy_plugin_source(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        if should_skip_root_entry(&entry.file_name()) {
            continue;
        }
        copy_path(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn copy_entry_artifact(source: &Path, destination: &Path, manifest: &PluginManifest) -> Result<()> {
    let Some(entry) = &manifest.entry else {
        return Ok(());
    };

    for interpreter in [
        "node ", "python3 ", "python ", "lua ", "luajit ", "sh ", "bash ",
    ] {
        if let Some(script) = entry.strip_prefix(interpreter) {
            validated_entry_path(script)?;
            return Ok(());
        }
    }

    let entry_path = validated_entry_path(entry)?;
    let from = source.join(&entry_path);
    if !from.exists() {
        bail!("entry point {} does not exist after build", from.display());
    }
    copy_path(&from, &destination.join(entry_path))
}

fn copy_path(from: &Path, to: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(from).with_context(|| format!("cannot inspect {}", from.display()))?;

    if metadata.file_type().is_symlink() {
        copy_symlink(from, to)?;
    } else if metadata.is_dir() {
        if fs::symlink_metadata(to).is_ok() {
            remove_path(to)?;
        }
        fs::create_dir_all(to)?;
        for entry in
            fs::read_dir(from).with_context(|| format!("failed to read {}", from.display()))?
        {
            let entry = entry?;
            copy_path(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::symlink_metadata(to).is_ok() {
            remove_path(to)?;
        }
        fs::copy(from, to)
            .with_context(|| format!("failed to copy {} to {}", from.display(), to.display()))?;
    }

    Ok(())
}

#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> Result<()> {
    let target =
        fs::read_link(from).with_context(|| format!("cannot read symlink {}", from.display()))?;
    if fs::symlink_metadata(to).is_ok() {
        remove_path(to)?;
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    std::os::unix::fs::symlink(&target, to).with_context(|| {
        format!(
            "failed to copy symlink {} to {}",
            from.display(),
            to.display()
        )
    })
}

#[cfg(not(unix))]
fn copy_symlink(from: &Path, to: &Path) -> Result<()> {
    // Symlinks are not portable; copy the resolved target when possible.
    copy_path(&from.canonicalize()?, to)
}

fn remove_path(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct LocalInstall {
    source: PathBuf,
}

fn write_local_metadata(destination: &Path, source: &Path) -> Result<()> {
    let metadata = LocalInstall {
        source: source.to_path_buf(),
    };
    let contents = toml::to_string(&metadata)?;
    fs::write(destination.join(LOCAL_INSTALL_METADATA), contents)?;
    Ok(())
}

fn read_local_metadata(destination: &Path) -> Result<Option<LocalInstall>> {
    let path = destination.join(LOCAL_INSTALL_METADATA);
    if !path.exists() {
        return Ok(None);
    }
    let contents = read_text_limited(&path, MAX_MANIFEST_BYTES)?;
    let metadata =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(metadata))
}

fn read_manifest(directory: &Path) -> Result<PluginManifest> {
    let path = directory.join(MANIFEST_FILE);
    if !path.exists() {
        return infer_manifest(directory);
    }
    let contents = read_text_limited(&path, MAX_MANIFEST_BYTES)?;
    let manifest: PluginManifest =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(manifest)
}

fn read_text_limited(path: &Path, limit: u64) -> Result<String> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut contents = String::new();
    file.take(limit + 1)
        .read_to_string(&mut contents)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if contents.len() as u64 > limit {
        bail!("{} is too large (limit {limit} bytes)", path.display());
    }
    Ok(contents)
}

fn write_text_private(path: &Path, contents: &str) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        fs::remove_file(path)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn infer_manifest(directory: &Path) -> Result<PluginManifest> {
    if directory.join("Cargo.toml").exists() {
        let contents = read_text_limited(&directory.join("Cargo.toml"), MAX_MANIFEST_BYTES)?;
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
            runtime: None,
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
            runtime: None,
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
            runtime: None,
            subscribe: vec!["*".into()],
            tick_interval: None,
            description: None,
        });
    }

    for (file, runtime) in [
        ("plugin.lua", PluginRuntime::Lua),
        ("main.lua", PluginRuntime::Lua),
        ("plugin.bash", PluginRuntime::Bash),
        ("main.bash", PluginRuntime::Bash),
        ("plugin.sh", PluginRuntime::Sh),
        ("main.sh", PluginRuntime::Sh),
    ] {
        if directory.join(file).exists() {
            return Ok(PluginManifest {
                name: None,
                build: None,
                entry: Some(file.into()),
                runtime: Some(runtime),
                subscribe: vec!["*".into()],
                tick_interval: None,
                description: None,
            });
        }
    }

    bail!(
        "{} has no {MANIFEST_FILE} and no recognisable project layout (expected Cargo.toml, package.json, main.py, plugin.lua or plugin.sh)",
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
    for (interpreter, program) in [
        ("node ", "node"),
        ("python3 ", "python3"),
        ("python ", "python3"),
        ("lua ", "lua"),
        ("luajit ", "lua"),
        ("sh ", "/bin/sh"),
        ("bash ", "/bin/bash"),
    ] {
        if let Some(rest) = entry.strip_prefix(interpreter) {
            let script = directory.join(validated_entry_path(rest)?);
            return Ok(ResolvedEntry {
                program: program.to_string(),
                args: vec![script.to_string_lossy().to_string()],
            });
        }
    }

    let relative = validated_entry_path(entry)?;
    let path = directory.join(relative);
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
    } else if entry.ends_with(".lua") {
        ResolvedEntry {
            program: "lua".into(),
            args: vec![path],
        }
    } else if entry.ends_with(".bash") {
        ResolvedEntry {
            program: "/bin/bash".into(),
            args: vec![path],
        }
    } else if entry.ends_with(".sh") {
        ResolvedEntry {
            program: "/bin/sh".into(),
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
    config_dir: &Path,
    name: &str,
    entry: &ResolvedEntry,
    manifest: &PluginManifest,
) -> Result<PathBuf> {
    let name = validated_name(name)?;
    let dir = config_dir.join("plugins.d");
    fs::create_dir_all(&dir)?;
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
        #[serde(skip_serializing_if = "Option::is_none")]
        runtime: Option<&'a str>,
        subscribe: &'a [String],
        #[serde(skip_serializing_if = "Option::is_none")]
        tick_interval: Option<u64>,
        auto_restart: bool,
    }

    let runtime = manifest.runtime.or_else(|| {
        let candidate = entry
            .args
            .iter()
            .find(|arg| {
                std::path::Path::new(arg)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| matches!(extension, "lua" | "sh" | "bash"))
            })
            .cloned()
            .unwrap_or_else(|| entry.program.clone());
        Some(PluginRuntime::from_entry(&candidate))
    });
    let runtime = runtime.filter(|runtime| runtime.is_script());

    let mut plugins = std::collections::BTreeMap::new();
    plugins.insert(
        name.as_str(),
        PluginEntry {
            plugin_type: "external",
            command: &entry.program,
            args: &entry.args,
            runtime: runtime.map(PluginRuntime::as_str),
            subscribe: &manifest.subscribe,
            tick_interval: manifest.tick_interval,
            auto_restart: true,
        },
    );
    let root = Root { plugins };
    let contents = toml::to_string_pretty(&root)?;
    write_text_private(&path, &contents)?;
    Ok(path)
}

/// Remove a plugin registration and its installed sources.
///
/// Git clones and copied local plugins live inside the configuration
/// directory. Linked local plugins are left untouched.
pub fn remove(name: &str) -> Result<()> {
    remove_at(name, &config_dir()?)
}

fn remove_at(name: &str, config_dir: &Path) -> Result<()> {
    let name = validated_name(name)?;
    let config_path = config_dir.join("plugins.d").join(format!("{name}.toml"));
    if !config_path.exists() {
        bail!("plugin `{name}` is not registered");
    }
    fs::remove_file(&config_path)?;

    let installed = config_dir.join("plugins").join(&name);
    if installed.exists() {
        fs::remove_dir_all(&installed)?;
    }
    Ok(())
}

/// Update a plugin (git pull + rebuild when applicable).
pub fn update(name: &str) -> Result<()> {
    update_at(name, &config_dir()?)
}

fn update_at(name: &str, config_dir: &Path) -> Result<()> {
    let name = validated_name(name)?;
    let config_path = config_dir.join("plugins.d").join(format!("{name}.toml"));
    if !config_path.exists() {
        bail!("plugin `{name}` is not registered");
    }

    let destination = config_dir.join("plugins").join(&name);
    let copied = read_local_metadata(&destination)?;
    let source = match &copied {
        Some(metadata) => metadata.source.clone(),
        None => locate_source(config_dir, &name, &config_path)?,
    };
    if !source.exists() {
        bail!("plugin source {} no longer exists", source.display());
    }

    if source.is_file() {
        if copied.is_none() {
            bail!("linked script plugins are updated in place");
        }
        let runtime = PluginRuntime::from_entry(&source.to_string_lossy());
        if !runtime.is_script() {
            bail!("unsupported single-file plugin {}", source.display());
        }
        let file_name = source
            .file_name()
            .context("plugin script has no file name")?;
        fs::create_dir_all(&destination)?;
        fs::copy(&source, destination.join(file_name))?;
        write_local_metadata(&destination, &source)?;
        let manifest = PluginManifest {
            name: Some(name.clone()),
            entry: Some(file_name.to_string_lossy().to_string()),
            runtime: Some(runtime),
            subscribe: vec!["*".into()],
            ..PluginManifest::default()
        };
        let entry = resolve_entry(&destination, &manifest)?;
        write_registration(config_dir, &name, &entry, &manifest)?;
        return Ok(());
    }

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

    let plugin_name = manifest_name(&manifest, &name)?;
    let mut entry = resolve_entry(&source, &manifest)?;

    if copied.is_some() {
        ensure_disjoint(&source, &destination)?;
        copy_local_install(&source, &destination, &manifest)?;
        write_local_metadata(&destination, &source)?;
        entry = resolve_entry(&destination, &manifest)?;
    }

    write_registration(config_dir, &plugin_name, &entry, &manifest)?;
    Ok(())
}

fn locate_source(config_dir: &Path, name: &str, config_path: &Path) -> Result<PathBuf> {
    let installed = config_dir.join("plugins").join(name);
    if installed.exists() {
        return Ok(installed);
    }

    // Local plugin: find the directory containing the registered command or
    // script argument.
    let contents = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let value: toml::Value = toml::from_str(&contents)?;
    let plugin = value
        .get("plugins")
        .and_then(|plugins| plugins.get(name))
        .with_context(|| format!("plugin `{name}` is missing from its registration"))?;

    if let Some(args) = plugin.get("args").and_then(toml::Value::as_array) {
        for argument in args.iter().filter_map(toml::Value::as_str) {
            let path = Path::new(argument);
            if path.is_absolute()
                && let Some(directory) = manifest_ancestor(path)
            {
                return Ok(directory);
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
    use tempfile::TempDir;

    fn write_python_plugin(root: &Path) -> PathBuf {
        let source = root.join("demo");
        fs::create_dir_all(source.join("target")).unwrap();
        fs::create_dir_all(source.join(".git")).unwrap();
        fs::write(source.join("target/ignored"), "ignored").unwrap();
        fs::write(source.join(".git/ignored"), "ignored").unwrap();
        fs::write(
            source.join(MANIFEST_FILE),
            r#"
name = "demo"
entry = "plugin.py"
subscribe = ["*"]
"#,
        )
        .unwrap();
        fs::write(source.join("plugin.py"), "print('v1')").unwrap();
        source
    }

    fn copy_options(source: &Path) -> InstallOptions {
        InstallOptions {
            source: source.display().to_string(),
            name: None,
            build: false,
            link: false,
        }
    }

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
    fn rejects_empty_and_escaping_names() {
        assert!(validated_name("!!!").is_err());
        assert!(validated_name("../../etc/passwd").is_err());
        assert_eq!(validated_name("My Plugin!").unwrap(), "my-plugin");
    }

    #[test]
    fn rejects_escaping_entry_paths() {
        assert!(validated_entry_path("../outside").is_err());
        assert!(validated_entry_path("/usr/bin/true").is_err());
        assert!(validated_entry_path("target/release/plugin").is_ok());
        assert!(validated_entry_path("./plugin.py").is_ok());
    }

    #[test]
    fn rejects_manifest_registration_traversal() {
        let manifest = PluginManifest {
            name: Some("../../escaped".into()),
            ..PluginManifest::default()
        };
        assert!(manifest_name(&manifest, "fallback").is_err());

        let config = TempDir::new().unwrap();
        let entry = ResolvedEntry {
            program: "/usr/bin/demo".into(),
            args: Vec::new(),
        };
        assert!(
            write_registration(
                config.path(),
                "../../escaped",
                &entry,
                &PluginManifest::default()
            )
            .is_err()
        );
        assert!(
            !config
                .path()
                .parent()
                .unwrap()
                .join("escaped.toml")
                .exists()
        );
    }

    #[test]
    fn rejects_manifest_entry_traversal_during_install() {
        let config = TempDir::new().unwrap();
        let source_root = TempDir::new().unwrap();
        let source = write_python_plugin(source_root.path());
        fs::write(
            source.join(MANIFEST_FILE),
            r#"
name = "demo"
entry = "../escaped.py"
"#,
        )
        .unwrap();

        let error = install_at(copy_options(&source), config.path()).unwrap_err();
        assert!(error.to_string().contains("must be relative"));
        assert!(!config.path().join("plugins/demo").exists());
    }

    #[test]
    fn rejects_oversized_manifest() {
        let root = TempDir::new().unwrap();
        fs::write(
            root.path().join(MANIFEST_FILE),
            "x".repeat(MAX_MANIFEST_BYTES as usize + 1),
        )
        .unwrap();
        let error = read_manifest(root.path()).unwrap_err();
        assert!(error.to_string().contains("too large"));
    }

    #[test]
    fn local_plugin_is_copied_into_config_dir() {
        let config = TempDir::new().unwrap();
        let source_root = TempDir::new().unwrap();
        let source = write_python_plugin(source_root.path());

        let installed = install_at(copy_options(&source), config.path()).unwrap();
        let destination = config.path().join("plugins/demo");

        assert_eq!(installed.name, "demo");
        assert_eq!(installed.location, destination.canonicalize().unwrap());
        assert!(destination.join("plugin.py").exists());
        assert!(destination.join(LOCAL_INSTALL_METADATA).exists());
        // Build output and VCS metadata are not duplicated.
        assert!(!destination.join("target").exists());
        assert!(!destination.join(".git").exists());
        // The original source is untouched.
        assert!(source.join("plugin.py").exists());

        let registration = fs::read_to_string(&installed.config_path).unwrap();
        let expected = destination.join("plugin.py");
        assert!(registration.contains(&expected.display().to_string()));
        assert!(registration.contains("python3"));
    }

    #[test]
    fn link_keeps_local_plugin_in_place() {
        let config = TempDir::new().unwrap();
        let source_root = TempDir::new().unwrap();
        let source = write_python_plugin(source_root.path());

        let mut options = copy_options(&source);
        options.link = true;
        let installed = install_at(options, config.path()).unwrap();

        assert_eq!(installed.location, source.canonicalize().unwrap());
        assert!(!config.path().join("plugins/demo").exists());
        let registration = fs::read_to_string(&installed.config_path).unwrap();
        assert!(registration.contains(&source.canonicalize().unwrap().display().to_string()));
    }

    #[test]
    fn copied_local_plugin_can_be_updated_and_removed() {
        let config = TempDir::new().unwrap();
        let source_root = TempDir::new().unwrap();
        let source = write_python_plugin(source_root.path());
        install_at(copy_options(&source), config.path()).unwrap();

        // The fake .git directory used to verify exclusion is not a real
        // repository, so do not let update try to pull it.
        fs::remove_dir_all(source.join(".git")).unwrap();
        fs::write(source.join("plugin.py"), "print('v2')").unwrap();
        update_at("demo", config.path()).unwrap();
        assert_eq!(
            fs::read_to_string(config.path().join("plugins/demo/plugin.py")).unwrap(),
            "print('v2')"
        );

        remove_at("demo", config.path()).unwrap();
        assert!(!config.path().join("plugins/demo").exists());
        assert!(!config.path().join("plugins.d/demo.toml").exists());
        assert!(source.join("plugin.py").exists());
    }

    #[test]
    fn installs_single_script_file() {
        let config = TempDir::new().unwrap();
        let source_root = TempDir::new().unwrap();
        let source = source_root.path().join("tick.sh");
        fs::write(&source, "#!/bin/sh\nprintf '{}\\n'\n").unwrap();

        let installed = install_at(
            InstallOptions {
                source: source.display().to_string(),
                name: None,
                build: false,
                link: false,
            },
            config.path(),
        )
        .unwrap();

        assert_eq!(installed.name, "tick");
        assert!(config.path().join("plugins/tick/tick.sh").exists());
        let registration = fs::read_to_string(installed.config_path).unwrap();
        assert!(registration.contains("command = \"/bin/sh\""));
        assert!(registration.contains("runtime = \"sh\""));
        assert!(registration.contains("plugins/tick/tick.sh"));
    }

    #[test]
    fn link_is_rejected_for_git_sources() {
        let config = TempDir::new().unwrap();
        let options = InstallOptions {
            source: "github:user/repo".into(),
            name: None,
            build: false,
            link: true,
        };
        let error = install_at(options, config.path()).unwrap_err();
        assert!(error.to_string().contains("--link"));
    }

    #[test]
    fn writes_registration() {
        let config = TempDir::new().unwrap();
        let manifest = PluginManifest {
            subscribe: vec!["tick".into()],
            tick_interval: Some(1),
            runtime: Some(PluginRuntime::Sh),
            ..PluginManifest::default()
        };
        let entry = ResolvedEntry {
            program: "/usr/bin/demo".into(),
            args: Vec::new(),
        };
        let path = write_registration(config.path(), "demo", &entry, &manifest).unwrap();
        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains("[plugins.demo]"));
        assert!(contents.contains("command = \"/usr/bin/demo\""));
        assert!(contents.contains("tick_interval = 1"));
    }
}
