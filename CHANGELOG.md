# Changelog

All notable changes to FluxBar are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
semantic versioning once a stable release is published.

## [Unreleased]

## [0.1.0] - 2026-10-30

Initial public release: native macOS bar, JSON-RPC IPC, configurable items,
events and timers, plugin SDK/host, SketchyBar compatibility layer, and the
yabai/AeroSpace workspaces example.

### Added

- A Homebrew formula with a launchd `service` block, so Homebrew users can run
  FluxBar at login with `brew services start fluxbar`. HEAD installs work
  before the first tagged release; the stable tap and checksum are added by
  the release process.
- A tag-triggered release workflow that verifies the source, builds a
  universal macOS binary, publishes checksums and release notes, and renders
  a stable Homebrew formula. Developer ID signing and Homebrew tap updates are
  enabled when the corresponding repository secrets are configured.
- `scripts/release.sh` and `./build.sh dist`, which build the same universal
  release artifacts locally, plus `./build.sh docs` for Markdown link checks
  and `./build.sh uninstall --yes` for source installations.
- A documented Rust 1.89 MSRV with a dedicated CI check.
- `fluxbar config reload`, which reports item/plugin/timer counts and returns
  configuration errors directly instead of requiring the lower-level
  `fluxbar trigger config_reload` event.
- Configurable progress fill/track colors, height and `bottom`/`top`/
  `background` placement, with reserved content space and progress color
  animation.
- Transactional, complete `config.reload`: config-owned item add/update/
  remove, runtime-item preservation, plugin spec reconciliation, timer rebuild,
  selective monitor restart, and bar visibility updates.
- Graceful `SIGINT`/`SIGTERM` shutdown and a locked, owner-only PID file with
  stale-process recovery.
- A process-level daemon integration suite covering IPC startup, item CRUD,
  transactional reload, plugin crash isolation, signals, permissions, stale
  PID recovery, and two-daemon conflicts.
- `fluxbar service install|status|uninstall` for source installs, with a
  per-user launchd agent, crash-only restarts, private logs, and cleanup of the
  plist, socket, PID file, and optional logs.
- Actionable CLI diagnostics: connection failures name the socket and suggest
  `fluxbar start`/`--socket`; item and plugin errors name the RPC method and
  point at `fluxbar list`/`fluxbar plugin list`; config validation includes the
  file and field; `fluxbar stop` is idempotent when nothing is running.
- `--color=auto|always|never` with `NO_COLOR` support for error output.
- Optional size-rotated file logging (`--log-file`, `--log-max-bytes`,
  `--log-keep`) with owner-only permissions and `RUST_LOG`/`-v` filtering.
- `fluxbar status --json` counters for events, dropped deliveries, frames,
  plugin restarts, IPC requests, and IPC errors. IPC/config reload spans,
  plugin lifecycle spans, and level-aware plugin stderr forwarding.
- `FLUXBAR_DEBUG_LAYOUT=1` draws red bar/item bounds and logs layout rectangles
  at debug level for clipping diagnostics.
- Explicit plugin lifecycle states (`starting`, `running`, `backoff`,
  `stopped`, `failed`), targeted direct notifications, graceful stop/restart,
  consecutive restart accounting, and init-failure recovery.

### Security

- Restricted the default IPC socket to the current user (`0600`) and hardened
  the default socket cache directory (`0700`).
- Added bounded IPC frames, a concurrent-connection limit, idle connection
  timeouts, and bounded plugin/script/configuration/manifest reads.
- Added output and runtime limits for external system probes and scripts.
- Validated plugin names and manifest entry paths against traversal and empty
  path components.
- Added private permissions for newly created configuration directories and
  files.
- Added dependency auditing with `cargo audit`, a `cargo-deny` license/source
  policy, and `cargo-machete` unused-dependency checks, plus a private
  vulnerability reporting policy.
- Removed unused direct dependencies and added explicit versions to workspace
  path dependencies; documented the unsafe-code boundary in `docs/SAFETY.md`.
- Added a locked PID file and graceful signal handling to prevent duplicate
  daemons and abrupt plugin/socket teardown.

### Fixed

- Progress is no longer hardcoded white/3 pt and no longer paints over labels,
  icons or graphs.
- Config reload no longer leaves deleted items on the bar, ignores item
  kind/position changes, or keeps removed/changed plugins running.
- Plugin init failures no longer leave broken plugins marked running;
  restart limits now count consecutive failures and reset after stable uptime.
- `plugin.stop` and `plugin.restart` now perform a graceful shutdown handshake.

### Documentation

- Added contribution, security, conduct, and third-party attribution policies.
- Added a Russian end-to-end guide, configuration reference, plugin guide, and
  release checklist.
- Documented the plugin and script trust model, Homebrew service usage, and the
  release artifact and signing workflow.
