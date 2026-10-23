# Implementation status

This page maps [REQUIREMENTS.md](REQUIREMENTS.md) to the current codebase.
Legend: ✅ implemented, 🟡 partial, ⬜ reserved / not implemented.

## Functional requirements

| ID | Status | Notes |
| --- | --- | --- |
| FR-1.1 | ✅ | `item.add/set/remove/get/list` over JSON-RPC; CLI wrappers. No restart required. |
| FR-1.2 | 🟡 | `left`/`center`/`right` groups and ordering within a group (`drawing_order`, then name) are supported. Absolute x positioning and global stacking are not exposed. |
| FR-1.3 | ⬜ | Native menu-bar icon aliases are not implemented. |
| FR-2.1 | 🟡 | Space (via `space_command`), app focus, volume, power/battery, media (opt-in) and CPU/memory events are emitted. `brightness_change` exists as an event type but has no poller. |
| FR-2.2 | ✅ | `fluxbar trigger <name> --payload '<json>'`; names are validated. |
| FR-2.3 | ✅ | `[events.timers.*]` with interval/event/payload/enabled, plus plugin `tick_interval`. |
| FR-3.1 | 🟡 | Persistent stdio JSON-RPC plugins are fully implemented. UDS transport is reserved (config type accepted, only `external` is started). |
| FR-3.2 | ⬜ | Wasm plugins are a v2 goal; no runtime/sandbox yet. |
| FR-3.3 | ✅ | `fluxbar-sdk` with one required async method (`on_event`), optional `on_init`/`on_tick`/`on_shutdown`, and `run`/`run_blocking`. |
| FR-3.4 | ✅ | `fluxbar plugin install github:user/repo`, git URLs and local paths; builds, resolves entry points, copies local plugins into `plugins/`, writes `plugins.d/<name>.toml`. `--link` keeps local sources in place. |
| FR-3.5 | ✅ | Process isolation per plugin, crash detection, exponential backoff capped at 60 s, configurable retries, stderr forwarding. |
| FR-4.1 | ✅ | A single item can draw icon + label + progress + graph simultaneously. |
| FR-4.2 | ✅ | Frame transitions interpolate bounds, label/icon/background/border colors, alpha/opacity, progress and corner radius using configurable duration + 30 easings. |
| FR-4.3 | 🟡 | Vector line graphs are rendered from numeric arrays. Dedicated bar-chart mode is not exposed. |
| FR-4.4 | ✅ | Click/right-click, scroll and enter/leave are delivered to the daemon, emit `mouse.*` events, and run shell/trigger bindings. `plugin`-type actions are reserved. |
| FR-5.1 | ✅ | TOML configuration with validation and `fluxbar config` tooling. The default config, `plugins/`, and `plugins.d/` are created under `~/.config/fluxbar` on first start. |
| FR-5.2 | ✅ | Deep-merged `include` list and modular `plugins.d/*.toml`. |

## Non-functional requirements

| ID | Status | Notes |
| --- | --- | --- |
| NFR-1 | 🟡 | Event-driven design (no polling when idle apart from configured system intervals; the clock wakes once per second and only repaints when the minute label changes) and no repaint unless the frame version changes. Formal <0.5% idle CPU and <16 ms event-latency benchmarks have not been added. |
| NFR-2 | ✅ | Slow event subscribers get a `Lagged` warning instead of blocking producers; plugin failures are isolated; invalid config/updates return errors. |
| NFR-3 | 🟡 | Plugins run in separate processes and cannot access core memory, but there is no Wasm sandbox or default filesystem/network restriction yet. |
| NFR-4 | 🟡 | `Renderer` is platform-independent and non-macOS builds run headless; a second native backend does not exist yet. |

## Definition of done

| Item | Status |
| --- | --- |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ clean |
| `cargo fmt --all -- --check` | ✅ clean |
| Unit tests for config parsing and IPC routing | ✅ (config includes/merging/timers/mouse actions, JSON-RPC roundtrip, handler routing, plugin protocol/backoff, layout/animations) |
| Docs updated | ✅ README, configuration reference, plugin guide, status, examples, Russian end-to-end guide |

## Milestones

- **v1 (current):** native bar, items, events, stdio plugins, SDK, installer,
  config includes, animations, mouse bindings, headless mode.
- **v1.1:** menu-bar aliases, absolute positioning, brightness events, blur,
  multi-display targeting, script items, plugin-type mouse actions.
- **v2:** Wasm sandbox, UDS transport, theme files, additional renderers.

See [../TODO.md](../TODO.md) for the prioritized and actionable backlog.
