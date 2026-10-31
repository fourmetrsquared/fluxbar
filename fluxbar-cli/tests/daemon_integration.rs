//! End-to-end tests for the real `fluxbar` binary.
//!
//! These tests start the daemon headless with a private temporary config and
//! socket, then exercise the same JSON-RPC and CLI paths users and plugins use.
//! They are intentionally process-level tests: unit tests cover the individual
//! pieces, while this suite protects the lifecycle guarantees introduced by the
//! P0 hardening work.

#![cfg(unix)]

use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_fluxbar");
const START_TIMEOUT: Duration = Duration::from_secs(20);
const RPC_TIMEOUT: Duration = Duration::from_secs(10);

struct TestEnv {
    dir: TempDir,
    config: PathBuf,
    socket: PathBuf,
    pid: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let dir = TempDir::new().expect("temporary directory");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();

        let config_dir = dir.path().join("config");
        let run_dir = dir.path().join("run");
        fs::create_dir(&config_dir).unwrap();
        fs::create_dir(&run_dir).unwrap();
        fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&run_dir, fs::Permissions::from_mode(0o700)).unwrap();

        let socket = run_dir.join("fluxbar.sock");
        let env = Self {
            config: config_dir.join("fluxbar.toml"),
            pid: socket.with_extension("pid"),
            socket,
            dir,
        };
        env.write_config(
            r#"
[bar]
height = 30

[items.base]
label = "base"
"#,
        );
        env
    }

    fn write_config(&self, contents: &str) {
        fs::write(&self.config, contents).expect("write config");
        fs::set_permissions(&self.config, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(BIN);
        command
            .arg("--socket")
            .arg(&self.socket)
            .arg("--config")
            .arg(&self.config);
        command
    }

    fn cli(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .output()
            .expect("run fluxbar command")
    }

    fn cli_ok(&self, args: &[&str]) -> String {
        let output = self.cli(args);
        assert!(
            output.status.success(),
            "`fluxbar {}` failed with {}\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn rpc(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut stream = UnixStream::connect(&self.socket)
            .map_err(|error| format!("connect to {}: {error}", self.socket.display()))?;
        stream
            .set_read_timeout(Some(RPC_TIMEOUT))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(RPC_TIMEOUT))
            .map_err(|error| error.to_string())?;

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let mut bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        stream
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if line.is_empty() {
            return Err("daemon closed the IPC connection".into());
        }

        let response: Value =
            serde_json::from_str(line.trim()).map_err(|error| error.to_string())?;
        if let Some(error) = response.get("error") {
            return Err(error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("JSON-RPC error")
                .to_string());
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    fn start(&self) -> Daemon {
        let log_path = self.dir.path().join("daemon.log");
        let log = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&log_path)
            .expect("create daemon log");
        let child = self
            .command()
            .args(["start", "--headless"])
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .expect("start fluxbar daemon");

        Daemon {
            child: Some(child),
            socket: self.socket.clone(),
            pid: self.pid.clone(),
            log: log_path,
        }
    }
}

struct Daemon {
    child: Option<Child>,
    socket: PathBuf,
    pid: PathBuf,
    log: PathBuf,
}

impl Daemon {
    fn pid(&self) -> u32 {
        self.child.as_ref().expect("daemon running").id()
    }

    fn wait_ready(&mut self, env: &TestEnv) {
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if env.rpc("system.ping", json!({})).is_ok() {
                return;
            }
            if let Some(status) = self
                .child
                .as_mut()
                .expect("daemon running")
                .try_wait()
                .expect("poll daemon")
            {
                panic!(
                    "daemon exited before becoming ready ({status})\n{}",
                    self.log_contents()
                );
            }
            if Instant::now() >= deadline {
                self.kill();
                panic!(
                    "daemon did not become ready within {START_TIMEOUT:?}\n{}",
                    self.log_contents()
                );
            }
            sleep(Duration::from_millis(50));
        }
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self
                .child
                .as_mut()
                .expect("daemon running")
                .try_wait()
                .expect("poll daemon")
            {
                self.child = None;
                return status;
            }
            if Instant::now() >= deadline {
                self.kill();
                panic!(
                    "daemon did not stop within {timeout:?}\n{}",
                    self.log_contents()
                );
            }
            sleep(Duration::from_millis(25));
        }
    }

    fn stop(&mut self, env: &TestEnv) {
        env.cli_ok(&["stop"]);
        let status = self.wait_for_exit(Duration::from_secs(10));
        assert!(status.success(), "daemon exited with {status}");
        self.assert_cleanup();
    }

    fn signal(&self, signal: &str) {
        let status = Command::new("/bin/kill")
            .arg(format!("-{signal}"))
            .arg(self.pid().to_string())
            .status()
            .expect("send signal");
        assert!(status.success(), "kill -{signal} failed with {status}");
    }

    fn assert_cleanup(&self) {
        assert!(!self.socket.exists(), "socket was not removed");
        assert!(!self.pid.exists(), "PID file was not removed");
    }

    fn log_contents(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_else(|error| format!("<no log: {error}>"))
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.kill();
    }
}

