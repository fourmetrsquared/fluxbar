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
| **v1.1** | Feature completion | Menu-bar aliases, absolute positioning, script items, brightness, themes, rich mouse actions, UDS plugins, multi-display, blur. |
| **v2** | Platform and sandbox | Wasm plugins, stable renderer abstraction, Linux/Windows backends, formal performance and security posture. |

## Suggested first slice (v1.0.x)

1. P0-01 vertical placement and `top_margin`.
2. P0-02 text measurement and truncation.
3. P0-03 progress rendering.
4. P0-04 transactional config reload.
5. P0-05 process lifecycle and socket safety.
6. P0-06 plugin lifecycle correctness.
7. Add the daemon integration harness from P3-01 while fixing the above.

---

## P0 — Correctness and security

### [ ] P0-01 — Honor `bar.top_margin` and unify the coordinate model (`Fix`, macOS)

- `layout()` adds `top_margin` to `bar_bounds`, but `MacApp::place()` always pins
  the window to the visible screen edge. Item painting then subtracts
  `bar_bounds`, so the configured margin effectively disappears.
- Bottom placement and the notch area need the same treatment.
- Decide on one model: layout produces screen coordinates; the platform layer
  converts them to window origin and local drawing coordinates.
- Acceptance: `top_margin` visibly moves the window for top and bottom bars;
  items remain aligned; negative/zero/large values are validated; layout tests
  cover top, bottom, notch-sized margins, and hidden menu bar.

### [ ] P0-02 — Replace text-width guessing with real font measurement (`Fix`, macOS)

- `measure_text()` in `fluxbar-macos/src/app.rs` calls
  `sizeWithAttributes(None)` and explicitly ignores the supplied `NSFont`, so
  all text is measured with the wrong font.
- Core layout uses `estimate_text_width()`, while the renderer measures again
  with AppKit. The two widths disagree, causing clipped labels, overlapping
  items, and incorrect hit boxes.
- Measure with the actual `NSFont`/`NSColor` attributes, cache results by
  `(text, font, size)`, and feed measured widths back into layout (renderer
  measurement callback or a post-layout adjustment pass). Apply `max_width`
  with an ellipsis instead of allowing overflow.
- Test ASCII, wide Latin, CJK, emoji, combining marks, and very long labels.
- Acceptance: painted content stays inside item bounds within 1 point; hit
  testing matches painted pixels; truncation works for fixed/max widths.

### [ ] P0-03 — Fix hardcoded progress rendering (`Fix`, macOS)

- `paint_progress()` always uses white, ignores item styling, reads
  `frame.items.first()` for no reason, and paints over the graph and label
  because no content region is reserved.
- Add backward-compatible item options such as `progress_color`,
  `progress_track_color`, `progress_height`, and `progress_position`
  (`bottom`, `top`, `background`). Default to the label color if unset.
- Reserve a dedicated strip that does not overlap the icon, label, or graph.
  Progress and graph must be usable simultaneously.
- Clamp and test 0, 100, NaN, negative, and out-of-range values.
- Acceptance: no hardcoded colors; progress and content never overlap; config
  docs and layout/renderer tests cover all positions.

### [ ] P0-04 — Make configuration reload transactional and complete (`Fix`, core/CLI)

Current `reload_config()` only adds new items and shallow-merges existing ones.
It does not reconcile the rest of the running system:

- existing item `kind`/`position` changes are ignored;
- changed static labels/icons are written to `item.config` but not to
  `item.state`, so the bar keeps showing the old text;
- items removed from config stay on the bar forever;
- removed plugins keep running; changed plugin specs are ignored because
  `start_all()` skips running names;
- timers are not recreated; system-monitor settings and `bar.hidden` are not
  applied; `merge_item_config()` preserves only label/icon/background and can
  lose intended resets.
- Implement a real reconcile pass with explicit ownership (`config`, `CLI`,
  `plugin`): add/update/remove config-owned items, preserve runtime-created
  items, stop/restart changed plugins, remove deleted plugins, rebuild timers,
  restart only the affected monitor tasks, and apply bar visibility/geometry.
- Validate and build the complete new config before mutating live state, so a
  failed reload leaves the previous configuration active.
- Acceptance: a single `fluxbar trigger config_reload` or `config.reload`
  applies label/kind/position/plugin/timer/monitor/bar changes and removes
  deleted config entries; runtime-only items survive; integration tests cover
  each case.

### [ ] P0-05 — Harden daemon lifecycle, signals, and socket handling (`Fix`, core/CLI)

- There is no `SIGINT`/`SIGTERM` handling. Ctrl+C terminates the process
  abruptly, leaving the socket and plugins to be cleaned up by the OS.
- There is no PID file or lock. A second daemon can remove a live socket in
  `IpcServer::bind()`, because any socket is treated as stale.
- Socket cleanup is not RAII: aborting the server task or returning early can
  leave the socket behind. The socket is created with the ambient umask.
- Add graceful signal handling that stops timers/plugins, shuts down the UI,
  and removes the socket on every exit path.
- Probe an existing socket before unlinking it: connect/ping; if alive, refuse
  to start with a clear error. Use a locked PID file with stale-process
  detection.
