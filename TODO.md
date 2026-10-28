# FluxBar TODO

Living backlog for FluxBar, covering bug fixes, missing functionality, and
architecture work. Items are ordered by priority, not by effort.

| Priority | Meaning |
| --- | --- |
| **P0** | Correctness, security, or data-loss issues. Block the next release. |
| **P1** | Required v1.x functionality and high-value user-facing improvements. |
| **P2** | Modernization, performance, packaging, and long-term architecture. |
| **P3** | Polish, documentation, and non-blocking quality work. |

Tags: `Fix`, `Feature`, `Refactor`, `Test`, `Docs`, `Chore`.

Unless stated otherwise, every item is done when:

- behavior is covered by a unit or integration test where technically possible;
- `./build.sh check` passes;
- `docs/CONFIGURATION.md`, `docs/PLUGIN_GUIDE.md`, and `docs/STATUS.md` are
  updated if the public API or configuration changes;
- configuration remains backward compatible, or a migration is documented.

## Roadmap at a glance

| Target | Theme | Exit criteria |
| --- | --- | --- |
| **v1.0.x** | Hardening | All P0 items closed; no known crash, socket lifecycle, or config-reload data loss. |
| **v1.1** | Feature completion | Menu-bar aliases, absolute positioning, brightness, themes, rich mouse actions, UDS plugins, multi-display, complete SketchyBar parity. |
| **v2** | Platform and sandbox | Wasm plugins, stable renderer abstraction, Linux/Windows backends, formal performance and security posture. |

## v1.0.x hardening status

P0-01 through P0-06 are complete. The main remaining release-quality task is
P3-01, the automated daemon integration harness; the current P0 regressions are
covered by unit/integration tests plus the manual checks in
[docs/RELEASE.md](docs/RELEASE.md).

---

## P0 — Correctness and security

### [x] P0-01 — Honor `bar.top_margin` and unify the coordinate model (`Fix`, macOS)

- Implemented `bar.top_margin` for top and bottom placement and added
  `bar.horizontal_margin` for a centered floating capsule.
- Layout owns screen-space `bar_bounds`; `MacApp` converts the origin to the
  AppKit window frame and keeps item painting/hit-testing local to the window.
- Also fixed the renderer accidentally replacing the screen size with the bar
  size after the first frame.
- Acceptance covered by layout tests for top/bottom margins, floating insets,
  capped centered bars, and renderer screen-size retention.

### [x] P0-02 — Replace text-width guessing with real font measurement (`Fix`, macOS)

- Added the `TextMetrics` hook to core layout. `layout()` keeps the heuristic
  fallback for headless/tests, while `layout_with_metrics()` accepts the
  platform measurer.
- `MacRenderer` now measures with CoreText using the actual resolved `NSFont`
  and caches `(text, font, size)` results. The daemon uses those exact widths
  for item layout, so painted content and hit boxes share the same geometry.
- Painting clamps content to the item's padded area, shrinks the label before
  the icon, and uses `NSParagraphStyle` tail truncation so fixed/`max_width`
  items get an ellipsis instead of overflowing.
- Added tests for ASCII/wide Latin, CJK, emoji, combining marks, very long
  labels, metric-aware layout, cache reuse and width clamping.

### [x] P0-03 — Fix hardcoded progress rendering (`Fix`, macOS)

- Added `progress_color`, `progress_track_color`, `progress_height` and
  `progress_position` (`bottom`/`top`/`background`) to item config. The fill
  defaults to `label_color`; the track defaults to a 16% translucent fill.
- `top`/`bottom` reserve a dedicated strip and inset the content rect for the
  label, icon and graph, so all three can be used with progress simultaneously.
  `background` paints a full-height fill behind the content.
- Removed hardcoded white/height/position values, interpolate progress colors
  during frame animations, and clamp non-finite/negative/out-of-range values.
- SketchyBar `progress.color`, `progress.track_color`, `progress.height` and
  `progress.position` map to the new fields. Docs and tests cover every
  position, content/no-overlap geometry, color defaults, opacity and NaN.