fn run_with_timeout(command: &mut Command, timeout: Duration) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn command");
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().expect("poll command").is_some() {
            return child.wait_with_output().expect("collect command output");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().expect("collect timed-out output");
            panic!(
                "command timed out after {timeout:?}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        sleep(Duration::from_millis(25));
    }
}

fn file_mode(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|error| panic!("metadata {}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

fn find_item<'a>(items: &'a Value, name: &str) -> Option<&'a Value> {
    items
        .as_array()?
        .iter()
        .find(|item| item["name"].as_str() == Some(name))
}

fn find_plugin<'a>(plugins: &'a Value, name: &str) -> Option<&'a Value> {
    plugins
        .as_array()?
        .iter()
        .find(|plugin| plugin["name"].as_str() == Some(name))
}

#[test]
fn daemon_lifecycle_item_crud_permissions_and_stale_pid() {
    let env = TestEnv::new();
    let mut daemon = env.start();
    daemon.wait_ready(&env);

    let ping = env.rpc("system.ping", json!({})).unwrap();
    assert_eq!(ping["pong"], true);

    assert_eq!(file_mode(&env.socket), 0o600, "socket permissions");
    assert_eq!(file_mode(&env.pid), 0o600, "PID permissions");
    assert_eq!(file_mode(&env.config), 0o600, "config permissions");
    assert_eq!(
        file_mode(env.config.parent().unwrap()),
        0o700,
        "config directory permissions"
    );

    env.cli_ok(&["add", "crud", "--label", "hello", "--position", "right"]);
    let items = env.rpc("item.list", json!({})).unwrap();
    let item = find_item(&items, "crud").expect("added item");
    assert_eq!(item["label"], "hello");
    assert_eq!(item["position"], "right");

    env.cli_ok(&["set", "crud", "label=updated", "visible=false"]);
    let items = env.rpc("item.list", json!({})).unwrap();
    let item = find_item(&items, "crud").expect("updated item");
    assert_eq!(item["label"], "updated");
    assert_eq!(item["visible"], false);

    env.cli_ok(&["remove", "crud"]);
    let items = env.rpc("item.list", json!({})).unwrap();
    assert!(find_item(&items, "crud").is_none(), "item was not removed");

    env.write_config(
        r#"
[bar]
height = 30

[items.base]
label = "reloaded"

[items.extra]
label = "extra"
"#,
    );
    let report = env.rpc("config.reload", json!({})).unwrap();
    assert_eq!(report["reloaded"], true);
    assert_eq!(report["added_items"], 1, "reload report: {report}");
    assert_eq!(report["updated_items"], 2, "reload report: {report}");
    let items = env.rpc("item.list", json!({})).unwrap();
    assert_eq!(find_item(&items, "base").unwrap()["label"], "reloaded");
    assert!(find_item(&items, "extra").is_some());

    daemon.stop(&env);

    // A stale PID file must not block the next start. The daemon has to lock
    // the file, overwrite the old PID, and keep the new instance private.
    fs::write(&env.pid, "999999\n").unwrap();
    fs::set_permissions(&env.pid, fs::Permissions::from_mode(0o644)).unwrap();

    let mut restarted = env.start();
    restarted.wait_ready(&env);
    assert_eq!(file_mode(&env.pid), 0o600, "recovered PID permissions");
    let pid = fs::read_to_string(&env.pid).unwrap();
    assert_eq!(pid.trim().parse::<u32>().unwrap(), restarted.pid());
    restarted.stop(&env);
}

