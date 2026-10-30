# Implementation status

This page maps [REQUIREMENTS.md](REQUIREMENTS.md) to the current codebase.
Legend: ✅ implemented, 🟡 partial, ⬜ reserved / not implemented.

## Functional requirements

| ID | Status | Notes |
| --- | --- | --- |
| FR-1.1 | ✅ | `item.add/set/remove/get/list` over JSON-RPC; CLI wrappers. No restart required. |
| FR-1.2 | 🟡 | The bar supports top/bottom margins, side insets for a centered floating capsule, and `left`/`center`/`right` groups with ordering inside a group (`drawing_order`, then name). Absolute item x positioning and global stacking are not exposed. |
| FR-1.3 | ⬜ | Native menu-bar icon aliases are not implemented. |
| FR-2.1 | 🟡 | Space (via `space_command`), app focus, volume, power/battery, media (opt-in) and CPU/memory events are emitted. `brightness_change` exists as an event type but has no poller. |
| FR-2.2 | ✅ | `fluxbar trigger <name> --payload '<json>'`; names are validated. |
| FR-2.3 | ✅ | `[events.timers.*]` with interval/event/payload/enabled, plus plugin `tick_interval`. |
| FR-3.1 | ✅ | Persistent stdio JSON-RPC plugins and event-driven `sh`/`bash`/Lua script plugins are implemented. UDS transport is reserved (config type accepted, only `external` is started). |
| FR-3.2 | ⬜ | Wasm plugins are a v2 goal; no runtime/sandbox yet. |
| FR-3.3 | ✅ | `fluxbar-sdk` with one required async method (`on_event`), optional `on_init`/`on_tick`/`on_shutdown`, and `run`/`run_blocking`. |
| FR-3.4 | ✅ | `fluxbar plugin install github:user/repo`, git URLs and local paths; builds, resolves entry points, copies local plugins into `plugins/`, writes `plugins.d/<name>.toml`. `--link` keeps local sources in place. |
| FR-3.5 | ✅ | Process isolation per plugin, crash detection, exponential backoff capped at 60 s, consecutive-retry limits reset after 60 s of stable operation, explicit lifecycle states, targeted direct notifications, graceful stop/restart, stderr forwarding. Init failures follow the restart policy instead of leaving a broken plugin marked running. |
| FR-4.1 | ✅ | A single item can draw icon + label + progress + graph simultaneously. Progress colors, track, height and `bottom`/`top`/`background` placement are configurable; top/bottom strips reserve content space so progress never overlaps the label, icon or graph. On macOS, item layout and painting use the same CoreText font metrics; fixed/`max_width` content truncates with an ellipsis and stays inside the item bounds. |
| FR-4.2 | ✅ | Frame transitions interpolate bounds, label/icon/background/border colors, item/bar opacity, progress and corner radius using configurable duration + 32 easings, including a UIKit-style `spring`. Bar and per-item backdrop blur use native Liquid Glass on macOS 26 and the painted sheen/rim/edge-shade fallback on earlier releases. `rectangle`/`rounded`/`circle` shapes are rendered on macOS. |
| FR-4.3 | 🟡 | Vector line graphs are rendered from numeric arrays. Dedicated bar-chart mode is not exposed. |
| FR-4.4 | ✅ | Click/right-click, scroll and enter/leave are delivered to the daemon, emit `mouse.*` events, and run shell/trigger bindings. `plugin`-type actions are reserved. |
| FR-5.1 | ✅ | TOML configuration with validation and `fluxbar config` tooling. The default config, `plugins/`, and `plugins.d/` are created under `~/.config/fluxbar` on first start. |
| FR-5.2 | ✅ | Deep-merged `include` list and modular `plugins.d/*.toml`. |

## Compatibility and workspaces

- ✅ `sketchybar`/`fluxbar` shims are generated in `<config-dir>/bin`, and script
  `PATH` includes them.
- ✅ Common SketchyBar CLI commands work: `--bar`, `--default`, `--add item`,
  `--set`, `--remove`, `--move`, `--rename`, `--subscribe`, `--trigger`,
  `--update`, `--reload`, `--query`.
- ✅ Item scripts support `subscribe`, `update_interval`, `click_script`,
  SketchyBar environment variables and JSON/CLI output.