### [x] P0-04 — Make configuration reload transactional and complete (`Fix`, core/CLI)

- `RuntimeReload::reload()` parses and validates the complete new file before
  touching live state. A failed load leaves the previous configuration active.
- Item ownership is explicit (`ItemOrigin::Config`/`Runtime`). Config-owned
  items are added, updated (`kind`, `position`, `visible`, static label/icon,
  inherited styles) or removed; runtime-created items survive reloads.
- Plugin specs are reconciled by value: new plugins start, changed specs
  restart, removed plugins stop, unchanged processes keep running.
- Timers are rebuilt, the system monitor restarts only when `[system]` changes,
  and bar geometry/visibility are applied from the new config.
- `config.reload` returns a report (`added_items`, `updated_items`,
  `removed_items`, `started_plugins`, `stopped_plugins`, `timers`,
  `monitor_restarted`) and `fluxbar config reload` prints it.
- Integration tests cover item add/update/remove, runtime ownership, timer and
  monitor changes, bar visibility, plugin add/change/remove, and host restart
  after `stop_all`.

### [x] P0-05 — Harden daemon lifecycle, signals, and socket handling (`Fix`, core/CLI)

- Added `SIGINT`/`SIGTERM` handlers that flip the daemon shutdown flag; the
  event loop then stops timers, the monitor, scripts, plugins and the UI through
  the same path as `fluxbar stop`.
- Added an advisory-locked PID file next to the socket, created `0600`. A
  second daemon with the same socket is refused; a stale PID file left after a
  crash is reused. The PID file is removed on normal exit.
- `IpcServer::bind()` probes a live socket and refuses to unlink it; stale
  sockets are recovered. The default runtime directory is `0700` and the socket
  is `0600`. `IpcServer` has an RAII `Drop` guard that only removes a socket
  file.
- Tests cover PID cleanup, duplicate-instance refusal, stale PID reuse,
  symlink rejection, and live/stale socket handling. Manual checks cover
  `SIGINT`, `SIGTERM` and `fluxbar stop`.

### [x] P0-06 — Correct plugin lifecycle semantics (`Fix`, core)

- Each plugin now has its own stop watch and direct-event channel.
  `PluginHost::notify(name, event)` delivers only to that plugin and bypasses
  subscriptions; removed plugins stop and the host remains restartable.
- A failed `init` is treated as a startup failure. It records `last_error`,
  follows `auto_restart`/`max_restarts`, and never leaves the plugin marked
  running.
- `max_restarts` counts consecutive crashes. Both the counter and backoff reset
  after 60 seconds of stable operation.
- `stop`/`restart` send the JSON-RPC `shutdown` request and wait up to five
  seconds before aborting. `shutdown_all()` is used on daemon exit;
  `stop_all()` only signals tasks so synchronous reconcile paths do not block.
- `plugin.list` exposes `starting`, `running`, `backoff`, `stopped` and
  `failed` states plus `last_error`; the CLI table prints them.
- Tests cover targeted direct events, host restart after `stop_all`, and
  plugin add/remove/change reconciliation.

---

## P1 — Missing functionality and user-facing improvements

### [ ] P1-01 — Native menu-bar aliases (FR-1.3) (`Feature`, macOS/config)

- Add an `NSStatusItem` that mirrors selected native menu-bar extras (Wi-Fi,
  Bluetooth, MeetingBar, etc.) or exposes a FluxBar menu.
- Define config for alias source/window id, title/icon fallback, click action,
  and menu contents. Capture via Accessibility APIs only when necessary and
  document required permissions.
- Ensure the alias does not steal focus from the bar and follows light/dark
  appearance.
- Acceptance: aliases can be configured, persist across restarts, and degrade
  to a static icon when capture is unavailable; docs cover permissions.

### [ ] P1-02 — Absolute positioning and global stacking (FR-1.2) (`Feature`, core)