- Create the runtime directory with `0700` and the socket with `0600`.
- Add an RAII socket guard so panics/errors do not leak files.
- Acceptance: `SIGINT`, `SIGTERM`, and `fluxbar stop` all exit cleanly; a
  second daemon cannot hijack a running one; stale sockets are recovered;
  permissions are restrictive; integration tests cover all paths.

### [ ] P0-06 — Correct plugin lifecycle semantics (`Fix`, core)

- `PluginHost::notify(name, event)` ignores `name` and broadcasts to every
  subscriber.
- A failed `init` request is only logged; the plugin remains marked running
  and continues receiving events.
- `max_restarts` counts total restarts, not consecutive crashes, and never
  resets after stable operation. Only the backoff timer resets.
- `stop_all()` trips a host-wide shutdown watch permanently; there is no way
  to start plugins again afterward.
- `stop()` aborts the task without a graceful `shutdown` handshake, and
  `plugin.list` does not expose an explicit failed/backoff state.
- Implement per-plugin cancellation, targeted notifications, consecutive
  restart accounting with a stable-period reset, graceful shutdown with a
  timeout, and a terminal `failed` state with `last_error`.
- Decide and document whether `init` failure consumes a restart attempt.
- Acceptance: targeted notify reaches one plugin; init failure surfaces in
  `plugin.list` and follows the retry policy; restart limit resets after the
  stable window; stop and stop-all are graceful and restartable; unit tests
  cover each transition.

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
  bar; `full_width = false` always anchors the 1200 pt bar at x=0 instead of
  centering it; negative computed widths are possible with extreme padding.
- Add an overflow policy: shrink/truncate low-priority items, hide overflow
  with a configurable indicator, or clip deterministically.
- Center a non-full-width bar and expose an optional `max_width`.
- Acceptance: 100-item and zero-width stress tests produce no overlap outside
  the chosen policy; center truncation is predictable; a non-full-width bar is
  centered.

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

### [ ] P1-05 — Implement script-backed items (`Feature`, core/CLI)

- `script` and `update_interval` are parsed but never executed.
- Add a scheduler that runs each script with a timeout, captures stdout, and
  applies either plain text to `label` or JSON
  (`label`/`icon`/`progress`/`graph`/`status`/`visible`).
- Deduplicate concurrent runs, kill hung children, log stderr at debug level,
  and skip scripts whose items are absent. Document the security implications.
- Acceptance: a script item updates at its interval, recovers from errors,
  never blocks the daemon, and stops when the item is removed; tests use a
  mock command.

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

### [ ] P1-10 — Blur and translucent backgrounds (`Feature`, macOS/config)

- `bar.blur_radius` is parsed but not applied.
- Use `NSVisualEffectView` with a rounded mask where possible, or a Core Image
  blur for custom radii. Define interaction with background opacity and
  rounded corners.
- Measure idle and animation cost; keep a no-blur fallback for reduced
  transparency/high-contrast modes.
- Acceptance: configured blur renders without clipping artifacts; disabling
  it restores the current behavior; performance does not regress beyond the
  P2-05 budget.

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
- Introduce frame diffing/damage regions, cache text/shape resources, and
  separate layout from painting.
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
  `cargo-deny`/`cargo-audit`; cache Cargo artifacts.
- Produce universal macOS release binaries, checksums, a changelog, and
  signed/notarized artifacts where credentials exist.
- Add a Homebrew tap/formula, `cargo install` guidance, and a proper uninstall
  path.
- Acceptance: a tagged release builds reproducibly from CI; installation
  instructions are verified on a clean machine; `main` is protected by checks.

### [ ] P2-08 — Launch-at-login service integration (`Feature`, CLI/macOS)

- Add `fluxbar service install|uninstall|status` using a LaunchAgent plist,
  with log paths, restart policy, and clean removal.
- Detect an already-running daemon and avoid duplicate agents.
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

- Add VoiceOver labels/roles for items, respect Reduce Motion and Increase
  Contrast, verify scaling on Retina/non-Retina and external displays, and
  provide keyboard focus where practical.
- Acceptance: VoiceOver can identify item labels and actions; reduced motion
  disables transitions; rendering is crisp at every tested scale.

### [ ] P3-05 — Configuration editor support (`Feature`, tooling)

- Ship the generated JSON Schema, add a `#:schema` directive to
  `config init`, and document VS Code/Helix/Neovim setup.
- Add `fluxbar config edit`/`config path --json` conveniences.
- Acceptance: editors validate keys and types out of the box; the schema is
  versioned with the binary.

### [ ] P3-06 — Repository and community hygiene (`Chore`, repo)

- Add `CONTRIBUTING.md`, `SECURITY.md`, issue/PR templates, a changelog, and
  a release checklist. Document the supported macOS versions and known
  Accessibility/Automation prompts.
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

- Add a theme showcase, a script-item example, a media/workspace plugin, a
  multi-display config, and a minimal launchd setup. Validate every example in
  CI with `fluxbar config validate`.
- Acceptance: examples cover every public feature and stay buildable.

---

## Explicit non-goals for v1.x

- Wasm sandboxing and capability permissions (v2, tracked as P2-12).
- Linux/Windows feature parity (architecture work tracked as P2-04/P2-11).
- Running as a privileged/system-wide daemon; FluxBar remains per-user.
- Loading arbitrary native code into the daemon process; plugins stay
  out-of-process.