#[test]
fn config_reload_is_transactional() {
    let env = TestEnv::new();
    let mut daemon = env.start();
    daemon.wait_ready(&env);

    // A malformed file must be rejected without changing the live state.
    env.write_config(
        r#"
[bar]
height = 30

[items.base]
label = "unterminated
"#,
    );
    let output = env.cli(&["config", "reload"]);
    assert!(
        !output.status.success(),
        "invalid config unexpectedly reloaded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let items = env.rpc("item.list", json!({})).unwrap();
    let base = find_item(&items, "base").expect("existing item after failed reload");
    assert_eq!(base["label"], "base");
    assert!(
        find_item(&items, "partial").is_none(),
        "failed reload leaked a new item"
    );

    env.write_config(
        r#"
[bar]
height = 32

[items.base]
label = "new"

[items.partial]
label = "now valid"
"#,
    );
    env.cli_ok(&["config", "validate"]);
    let report = env.rpc("config.reload", json!({})).unwrap();
    assert_eq!(report["added_items"], 1);
    assert_eq!(report["updated_items"], 2);

    let items = env.rpc("item.list", json!({})).unwrap();
    assert_eq!(find_item(&items, "base").unwrap()["label"], "new");
    assert_eq!(find_item(&items, "partial").unwrap()["label"], "now valid");

    let status = env.rpc("system.status", json!({})).unwrap();
    assert_eq!(status["items"], 2);

    daemon.stop(&env);
}

#[test]
fn two_daemons_conflict_and_signals_clean_up() {
    let env = TestEnv::new();
    let mut daemon = env.start();
    daemon.wait_ready(&env);

    let mut second = env.command();
    second.args(["start", "--headless"]);
    let output = run_with_timeout(&mut second, Duration::from_secs(10));
    assert!(
        !output.status.success(),
        "a second daemon unexpectedly started"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("another FluxBar instance is running") || stderr.contains("failed to lock"),
        "unexpected second-daemon error:\n{stderr}"
    );
    assert_eq!(env.rpc("system.ping", json!({})).unwrap()["pong"], true);

    daemon.signal("TERM");
    let status = daemon.wait_for_exit(Duration::from_secs(10));
    assert!(status.success(), "SIGTERM exit status: {status}");
    daemon.assert_cleanup();

    let mut daemon = env.start();
    daemon.wait_ready(&env);
    daemon.signal("INT");
    let status = daemon.wait_for_exit(Duration::from_secs(10));
    assert!(status.success(), "SIGINT exit status: {status}");
    daemon.assert_cleanup();
}

#[test]
fn plugin_crash_is_isolated_and_restart_is_bounded() {
    let env = TestEnv::new();
    env.write_config(
        r#"
[bar]
height = 30

[items.base]
label = "base"

[plugins.crasher]
type = "external"
command = "/usr/bin/false"
auto_restart = true
max_restarts = 1
restart_delay = 1
"#,
    );
    let mut daemon = env.start();
    daemon.wait_ready(&env);

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let plugins = env.rpc("plugin.list", json!({})).unwrap();
        if let Some(plugin) = find_plugin(&plugins, "crasher")
            && plugin["state"] == "failed"
            && plugin["restarts"].as_u64().unwrap_or(0) >= 1
            && plugin["running"] == false
        {
            assert!(
                plugin["last_error"]
                    .as_str()
                    .is_some_and(|error| !error.is_empty()),
                "crashed plugin has no diagnostic: {plugin}"
            );
            break;
        }
        if Instant::now() >= deadline {
            panic!("plugin did not reach a bounded failed state: {plugins}");
        }
        sleep(Duration::from_millis(100));
    }

    // A crash loop is isolated from the daemon and its IPC surface.
    assert_eq!(env.rpc("system.ping", json!({})).unwrap()["pong"], true);
    let status = env.rpc("system.status", json!({})).unwrap();
    assert_eq!(status["plugins"], 0);

    env.write_config(
        r#"
[bar]
height = 30

[items.base]
label = "base"

[items.after_plugin]
label = "still alive"
"#,
    );
    let report = env.rpc("config.reload", json!({})).unwrap();
    assert_eq!(report["stopped_plugins"], 1, "reload report: {report}");
    let plugins = env.rpc("plugin.list", json!({})).unwrap();
    assert!(
        plugins.as_array().is_some_and(Vec::is_empty),
        "plugin was not removed: {plugins}"
    );
    let items = env.rpc("item.list", json!({})).unwrap();
    assert!(find_item(&items, "after_plugin").is_some());

    daemon.stop(&env);
}