- Add item `x` coordinates, anchor (`left`/`center`/`right`/absolute), and a
  global `z_index`. `drawing_order` currently only sorts items inside one
  group and does not control overlap between groups.
- Define collision behavior; allow an item to be pinned independently of the
  left/center/right flow.
- Acceptance: absolute items land at exact points for both bar positions;
  z-index controls paint and hit-test order; layout tests cover overlaps,
  negative positions, and bounds clamping.

### [ ] P1-03 — Make layout robust under pressure (`Fix`, core/macOS)

- Left and right groups can overlap the center group; items can overflow the
  bar; negative computed widths are possible with extreme padding.
- Add an overflow policy: shrink/truncate low-priority items, hide overflow
  with a configurable indicator, or clip deterministically.
- Non-full-width bars are now centered and `horizontal_margin` controls the
  floating inset; an optional `max_width` is still pending.
- Acceptance: 100-item and zero-width stress tests produce no overlap outside
  the chosen policy; center truncation is predictable.

### [ ] P1-04 — Add brightness events (FR-2.1) (`Feature`, system)

- `brightness_change` exists as an event kind but has no poller.
- Add configurable polling via IOKit/DisplayServices where available, with a
  documented CLI fallback for external displays. Respect an opt-out and
  per-display scope.
- Emit changed values only; update `brightness` items and expose the value in
  payloads.
- Acceptance: brightness changes produce one event per actual change; no
  polling when disabled; failure degrades silently with a debug log; docs
  describe API/permission limitations.

### [x] P1-05 — Implement script-backed items (`Feature`, core/CLI)

- Implemented `script`, `subscribe`, `update_interval` and `click_script` for
  items, with a 60-second timeout, concurrent-run protection, stderr logging,
  JSON output parsing and `sketchybar`/`fluxbar` shims on `PATH`.
- Plain stdout is intentionally ignored; scripts emit JSON or call the
  compatibility CLI.
- Remaining polish: configurable timeout and richer per-script error state.

### [ ] P1-06 — Rich mouse bindings and plugin actions (FR-4.4) (`Feature`, core/macOS)

- `click_action` handles both left and right clicks; there is no separate
  right/middle binding, and `plugin`-type actions only log a warning.
- Add `right_click_action`, `middle_click_action`, and a `plugin` action that
  sends a targeted JSON-RPC notification (or event) to the owning plugin.
- Forward button, click count, and scroll delta as action payload fields;
  define scroll threshold/accumulation behavior.
- Acceptance: left/right/middle each run their own binding; plugin actions
  reach only the target plugin; mouse events and action payloads are covered
  by tests and documented.

### [ ] P1-07 — Implement theme files (`Feature`, core)

- `Config::theme` is parsed but unused.
- Load a theme TOML with named colors/palette entries, resolve tokens in every
  color field, and support theme inheritance/overrides through includes.
- Define precedence: item > config > theme > built-in default.
- Add `fluxbar config validate` checks for unknown tokens, cycles, and
  unresolved references.
- Acceptance: themes work without changing existing configs; validation
  produces actionable errors; docs include a complete themed example.

### [ ] P1-08 — UDS plugin transport (FR-3.1) (`Feature`, core)

- `PluginType::Uds` is accepted but not started. Define whether the plugin or
  host owns the socket, the handshake, reconnect/backoff behavior, and frame
  limits.
- Reuse the stdio JSON-RPC codec to avoid a second protocol implementation.
- Add a mock UDS plugin test and document that Wasm remains v2.
- Acceptance: a UDS plugin passes the same lifecycle, subscription, update,
  crash, and restart tests as a stdio plugin.

### [ ] P1-09 — Multi-display support (`Feature`, macOS/config)

- `bar.display` is parsed but ignored; only `NSScreen::mainScreen()` is used.
- Support selection by index, localized name, or stable display UUID; define
  one bar total versus one bar per display. Handle hot-plug, resolution,
  scale, notch, and menu-bar visibility changes.
- Move screen geometry behind a testable abstraction.
- Acceptance: the bar appears on the selected display and follows display
  changes; configuration is deterministic when displays are added/removed;
  docs explain identifiers and fallback behavior.