- 🟡 Compatibility is not complete: `--add alias`/`bracket`, popups, shadows,
  images, and native Mission Control spaces are accepted/ignored or absent.
- ✅ `examples/plugins/workspaces` renders desktop tiles for yabai (native
  Mission Control spaces) and AeroSpace with global and per-tile
  opacity/blur/shape/color overrides, shows the desktop number plus a short
  name, switches spaces on click, and moves new AeroSpace windows according to
  explicit app rules. A window manager is required; macOS exposes no public
  Mission Control discovery or switching API.
- 🟡 A tap-ready Homebrew formula with a `brew services` launchd definition is
  available in `Formula/fluxbar.rb`; HEAD installs work now. The tag-triggered
  release workflow builds a universal binary, `SHA256SUMS`, release notes, and
  a stable binary formula from `packaging/homebrew/fluxbar.rb.template`, with
  optional Developer ID signing and automatic tap publication. Publishing the
  tap and clean-machine install verification remain open under P2-07.

## Security posture

- ✅ The default IPC socket is owner-only (`0600`) and lives in a private cache
  directory (`0700`). A live socket is never unlinked; stale sockets are
  detected and removed.
- ✅ The daemon holds an advisory-locked PID file (`0600`) next to the socket.
  A second instance with the same socket is refused, stale PID files are reused,
  and `SIGINT`/`SIGTERM`/`fluxbar stop` share the same graceful shutdown path.
- ✅ IPC messages are capped at 1 MiB, with 64 concurrent connections and a
  60-second idle timeout. Responses that exceed the cap are replaced with a
  small JSON-RPC error.
- ✅ Script and plugin stdout/stderr, config includes, and plugin manifests are
  bounded. External system probes have a 10-second timeout and bounded output.
- ✅ Plugin names and entry-point paths are validated; empty and traversal
  names are rejected. New config/plugin directories use `0700` and new
  registration/config files use `0600`.
- ✅ Dependency audit (`cargo audit`) is clean for the workspace and both example
  lockfiles. Private reporting and a release checklist are documented in
  `SECURITY.md` and `docs/RELEASE.md`.
- 🟡 Plugins, build commands, scripts, and click actions are arbitrary code
  running as the logged-in user; there is no sandbox. Install only trusted
  plugins and review build commands.

## Non-functional requirements

| ID | Status | Notes |
| --- | --- | --- |
| NFR-1 | 🟡 | Event-driven design (no polling when idle apart from configured system intervals; the clock wakes once per second and only repaints when the minute label changes). Frame versions drive repaints, and the macOS content view now invalidates only changed item rectangles; unchanged native glass states are not reapplied. A release build with native Liquid Glass measured ~11% CPU on the Intel test Mac during 480 ms spring transitions, so the formal <0.5% idle and <16 ms event-latency budgets are still open under P2-05. |
| NFR-2 | ✅ | Slow event subscribers get a `Lagged` warning instead of blocking producers; plugin failures are isolated; invalid config/updates return errors. |
| NFR-3 | 🟡 | Plugins run in separate processes and cannot access core memory, but there is no Wasm sandbox or default filesystem/network restriction yet. |
| NFR-4 | 🟡 | `Renderer` is platform-independent and non-macOS builds run headless; a second native backend does not exist yet. |

## Definition of done

| Item | Status |
| --- | --- |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ clean |
| `cargo fmt --all -- --check` | ✅ clean |
| Rust 1.89 MSRV check | ✅ CI job |
| Unit tests for config parsing and IPC routing | ✅ (config includes/merging/timers/mouse actions, JSON-RPC roundtrip, handler routing, plugin protocol/backoff, layout/animations) |
| Docs updated | ✅ README, configuration reference, plugin guide, status, examples, Russian end-to-end guide |

## Milestones

- **v1 (current):** native bar, items, events, stdio plugins, SDK, installer,
  config includes, animations, mouse bindings, headless mode.
- **v1.1 (in progress):** menu-bar aliases, absolute positioning, brightness
  events, multi-display targeting, plugin-type mouse actions, complete
  SketchyBar popup/shadow/image parity.
- **v2:** Wasm sandbox, UDS transport, theme files, additional renderers.

See [../TODO.md](../TODO.md) for the prioritized and actionable backlog.
