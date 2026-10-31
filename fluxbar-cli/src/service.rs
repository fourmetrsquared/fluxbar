//! macOS login service management for source installs.
//!
//! Homebrew manages its own launchd agent through `brew services`. This module
//! provides the equivalent lifecycle for a binary installed from source:
//! `fluxbar service install|status|uninstall`.

use clap::Subcommand;

/// Login-service commands.
#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// Install and start a per-user LaunchAgent.
    Install {
        /// Write the plist but do not load or start it.
        #[arg(long)]
        no_start: bool,
    },
    /// Stop and remove the LaunchAgent.
    Uninstall {
        /// Also delete the service log files.
        #[arg(long)]
        remove_logs: bool,
    },
    /// Show the plist, launchd, daemon, and log status.
    Status,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::ServiceCommand;
    use crate::client;
    use anyhow::{Context, Result, bail};
    use serde_json::json;
    use std::fmt::Write as _;
    use std::fs;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    const LABEL: &str = "dev.fluxbar.fluxbar";
    const LAUNCHCTL: &str = "/bin/launchctl";
    const STOP_TIMEOUT: Duration = Duration::from_secs(10);
    const START_TIMEOUT: Duration = Duration::from_secs(15);

    #[derive(Debug)]
    struct ServicePaths {
        home: PathBuf,
        executable: PathBuf,
        plist: PathBuf,
        log_dir: PathBuf,
        log: PathBuf,
        error_log: PathBuf,
        socket: PathBuf,
        pid: PathBuf,
    }

    impl ServicePaths {
        fn new(home: &Path, executable: &Path) -> Self {
            let log_dir = home.join("Library/Logs/FluxBar");
            let socket = fluxbar_core::ipc::default_socket_path();
            Self {
                home: home.to_path_buf(),
                executable: executable.to_path_buf(),
                plist: home
                    .join("Library/LaunchAgents")
                    .join(format!("{LABEL}.plist")),
                log: log_dir.join("fluxbar.log"),
                error_log: log_dir.join("fluxbar.err.log"),
                log_dir,
                pid: socket.with_extension("pid"),
                socket,
            }
        }

        fn path_environment(&self) -> String {
            let mut entries = vec![self.home.join(".local/bin")];
            entries.extend(
                [
                    "/opt/homebrew/bin",
                    "/opt/homebrew/sbin",
                    "/usr/local/bin",
                    "/usr/local/sbin",
                    "/usr/bin",
                    "/bin",
                    "/usr/sbin",
                    "/sbin",
                ]
                .into_iter()
                .map(PathBuf::from),
            );
            entries
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join(":")
        }

        fn render_plist(&self) -> String {
            let mut xml = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
                 \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\">\n<dict>\n",
            );
            push_string(&mut xml, "Label", LABEL);
            xml.push_str("  <key>ProgramArguments</key>\n  <array>\n");
            push_array_string(&mut xml, &self.executable.to_string_lossy());
            push_array_string(&mut xml, "start");
            xml.push_str("  </array>\n");
            push_bool(&mut xml, "RunAtLoad", true);
            xml.push_str(
                "  <key>KeepAlive</key>\n  <dict>\n    <key>Crashed</key>\n    <true/>\n  </dict>\n",
            );
            push_string(&mut xml, "ProcessType", "Interactive");
            push_integer(&mut xml, "ThrottleInterval", 10);
            push_integer(&mut xml, "ExitTimeOut", 10);
            push_string(&mut xml, "LimitLoadToSessionType", "Aqua");
            push_string(&mut xml, "WorkingDirectory", &self.home.to_string_lossy());
            push_string(&mut xml, "StandardOutPath", &self.log.to_string_lossy());
            push_string(
                &mut xml,
                "StandardErrorPath",
                &self.error_log.to_string_lossy(),
            );
            xml.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
            push_string(&mut xml, "PATH", &self.path_environment());
            xml.push_str("  </dict>\n</dict>\n</plist>\n");
            xml
        }
    }

    fn push_string(xml: &mut String, key: &str, value: &str) {
        let _ = writeln!(
            xml,
            "  <key>{}</key>\n  <string>{}</string>",
            escape_xml(key),
            escape_xml(value)
        );
    }

    fn push_array_string(xml: &mut String, value: &str) {
        let _ = writeln!(xml, "    <string>{}</string>", escape_xml(value));
    }

    fn push_bool(xml: &mut String, key: &str, value: bool) {
        let _ = writeln!(
            xml,
            "  <key>{}</key>\n  <{} />",
            escape_xml(key),
            if value { "true" } else { "false" }
        );
    }

    fn push_integer(xml: &mut String, key: &str, value: i64) {
        let _ = writeln!(
            xml,
            "  <key>{}</key>\n  <integer>{value}</integer>",
            escape_xml(key)
        );
    }

    fn escape_xml(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn home_dir() -> Result<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set; cannot locate ~/Library/LaunchAgents")
    }

    fn executable() -> Result<PathBuf> {
        let path = std::env::current_exe().context("could not determine the fluxbar executable")?;
        Ok(fs::canonicalize(&path).unwrap_or(path))
    }

    fn service_paths() -> Result<ServicePaths> {
        Ok(ServicePaths::new(&home_dir()?, &executable()?))
    }

    fn current_uid() -> Result<String> {
        let output = Command::new("/usr/bin/id")
            .arg("-u")
            .output()
            .context("could not run `id -u`")?;
        if !output.status.success() {
            bail!(
                "`id -u` failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let uid = String::from_utf8(output.stdout).context("`id -u` returned non-UTF-8 output")?;
        let uid = uid.trim();
        if uid.is_empty() {
            bail!("`id -u` returned an empty user id");
        }
        Ok(uid.to_string())
    }

    fn launchctl(args: &[&str]) -> Result<std::process::Output> {
        Command::new(LAUNCHCTL)
            .args(args)
            .output()
            .with_context(|| format!("could not run `{LAUNCHCTL} {}`", args.join(" ")))
    }

    fn agent_loaded(uid: &str) -> bool {
        Command::new(LAUNCHCTL)
            .args(["print", &format!("gui/{uid}/{LABEL}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn bootout(uid: &str) -> Result<()> {
        let output = launchctl(&["bootout", &format!("gui/{uid}/{LABEL}")])?;
        if output.status.success() {
            return Ok(());
        }
        let message = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // launchctl uses a few different messages when the label is already
        // gone. Race with login/logout should not make uninstall fail.
        if message.contains("No such process")
            || message.contains("Could not find service")
            || message.contains("not find service")
        {
            return Ok(());
        }
        bail!("`launchctl bootout` failed: {}", message.trim());
    }

    fn bootstrap(uid: &str, plist: &Path) -> Result<()> {
        let output = launchctl(&["bootstrap", &format!("gui/{uid}"), &plist.to_string_lossy()])?;
        if !output.status.success() {
            let message = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            bail!("`launchctl bootstrap` failed: {}", message.trim());
        }
        Ok(())
    }

    fn daemon_ping(paths: &ServicePaths) -> bool {
        client::call(&paths.socket, "system.ping", json!({})).is_ok()
    }

    fn wait_for_daemon_start(paths: &ServicePaths) -> bool {
        wait_until(START_TIMEOUT, || daemon_ping(paths))
    }

    fn wait_for_daemon_stop(paths: &ServicePaths) -> bool {
        wait_until(STOP_TIMEOUT, || !daemon_ping(paths))
    }

    fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if predicate() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            sleep(Duration::from_millis(100));
        }
    }

    fn ensure_private_dir(path: &Path) -> Result<()> {
        fs::create_dir_all(path).with_context(|| format!("could not create {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .with_context(|| format!("could not restrict {}", path.display()))?;
        }
        Ok(())
    }

    fn write_plist(paths: &ServicePaths) -> Result<()> {
        ensure_private_dir(&paths.log_dir)?;
        for log in [&paths.log, &paths.error_log] {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .with_context(|| format!("could not create {}", log.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(log, fs::Permissions::from_mode(0o600))
                    .with_context(|| format!("could not restrict {}", log.display()))?;
            }
            let _ = file.flush();
        }

        if let Some(parent) = paths.plist.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        fs::write(&paths.plist, paths.render_plist())
            .with_context(|| format!("could not write {}", paths.plist.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&paths.plist, fs::Permissions::from_mode(0o644)).with_context(
                || format!("could not set permissions on {}", paths.plist.display()),
            )?;
        }
        Ok(())
    }

    fn install(no_start: bool) -> Result<()> {
        let paths = service_paths()?;
        write_plist(&paths)?;
        println!("wrote {}", paths.plist.display());

        if no_start {
            println!(
                "not loaded (--no-start); load it with `launchctl bootstrap gui/$(id -u) {}`",
                paths.plist.display()
            );
            return Ok(());
        }

        let uid = current_uid()?;
        if agent_loaded(&uid) {
            println!("unloading the existing {LABEL} agent");
            bootout(&uid)?;
            if !wait_for_daemon_stop(&paths) {
                bail!(
                    "the existing FluxBar service did not stop; check `launchctl print gui/{uid}/{LABEL}`"
                );
            }
        }

        if daemon_ping(&paths) {
            bail!(
                "FluxBar is already running outside the service; run `fluxbar stop` first, then retry `fluxbar service install`"
            );
        }

        bootstrap(&uid, &paths.plist)?;
        if !wait_for_daemon_start(&paths) {
            bail!(
                "the service was loaded but FluxBar did not become ready; check `launchctl print gui/{uid}/{LABEL}` and {}",
                paths.error_log.display()
            );
        }
        println!("loaded {LABEL}");
        println!("logs: {}", paths.log.display());
        println!("      {}", paths.error_log.display());
        Ok(())
    }

    fn uninstall(remove_logs: bool) -> Result<()> {
        let paths = service_paths()?;
        let uid = current_uid()?;
        if agent_loaded(&uid) {
            bootout(&uid)?;
            if !wait_for_daemon_stop(&paths) {
                bail!(
                    "the service was unloaded but FluxBar is still running; refusing to remove its socket"
                );
            }
        }

        match fs::remove_file(&paths.plist) {
            Ok(()) => println!("removed {}", paths.plist.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                println!("not installed: {}", paths.plist.display());
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not remove {}", paths.plist.display()));
            }
        }

        if !daemon_ping(&paths) {
            for path in [&paths.socket, &paths.pid] {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("could not remove {}", path.display()));
                    }
                }
            }
        }

        if remove_logs {
            for log in [&paths.log, &paths.error_log] {
                match fs::remove_file(log) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("could not remove {}", log.display()));
                    }
                }
            }
            let _ = fs::remove_dir(&paths.log_dir);
        }

        println!("uninstalled {LABEL}");
        Ok(())
    }

    fn status() -> Result<()> {
        let paths = service_paths()?;
        let uid = current_uid()?;
        println!("label:  {LABEL}");
        println!(
            "plist:  {} ({})",
            paths.plist.display(),
            if paths.plist.exists() {
                "installed"
            } else {
                "missing"
            }
        );
        println!(
            "agent:  {}",
            if agent_loaded(&uid) {
                "loaded"
            } else {
                "not loaded"
            }
        );
        match client::call(&paths.socket, "system.status", json!({})) {
            Ok(value) => println!(
                "daemon: running (version {}, uptime {}s, items {})",
                value["version"].as_str().unwrap_or("unknown"),
                value["uptime_secs"].as_u64().unwrap_or(0),
                value["items"].as_u64().unwrap_or(0)
            ),
            Err(_) => println!("daemon: not running"),
        }
        println!("logs:   {}", paths.log.display());
        println!("        {}", paths.error_log.display());
        Ok(())
    }

    pub fn run(command: ServiceCommand) -> Result<()> {
        match command {
            ServiceCommand::Install { no_start } => install(no_start),
            ServiceCommand::Uninstall { remove_logs } => uninstall(remove_logs),
            ServiceCommand::Status => status(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn paths() -> ServicePaths {
            ServicePaths::new(
                Path::new("/Users/Test User"),
                Path::new("/Users/Test & Co/bin/fluxbar"),
            )
        }

        #[test]
        fn plist_has_launchd_service_contract() {
            let xml = paths().render_plist();
            assert!(xml.contains("<key>Label</key>"));
            assert!(xml.contains("<string>dev.fluxbar.fluxbar</string>"));
            assert!(xml.contains("<key>RunAtLoad</key>"));
            assert!(xml.contains("<key>KeepAlive</key>"));
            assert!(xml.contains("<key>Crashed</key>"));
            assert!(xml.contains("<key>StandardOutPath</key>"));
            assert!(xml.contains("<key>StandardErrorPath</key>"));
            assert!(xml.contains("<key>LimitLoadToSessionType</key>"));
            assert!(xml.contains("<string>Aqua</string>"));
            assert!(xml.contains("<key>ExitTimeOut</key>"));
            assert!(xml.contains("<integer>10</integer>"));
            assert!(xml.contains("<string>start</string>"));
        }

        #[test]
        fn plist_escapes_paths_and_uses_private_logs() {
            let paths = paths();
            let xml = paths.render_plist();
            assert!(xml.contains("/Users/Test &amp; Co/bin/fluxbar"));
            assert!(xml.contains("/Users/Test User/Library/Logs/FluxBar/fluxbar.log"));
            assert!(xml.contains("/Users/Test User/Library/Logs/FluxBar/fluxbar.err.log"));
            assert!(xml.contains("/Users/Test User/.local/bin"));
            assert_eq!(
                paths.plist,
                Path::new("/Users/Test User/Library/LaunchAgents/dev.fluxbar.fluxbar.plist")
            );
        }

        #[test]
        fn xml_escape_handles_plist_specials() {
            assert_eq!(
                escape_xml("a&b<c>\"d\"'e'"),
                "a&amp;b&lt;c&gt;&quot;d&quot;&apos;e&apos;"
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::ServiceCommand;
    use anyhow::{Result, bail};

    pub fn run(_command: ServiceCommand) -> Result<()> {
        bail!(
            "`fluxbar service` is only available on macOS; use launchd or another service manager on this platform"
        )
    }
}

pub use imp::run;