### [x] P1-10 — Blur and translucent backgrounds (`Feature`, macOS/config)

- Implemented `bar.opacity`, `bar.blur_radius`, and per-item
  `opacity`, `background_blur_radius` and `background_shape`.
- On macOS 26, blurred surfaces use Apple's `NSGlassEffectView` for real
  Liquid Glass edge refraction; the tinted interior is inset so the native
  lensing remains visible. Older releases use `NSVisualEffectView` plus the
  painted sheen/rim fallback.
- The renderer splits the bar into backdrop, item-blur and content layers so
  item vibrancy sits above the bar background and below text/icons.
- macOS exposes semantic materials rather than exact radii; the configured
  radius selects an intensity tier (`1–8`, `9–24`, `>24`) on the fallback path.
- Incremental damage regions and native-glass state caching avoid repainting
  unchanged items and reapplying unchanged glass properties during spring
  transitions.
- Remaining: add an explicit reduced-transparency fallback and test
  high-contrast mode (tracked under P2-05/P3-04). Formal performance budgets
  remain under P2-05.

### [ ] P1-11 — Graph modes and history (`Feature`, core/macOS)

- FR-4.3 asks for line and bar charts. Only a line sparkline is implemented,
  with the graph color tied to the label color and no history management.
- Add `graph_mode` (`line`/`bar`), `graph_color`, optional min/max/fixed scale,
  and a bounded history buffer with a configurable sample count.
- Keep the existing `graph = [...]` IPC contract; add append semantics
  without breaking full replacements.
- Acceptance: line/bar render identically from the same data; colors are
  independent; history is bounded; tests cover single-point and flat data.

### [ ] P1-12 — Built-in item polish (`Feature`, core)

- Battery charging state is read and discarded; volume mute state is not
  represented; media has only artist/title; progress values duplicate the
  label.
- Add charging/muted indicators and optional status text, and avoid replacing
  user labels unless the item opts in.
- Add a configurable time/date format and locale-aware clock instead of the
  hardcoded `%H:%M`.
- Acceptance: charging/muted states render distinctly; built-in labels can be
  formatted without plugins; unit tests cover all state combinations.

### [ ] P1-13 — Complete the animation model (`Feature`, macOS)

- New items snap into place; removed items disappear instantly; label/icon
  text changes do not fade; background/border transitions from `None` to a
  color snap.
- Animate enter/exit opacity and position, cross-fade text, and interpolate
  optional colors from/to transparent.
- Respect a reduced-motion setting and avoid animating hidden/destroyed views.
- Acceptance: add/remove/update all animate smoothly with no ghost frames;
  animation math is unit-tested; disabling animations restores exact layout.

### [ ] P1-14 — CLI parity and discoverability (`Feature`, CLI)

- `OutputFormat` in `fluxbar-cli/src/commands.rs` is unused; commands print
  ad-hoc text and cannot be consumed reliably by scripts.
- Add `--json` to status/list/get/config show, a `get` command, `--quiet`,
  stable exit codes, and machine-readable errors.
- Add shell completions (`clap_complete`), man pages, and a richer `doctor`
  (config parse, socket owner/permissions, stale PID, plugin entry points,
  missing fonts, Accessibility/Automation permissions).
- Acceptance: every read command supports JSON; completions/man pages are
  generated in CI; `doctor` catches the most common setup failures.

### [ ] P1-15 — Reproducible plugin installation (`Feature`, CLI)

- `plugin install` always clones the default branch and has no lockfile,
  `--ref`, or integrity check. `plugin update` uses `git pull`.
- Support `--ref/--tag/--branch`, record the resolved commit in a lockfile,
  add `--frozen`/offline installs, and update all installed plugins
  deterministically. Verify local manifests before writing registrations.
- Acceptance: install/update is reproducible from the lockfile; a missing or
  moved ref fails cleanly; tests cover git and local sources.

