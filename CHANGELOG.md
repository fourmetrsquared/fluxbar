# Changelog

All notable changes to FluxBar are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
semantic versioning once a stable release is published.

## [Unreleased]

### Added

- A Homebrew formula with a launchd `service` block, so Homebrew users can run
  FluxBar at login with `brew services start fluxbar`. HEAD installs work
  before the first tagged release; the stable tap and checksum are added by
  the release process.
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
- Added dependency auditing with `cargo audit` and a private vulnerability
  reporting policy.
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
- Documented the plugin and script trust model.

## [0.1.0] - 2026-10-23

Initial FluxBar implementation: native macOS bar, JSON-RPC IPC, configurable
items, events and timers, plugin SDK/host, SketchyBar compatibility layer, and
the yabai/AeroSpace workspaces example.