#[cfg(target_os = "macos")]
#[test]
fn service_install_no_start_writes_private_launch_agent() {
    let env = TestEnv::new();
    let home = env.dir.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();

    let output = env
        .command()
        .env("HOME", &home)
        .args(["service", "install", "--no-start"])
        .output()
        .expect("run service install");
    assert!(
        output.status.success(),
        "service install failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let plist = home.join("Library/LaunchAgents/dev.fluxbar.fluxbar.plist");
    assert!(plist.exists(), "plist was not written");
    assert_eq!(file_mode(&plist), 0o644);
    assert_eq!(
        file_mode(&home.join("Library/Logs/FluxBar")),
        0o700,
        "log directory permissions"
    );
    assert_eq!(
        file_mode(&home.join("Library/Logs/FluxBar/fluxbar.log")),
        0o600,
        "log permissions"
    );
    let contents = fs::read_to_string(&plist).unwrap();
    assert!(contents.contains("<string>dev.fluxbar.fluxbar</string>"));
    assert!(contents.contains("<string>start</string>"));

    let lint = Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&plist)
        .output()
        .expect("run plutil");
    assert!(
        lint.status.success(),
        "plist is invalid:\n{}",
        String::from_utf8_lossy(&lint.stderr)
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn service_command_is_macos_only() {
    let env = TestEnv::new();
    let output = env.cli(&["service", "status"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("only available on macOS"),
        "unexpected error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn not_running_errors_include_remedies_and_stop_is_idempotent() {
    let env = TestEnv::new();

    let status = env.cli(&["status"]);
    assert!(!status.status.success());
    let stderr = String::from_utf8_lossy(&status.stderr);
    assert!(
        stderr.contains("cannot connect to FluxBar at")
            && stderr.contains("fluxbar start")
            && stderr.contains("--socket"),
        "unhelpful status error: {stderr}"
    );

    let stop = env.cli(&["stop"]);
    assert!(stop.status.success(), "idempotent stop failed: {stop:?}");
    let stdout = String::from_utf8_lossy(&stop.stdout);
    assert!(
        stdout.contains("not running") && stdout.contains("nothing to stop"),
        "unhelpful stop output: {stdout}"
    );
}

#[test]
fn invalid_config_reports_the_file_and_field() {
    let env = TestEnv::new();
    env.write_config("[bar]\nheight = 999\n");

    let output = env.cli(&["config", "validate"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&env.config.display().to_string()),
        "missing config path: {stderr}"
    );
    assert!(
        stderr.contains("bar.height must be between 0 and 256"),
        "missing field error: {stderr}"
    );
}

#[test]
fn missing_item_and_plugin_errors_name_the_object_and_remedy() {
    let env = TestEnv::new();
    let mut daemon = env.start();
    daemon.wait_ready(&env);

    let item = env.cli(&["remove", "does-not-exist"]);
    assert!(!item.status.success());
    let stderr = String::from_utf8_lossy(&item.stderr);
    assert!(
        stderr.contains("item not found")
            && stderr.contains("does-not-exist")
            && stderr.contains("fluxbar list")
            && stderr.contains("item.remove"),
        "unhelpful item error: {stderr}"
    );

    let plugin = env.cli(&["plugin", "restart", "does-not-exist"]);
    assert!(!plugin.status.success());
    let stderr = String::from_utf8_lossy(&plugin.stderr);
    assert!(
        stderr.contains("plugin not found")
            && stderr.contains("does-not-exist")
            && stderr.contains("fluxbar plugin list")
            && stderr.contains("plugin.restart"),
        "unhelpful plugin error: {stderr}"
    );

    daemon.stop(&env);
}

#[test]
fn color_flag_controls_error_styling() {
    let env = TestEnv::new();

    let always = env.cli(&["--color=always", "status"]);
    assert!(!always.status.success());
    assert!(String::from_utf8_lossy(&always.stderr).contains("\u{1b}[1;31m"));

    let never = env.cli(&["--color=never", "status"]);
    assert!(!never.status.success());
    assert!(!String::from_utf8_lossy(&never.stderr).contains("\u{1b}["));
}