### [ ] P1-16 — Complete SketchyBar compatibility (`Feature`, CLI/macOS)

- `--add alias` and `--add bracket` are accepted but do not create native
  menu-bar aliases or grouped bracket items.
- Popup, shadow, image, background image and native `space` selected-state
  properties are parsed/ignored rather than rendered.
- `--add space` creates a custom item only; it cannot observe or switch native
  Mission Control spaces.
- Add the missing item types or explicitly reject them with actionable errors;
  keep unknown properties warning-only for script portability.
- Acceptance: the documented supported surface matches behavior, unsupported
  properties are covered by tests, and the parity table is updated.

### [ ] P1-17 — Workspace backends and app-rule ergonomics (`Feature`, examples)

- The bundled workspaces plugin supports yabai and AeroSpace with automatic
  backend detection.
- Add a command-template backend for other window managers.
- yabai app rules are not implemented yet; AeroSpace rules remain the only
  managed-window path.
- Offer optional rule grouping (bundle-id prefixes, app categories) while
  keeping explicit user rules authoritative.
- Acceptance: a command-template backend passes the headless E2E harness;
  documentation states polling latency and permission requirements.

---

## P2 — Modernization and long-term architecture

### [ ] P2-01 — Version the config schema and strengthen validation (`Refactor`, core)

- Add `config_version`, migrations, recursive includes with cycle detection,
  path canonicalization, and range validation for numeric fields (height,
  padding, intervals, radii).
- Generate a JSON Schema from the serde model and expose
  `fluxbar config schema`.
- Reject unsupported `PluginType` values at validation time with a clear
  "reserved for v2" error instead of silently not starting them.
- Acceptance: old configs load through migrations; invalid values report file
  and field; schema validates the shipped examples in CI.

### [ ] P2-02 — Evolve the IPC protocol (`Refactor`, core)

- Add protocol version negotiation, maximum frame size, read/write timeouts,
  per-connection concurrency limits, proper batch/notification support, and
  optional authentication for the socket.
- Publish an OpenRPC/JSON schema document generated from the handler metadata.
- Keep v1 clients working for at least one minor release.
- Acceptance: malformed/oversized/slow clients cannot exhaust the daemon;
  a generated API document matches `rpc.discover`; fuzz tests cover decoding.

### [ ] P2-03 — Improve plugin protocol throughput and observability (`Refactor`, core/SDK)

- The host waits up to 10 seconds per request and a slow plugin stalls its
  own event stream; the broadcast bus can drop events under load.
- Support request pipelines with correlated IDs, cancellation, coalesced
  `tick` events, heartbeat/health checks, and structured log-level forwarding.
- Define backpressure explicitly (drop oldest, coalesce, or disconnect).
- Acceptance: a slow plugin cannot delay other plugins; dropped events are
  counted and logged; SDK and host protocol versions are negotiated.

### [ ] P2-04 — Mature the renderer abstraction (NFR-4) (`Refactor`, core/macOS)

- Move text measurement, font resolution, screen queries, and image loading
  behind renderer traits so core layout is platform-independent and testable.
- Frame damage regions are implemented for the macOS content view; caching
  text/shape resources and separating layout from painting remain.
- Add headless snapshot/golden tests for bar/items/progress/graphs.
- Acceptance: core has no AppKit types; a second dummy renderer can render
  the same frame; golden tests catch visual regressions.

### [ ] P2-05 — Enforce performance budgets (NFR-1) (`Test`, core/macOS)

- The UI loop currently wakes every 16 ms; the clock task ticks every second
  even though the displayed minute rarely changes; the system monitor polls
  regardless of which item kinds/plugins exist.
- Schedule the clock to the next minute boundary, poll system sources only
  when subscribed/needed, and wake the UI from events or a display link
  instead of a fixed polling loop.
- Add Criterion benchmarks and a CI smoke check for idle CPU and end-to-end
  event latency (< 16 ms), with a documented methodology.
- Acceptance: measurements are published in `docs/STATUS.md`; idle CPU stays
  below 0.5% on the reference machine; no busy loop or per-frame allocation
  regression.

