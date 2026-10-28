# Security policy

## Supported versions

FluxBar is currently in the `0.1.x` series. Security fixes are applied to the
latest release and to the default branch.

## Reporting a vulnerability

Please do not open a public issue for a suspected vulnerability. Use GitHub's
private vulnerability reporting for this repository:

1. Open **Security** on the repository page.
2. Choose **Report a vulnerability**.
3. Include the affected version, macOS version, reproduction steps, and any
   relevant logs or configuration (redact secrets).

You will receive an acknowledgement as soon as possible. Please allow time for
a fix and a coordinated disclosure before publishing details.

## Threat model

FluxBar is a desktop application, not a multi-user service.

- The daemon runs with the privileges of the logged-in user.
- The JSON-RPC control socket is the main trust boundary. It is created with
  mode `0600`, and the default cache directory is kept at mode `0700`.
- Any process running as the same user can control the bar. This is intentional:
  plugins, item scripts, and compatibility shims use the same socket.
- Plugins, build commands, item scripts, and mouse actions are **arbitrary code
  running as your user**. Install plugins only from sources you trust. A
  manifest's `build` command is executed with `/bin/sh -c` during install when
  building is enabled (`--no-build` skips it).
- FluxBar does not expose a network listener.

Do not point `--socket` at a world-writable directory such as `/tmp`. If you use
a custom path, place it in a directory that only your user can access.

## Hardening checklist

For an existing installation, make the configuration tree private:

```bash
chmod 700 ~/.config/fluxbar
chmod 600 ~/.config/fluxbar/fluxbar.toml
find ~/.config/fluxbar/plugins.d -type f -exec chmod 600 {} +
```

The default socket is created with mode `0600` by the daemon. Check it with:

```bash
stat -f '%Sp %N' ~/Library/Caches/dev.fluxbar.fluxbar/fluxbar.sock
stat -f '%Sp %N' ~/Library/Caches/dev.fluxbar.fluxbar/fluxbar.pid
```

A second daemon cannot start against the same socket: the PID file is locked
with an advisory lock, and a stale file left by a crash is detected and reused.
`SIGINT`, `SIGTERM` and `fluxbar stop` all stop timers, plugins and the UI
through the same shutdown path.

If you publish logs or bug reports, remove environment values, tokens, plugin
configuration, and personal paths first.

## Defensive measures in the codebase

The current code includes:

- bounded IPC messages and a maximum number of concurrent connections;
- idle-connection and external-command timeouts;
- bounded script, plugin, configuration, and manifest input sizes;
- validation of plugin names and entry-point paths;
- an IPC socket restricted to the current user;
- a locked, owner-only PID file next to the socket, with stale-file recovery;
- graceful `SIGINT`/`SIGTERM` shutdown;
- private permissions for new configuration directories and files;
- dependency auditing in CI with `cargo audit`;
- no `unsafe` code in the platform-independent crates. The native macOS
  renderer contains the AppKit/Objective-C interop, documented at each call
  site.

## Dependency audits

Run the same checks locally:

```bash
cargo audit
cargo audit --file examples/plugins/hello/Cargo.lock
cargo audit --file examples/plugins/workspaces/Cargo.lock
```

If an advisory affects an indirect dependency, update `Cargo.lock` or open an
issue with the affected path and the fixed version.