### [ ] P2-06 — Add production observability (`Feature`, core/CLI)

- Add optional file logging with rotation, `RUST_LOG` filtering, tracing spans
  for IPC/plugin/config operations, counters (events, dropped events, frames,
  restarts), and a debug overlay for layout bounds.
- Forward plugin stderr with plugin name/level instead of treating every line
  as a warning.
- Acceptance: logs can be collected without a terminal; `fluxbar status
  --json` exposes counters; a debug flag produces actionable diagnostics.

### [ ] P2-07 — Build release and packaging infrastructure (`Chore`, repo)

- Add GitHub Actions for fmt, clippy, tests, example plugin, docs, and
  `cargo-deny`/`cargo-audit`; cache Cargo artifacts. (CI and `cargo-audit` are
  now in `.github/workflows`; `cargo-deny` and docs-link checks remain.)
- Produce universal macOS release binaries, checksums, a changelog, and
  signed/notarized artifacts where credentials exist.
- Add a Homebrew tap/formula, `cargo install` guidance, and a proper uninstall
  path. A tap-ready formula with a `brew services` launchd definition is now
  in `Formula/fluxbar.rb`; HEAD installs work before the first tag. The stable
  URL/checksum, published tap, and clean-machine verification remain.
- Acceptance: a tagged release builds reproducibly from CI; installation
  instructions are verified on a clean machine; `main` is protected by checks.

### [ ] P2-08 — Launch-at-login service integration (`Feature`, CLI/macOS)

- Add `fluxbar service install|uninstall|status` using a LaunchAgent plist,
  with log paths, restart policy, and clean removal.
- Detect an already-running daemon and avoid duplicate agents.
- Homebrew users can already use the formula's `brew services` definition;
  this item covers source installs without Homebrew.
- Acceptance: login starts exactly one daemon; uninstall removes the plist,
  socket, and optional logs; docs cover manual and service modes.

### [ ] P2-09 — Stabilize the SDK and protocol contract (`Refactor`, SDK)

- Define a semver policy for `fluxbar-sdk`, re-export stable protocol types,
  document docs.rs builds, and publish to crates.io when ready.
- Add protocol version negotiation, typed errors, and examples for async and
  blocking plugins.
- Acceptance: an external plugin can build against the published crate;
  breaking protocol changes require a major SDK version; docs links are
  checked in CI.

### [ ] P2-10 — Dependency, MSRV, and unsafe-code policy (`Chore`, repo)

- Document the MSRV (currently Rust 1.85/edition 2024), enforce it in CI, and
  add `cargo-deny` for licenses/advisories and `cargo-machete` for unused
  dependencies.
- Audit the `objc2` unsafe blocks, document their invariants, and add
  `#[deny(unsafe_op_in_unsafe_fn)]` where practical.
- Acceptance: CI fails on advisories/license violations/MSRV regression;
  unsafe code has a short written safety argument.

### [ ] P2-11 — Cross-platform renderer backends (NFR-4) (`Feature`, platform)

- Implement Linux (Wayland layer-shell or GTK) and Windows (AppBar) backends
  behind the P2-04 traits; keep the headless backend for CI.
- Abstract platform system events (workspaces, media, power) with capability
  discovery and no-op fallbacks.
- Acceptance: the workspace builds and the headless test suite passes on all
  three OSes; Linux/Windows bars support the documented core feature set.

### [ ] P2-12 — Wasm plugin runtime (NFR-3, v2) (`Feature`, core)

- Embed a Wasm runtime (e.g. wasmtime) with fuel/time limits, bounded memory,
  no filesystem/network access by default, and explicit capability grants.
- Reuse the JSON-RPC contract through host functions; add a WAT/Rust test
  plugin, crash/limit tests, and a migration path for stdio plugins.
- Acceptance: a malicious/looping plugin cannot block or crash the daemon;
  capabilities are deny-by-default; docs explain packaging and limits.

---

## P3 — Quality, documentation, and polish

### [ ] P3-01 — Add a daemon integration harness (`Test`, repo)

- Spawn the real daemon headless with a temporary config/socket and exercise
  start, ping, item CRUD, config reload, plugin crash/restart, `SIGTERM`,
  `SIGINT`, and `stop`.
- Cover socket cleanup, PID locking, permissions, and two-daemon conflicts
  introduced by P0-05.
- Acceptance: the suite runs in CI on macOS and Linux; each P0 regression has
  at least one end-to-end test.

### [ ] P3-02 — Expand automated test coverage (`Test`, repo)

- Add property tests for config deep-merge/includes and JSON-RPC
  serialization, fuzz tests for IPC/plugin decode paths, snapshot tests for
  layout/animations, and tests for git/local plugin installs.
- Measure coverage and set a realistic floor for core (not a vanity target).
- Acceptance: malformed input is covered by fuzzing; merge/order semantics
  are property-tested; plugin installer paths are exercised.

### [ ] P3-03 — Refresh documentation and status (`Docs`, repo)

- Keep `docs/STATUS.md`, `docs/CONFIGURATION.md`, `docs/PLUGIN_GUIDE.md` and
  the known-limitations list in sync with the implementation (including the
  distinction between per-group `drawing_order` and a future global z-index).
- Add an architecture overview, an IPC/API reference, a plugin migration
  guide, a troubleshooting/permissions page, and a configuration cookbook.
- Acceptance: every FR/NFR maps to code or a TODO ID; docs build/link checks
  pass; no undocumented reserved keys remain.

### [ ] P3-04 — Accessibility and display adaptation (`Feature`, macOS)

- Add VoiceOver labels/roles for items, respect Reduce Motion, Reduce
  Transparency and Increase Contrast, verify scaling on Retina/non-Retina and
  external displays, and provide keyboard focus where practical.
- Acceptance: VoiceOver can identify item labels and actions; reduced motion
  disables transitions; rendering is crisp at every tested scale.

### [ ] P3-05 — Configuration editor support (`Feature`, tooling)

- Ship the generated JSON Schema, add a `#:schema` directive to
  `config init`, and document VS Code/Helix/Neovim setup.
- Add `fluxbar config edit`/`config path --json` conveniences.
- Acceptance: editors validate keys and types out of the box; the schema is
  versioned with the binary.

### [x] P3-06 — Repository and community hygiene (`Chore`, repo)

- Added `CONTRIBUTING.md`, `SECURITY.md`, a contributor covenant, issue/PR
  templates, a changelog, third-party notices, and `docs/RELEASE.md`.
- Added macOS/Linux CI plus scheduled dependency auditing and Dependabot.
- Supported versions and the plugin trust model are documented; security
  reports use GitHub private vulnerability reporting.
- Acceptance: a new contributor can build, test, and submit a plugin without
  reading source; security reports have a private channel.

### [ ] P3-07 — UX and error-message polish (`Fix`, CLI/daemon)

- Replace generic RPC/config errors with actionable messages that include the
  method, item/plugin name, config file, and field. Honor `NO_COLOR` and add
  `--color=auto|always|never`.
- Ensure failed shell/plugin actions are visible at an appropriate log level.
- Acceptance: common failures (not running, bad socket, bad config, missing
  plugin, stale PID) each have a clear one-line remedy.

### [ ] P3-08 — Grow the examples and presets (`Docs`, examples)

- Add a theme showcase, a media plugin, a multi-display config, and a minimal
  launchd setup. The workspace plugin and script-item examples already exist.
  Validate every example in CI with `fluxbar config validate`.
- Acceptance: examples cover every public feature and stay buildable.

---

## Explicit non-goals for v1.x

- Wasm sandboxing and capability permissions (v2, tracked as P2-12).
- Linux/Windows feature parity (architecture work tracked as P2-04/P2-11).
- Running as a privileged/system-wide daemon; FluxBar remains per-user.
- Loading arbitrary native code into the daemon process; plugins stay
  out-of-process.
