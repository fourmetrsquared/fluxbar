# FluxBar

[![CI](https://github.com/fluxbar/fluxbar/actions/workflows/ci.yml/badge.svg)](https://github.com/fluxbar/fluxbar/actions/workflows/ci.yml)
[![Security](https://github.com/fluxbar/fluxbar/actions/workflows/security.yml/badge.svg)](https://github.com/fluxbar/fluxbar/actions/workflows/security.yml)

**Blazing fast. Secure. Absolutely customizable.** A next-generation macOS
status bar written in Rust — a fast, scriptable alternative to SketchyBar with
a first-class plugin system.

FluxBar renders a native AppKit bar, exposes a JSON-RPC API over a Unix socket,
and runs persistent plugins that speak line-delimited JSON-RPC over
stdin/stdout. Items and styling can be changed live from the CLI, and
`fluxbar config reload` reconciles items, plugins, timers, the monitor and bar
visibility without restarting the daemon. Plugin code changes still require a
rebuild and a plugin restart.

```
┌──────────────────────────────────────────────────────────────────────────┐
│  ◆ FluxBar   ▦ 12%   ▤ 9.2/32G        ♪ Song — Artist      ◍ 80%  ▮ 87%  │
└──────────────────────────────────────────────────────────────────────────┘
```

## Quick start

Requirements: macOS 12+, Rust 1.89+ (install via [rustup](https://rustup.rs)).
Native Liquid Glass edge deformation requires macOS 26; earlier releases use
the built-in painted glass fallback. The `rust-toolchain.toml` pins stable and
enables `rustfmt` + `clippy`.

```bash
git clone https://github.com/fluxbar/fluxbar.git fluxbar
cd fluxbar

./build.sh                 # debug build + tests + example plugins
./build.sh install         # release build, installs to ~/.local/bin/fluxbar
fluxbar config init        # writes ~/.config/fluxbar/fluxbar.toml
fluxbar start              # launch the daemon and the bar
```

`./build.sh` is safe to run immediately after cloning. It locates the repository
root itself, checks the toolchain version, and prints actionable errors. Run
`./build.sh help` for all commands.

### Homebrew and login service

The repository contains a Homebrew formula with a launchd service definition,
so Homebrew users can run FluxBar at login through `brew services`:

```bash
# Before the first tagged release (HEAD install from this repository):
brew tap fluxbar/dev https://github.com/fluxbar/fluxbar.git
brew install --HEAD fluxbar/dev/fluxbar
brew services start fluxbar

brew services list
brew services restart fluxbar
brew services stop fluxbar
```

Once a tagged release and the `fluxbar/tap` tap are published, the stable form
is:

```bash
brew tap fluxbar/tap
brew install fluxbar
brew services start fluxbar
```

On Homebrew versions that require trust for third-party taps, run
`brew trust fluxbar/dev` once (or `brew trust fluxbar/tap` for the stable tap)
if Homebrew asks you to.

Run `brew services` as your normal user, not with `sudo`: the service is a
user LaunchAgent that must run in the graphical login session to display the
bar. Configuration remains at `~/.config/fluxbar/fluxbar.toml`. Service logs
are written to `$(brew --prefix)/var/log/fluxbar.log` and
`$(brew --prefix)/var/log/fluxbar.err.log`. Use `brew services stop fluxbar`
to stop the managed service; a clean `fluxbar stop` also works, but the
service is registered to start again at the next login. To resume before the
next login, use `brew services restart fluxbar`. Remove a Homebrew install with
`brew uninstall fluxbar`; remove a source install with
`./build.sh uninstall --yes` (configuration and caches are kept).

### Source installs: launchd

If FluxBar was installed with `./build.sh install` (or another source method),
the binary can manage its own per-user LaunchAgent:

```bash
fluxbar service install       # write, load, and start the agent
fluxbar service status        # plist, launchd, daemon, and log paths
fluxbar service uninstall     # stop the agent and remove the plist/socket
```

The agent is stored at `~/Library/LaunchAgents/dev.fluxbar.fluxbar.plist` and
logs to `~/Library/Logs/FluxBar/fluxbar.log` and `fluxbar.err.log`. It starts at
login and is restarted after a crash, but a clean `fluxbar stop` stays stopped
until the next login or `launchctl kickstart`. Run these commands as your normal
user, not with `sudo`, and do not enable this agent alongside
`brew services start fluxbar`: both use the same daemon socket.

Use `fluxbar service install --no-start` to write and inspect the plist without
loading it. `fluxbar service uninstall --remove-logs` also deletes the service
logs. The agent points at the executable that ran `service install`; rerun the
command after moving or upgrading a source-installed binary.

```text
./build.sh              Build (debug), build examples, run tests
./build.sh release      Optimised build (LTO, stripped)
./build.sh install      Release build + install to ~/.local/bin
./build.sh uninstall    Remove the source-installed binary (--yes required)
./build.sh dist         Universal release artifacts + SHA256SUMS
./build.sh docs         Check relative links in Markdown files
./build.sh run start    Build and run the daemon
./build.sh test         Run the test suite
./build.sh check        fmt --check + clippy -D warnings + tests
./build.sh examples     Build the example plugins
./build.sh doctor       Print environment diagnostics
```

### Release artifacts

A tagged `v*` release runs the GitHub Actions workflow in
[.github/workflows/release.yml](.github/workflows/release.yml), which builds a
universal Apple Silicon + Intel archive, publishes `SHA256SUMS`, release notes,
and a stable Homebrew formula. Developer ID signing and Homebrew tap updates
are enabled when the release secrets are configured.

The same artifacts can be built locally:

```bash
./scripts/release.sh          # or: ./build.sh dist
cd target/dist/assets
shasum -a 256 -c SHA256SUMS
```

See [docs/RELEASE.md](docs/RELEASE.md) for the tag, signing, Homebrew tap, and
clean-machine verification steps.

## CLI

```bash
fluxbar start                       # daemon + native bar
fluxbar start --headless            # daemon only (IPC/plugin testing)
fluxbar status [--json]             # uptime, counts, counters (raw JSON optional)
fluxbar stop
fluxbar service install|status|uninstall  # source-install LaunchAgent

fluxbar add weather --kind text --position right --label "18°"
fluxbar set weather label=19° icon=☀
fluxbar set cpu progress=42 visible=true
fluxbar remove weather
fluxbar list
fluxbar list --plugins

fluxbar trigger build_finished --payload '{"job": 3}'

fluxbar config path
fluxbar config init [--force]
fluxbar config validate
fluxbar config reload              # transactional item/plugin/timer/monitor reconcile
fluxbar config show

fluxbar plugin install github:user/repo
fluxbar plugin install ./examples/plugins/hello
# or, for development, keep sources in place:
fluxbar plugin install ./examples/plugins/hello --link
fluxbar plugin list
fluxbar plugin restart hello
fluxbar plugin update hello
fluxbar plugin remove hello
fluxbar paths
```

Global flags: `--socket <path>`, `--config <file>`, `-v/-vv/-vvv`,
`--color=auto|always|never`, and the logging flags below. Diagnostics are
colorized only on a terminal by default; `NO_COLOR=1` or `--color=never`
disables styling, while `--color=always` is useful in CI logs. Errors include
the failing RPC method, item/plugin name, config path, and a concrete next step
where possible.

### Observability

Run with `-v` (or `RUST_LOG=fluxbar=debug`) for structured diagnostics. File
logging is optional and size-rotated, so a daemon can run without a terminal:

```bash
fluxbar --log-file ~/Library/Logs/FluxBar/fluxbar.log \
        --log-max-bytes 10485760 --log-keep 3 -v start
```

`--log-max-bytes 0` disables rotation; `--log-keep 0` truncates the active file
instead of keeping archives. Log files and newly created parent directories are
owner-only. `RUST_LOG` filtering works with both stderr and file output.

`fluxbar status --json` returns the raw `system.status` response, including
`counters.events`, `counters.dropped_events`, `counters.frames`,
`counters.plugin_restarts`, `counters.ipc_requests`, and
`counters.ipc_errors`. `fluxbar status` prints the same counters in text form.

Set `FLUXBAR_DEBUG_LAYOUT=1` while starting the GUI to draw red bar/item bounds
and log each item's layout rectangle at debug level. This is useful for
diagnosing clipping and spacing without changing the configuration.

Environment variables: `FLUXBAR_SOCKET`, `FLUXBAR_CONFIG`, `FLUXBAR_CONFIG_DIR`,
`FLUXBAR_LOG_FILE`, `FLUXBAR_LOG_MAX_BYTES`, `FLUXBAR_LOG_KEEP`,
`FLUXBAR_DEBUG_LAYOUT`, `NO_COLOR`, and `RUST_LOG`.

## Documentation

- [docs/GUIDE_RU.md](docs/GUIDE_RU.md) — пошаговое руководство: первый запуск,
  настройка панели, размещение и стилизация виджетов, создание Rust/Python
  плагинов, итерация, события, IPC и отладка.
- [docs/CONFIGURATION.md](docs/CONFIGURATION.md) — field reference for the TOML
  configuration.
- [docs/PLUGIN_GUIDE.md](docs/PLUGIN_GUIDE.md) — protocol, manifest, SDK,
  sh/bash/Lua plugins, SketchyBar compatibility and workspace setup.
- [docs/SAFETY.md](docs/SAFETY.md) — unsafe-code invariants and the dependency
  policy enforced by `cargo-deny` and `cargo-machete`.
- [examples/config/full.toml](examples/config/full.toml) — a complete working
  configuration.
- [examples/plugins](examples/plugins) — Rust, Python and workspace plugin
  examples.

## Configuration

The main file is `~/.config/fluxbar/fluxbar.toml` (respects
`XDG_CONFIG_HOME`; override with `--config`). On first start FluxBar creates the
default configuration plus `plugins/` and `plugins.d/` there if they do not
exist. Partial configs are merged from `include = [...]` and every `*.toml` in
`~/.config/fluxbar/plugins.d/`, which is where `fluxbar plugin install` writes
registrations. See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for the full
reference and [examples/config/full.toml](examples/config/full.toml) for a
complete example.

## Plugins

FluxBar supports persistent JSON-RPC plugins and event-driven `sh`, `bash` and
Lua scripts. JSON-RPC plugins are started once and exchange messages over
stdin/stdout; script plugins are spawned per event and can either print a JSON
update or call `sketchybar`/`fluxbar` through compatibility shims. Crash
isolation and exponential-backoff restarts are built in. Installed plugins are
stored under `~/.config/fluxbar/plugins/`; use `--link` during development to
register a local plugin in place instead of copying it.

> **Plugins are code.** Installation, build commands, item scripts, and mouse
> actions run with your user privileges. Install only trusted plugins. If a
> repository contains a `build` command, review it or install with `--no-build`
> first. The IPC socket is private to your user; a custom `--socket` path must
> also live in a private directory. See [SECURITY.md](SECURITY.md).

The bundled [workspaces plugin](examples/plugins/workspaces) renders one
clickable tile per desktop for yabai (native Mission Control spaces) and
AeroSpace, with a desktop number, a short name, per-tile
opacity/blur/shape/color, and optional app-to-workspace rules for newly opened
windows.

A minimal Rust plugin (full example in
[examples/plugins/hello](examples/plugins/hello)):

```rust
use fluxbar_sdk::async_trait;
use fluxbar_sdk::{run, Plugin, PluginContext, PluginEvent, PluginOutput};

struct Hello;

#[async_trait]
impl Plugin for Hello {
    async fn on_event(
        &mut self,
        event: &PluginEvent,
        _ctx: &PluginContext,
    ) -> fluxbar_sdk::Result<PluginOutput> {
        let mut out = PluginOutput::new();
        out.set_label("hello", format!("saw {}", event.kind()));
        Ok(out)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> fluxbar_sdk::Result<()> {
    run(Hello).await
}
```

Persistent plugins can be written in any language: they are executables that
read one JSON object per line from stdin and write one response per line to
stdout. Shell and Lua scripts can instead run per event and use the
`sketchybar` compatibility CLI. See
[docs/PLUGIN_GUIDE.md](docs/PLUGIN_GUIDE.md) for the protocol, script model,
manifest format, installation flow, and a dependency-free Python example.

## What works today

- Native AppKit bar pinned below the menu bar, top or bottom placement.
- Items with icon + label + progress bar + vector graph, per-item styling,
  padding, backgrounds, borders, width constraints and alignment. Progress
  colors, track, height and `bottom`/`top`/`background` placement are
  configurable; top/bottom strips reserve space instead of covering content.
- Bar and per-item transparency, native macOS backdrop blur, and per-item
  `rectangle`/`rounded`/`circle` backgrounds.
- Runtime add/update/remove/list over CLI and JSON-RPC.
- Built-in CPU, memory, battery/power, volume, app-focus, space (via command)
  and optional media events, plus configurable timers and custom events.
- Persistent JSON-RPC plugins and event-driven `sh`/`bash`/Lua scripts, with the
  `fluxbar-sdk`, a plugin installer supporting `github:user/repo`, git URLs and
  local paths, explicit lifecycle states, targeted notifications, graceful
  shutdown, and automatic restarts with consecutive-failure limits.
- A SketchyBar-compatible CLI and generated `sketchybar`/`fluxbar` shims for
  existing scripts, including item scripts, click scripts, subscriptions and
  event triggers.
- A bundled yabai/AeroSpace desktop plugin with clickable Liquid Glass tiles,
  short names, and explicit app-to-workspace rules.
- Mouse click / scroll / enter / leave actions and `mouse.*` events.
- Liquid Glass surfaces: a centered floating capsule with configurable top and
  side margins, native vibrancy, and rounded glass item capsules. On macOS 26
  FluxBar uses Apple's `NSGlassEffectView` for real Liquid Glass edge
  deformation and refraction; older releases get a painted specular sheen,
  bright rim and bottom refraction shade.
- Animated transitions (bounds, colors, opacity, progress) with 32 easing
  functions, including a UIKit-style `spring`; configurable duration and
  globally disableable.
- TOML config with deep-merged includes and modular `plugins.d` files.
- Transactional `config.reload` that reconciles items, plugins, timers, the
  system monitor and bar visibility.
- Graceful `SIGINT`/`SIGTERM` shutdown and a locked PID file that prevents
  duplicate daemons.
- A Homebrew formula with a launchd service definition for
  `brew services start fluxbar`.
- Headless mode for CI and IPC tests.

## Architecture

```
fluxbar-core     Platform-independent state, config, layout, events, plugins,
                 IPC server/client, system monitor, animations
fluxbar-macos    AppKit window, drawing, mouse input, manual run loop
fluxbar-sdk      Rust plugin SDK + JSON-RPC stdio runtime
fluxbar-cli      `fluxbar` binary: CLI, daemon wiring, plugin installer
examples/        Standalone example plugins and a full config
```

The daemon runs on a background tokio runtime. AppKit owns the main thread; the
two communicate through `std::sync::mpsc` channels carrying `UiCommand` and
`UiEvent`. Layout is computed in core and reduced to a `RenderFrame`; the
platform renderer only paints it. This keeps the core testable and makes a
future Windows/Linux renderer a matter of implementing the `Renderer` trait.

## Development

```bash
./build.sh check          # fmt + clippy -D warnings + tests
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

`docs/STATUS.md` maps the functional requirements to their current
implementation status, including known gaps.

## Known limitations / roadmap

- macOS is the only native UI backend (headless builds work everywhere).
- Native menu-bar icon aliases (FR-1.3) are not implemented.
- Absolute x positioning and stacking are not exposed yet; left/center/right
  grouping is supported.
- Wasm and UDS plugin transports are reserved; stdio plugins are supported.
- Brightness events are defined but not polled yet.
- `display` (multi-display targeting) is not wired; the bar uses the main screen.
  `bar.top_margin` and `bar.horizontal_margin` position a floating capsule, and
  non-full-width bars are centered with a 1200 pt cap.
- Backdrop blur uses semantic macOS materials, not exact pixel radii, and is
  macOS-only. Per-item blur is rendered on macOS.
- Progress supports configurable fill/track colors, height and
  `bottom`/`top`/`background` placement; reserved strips do not overlap content.
  Text uses real CoreText metrics on macOS; fixed/`max_width` items truncate
  long labels with an ellipsis.
- `fluxbar config reload` is transactional and reconciles items, plugins,
  timers, the system monitor and bar visibility. Runtime-created items survive;
  config items removed from the file disappear; changed plugin specs restart.
- `plugin` mouse actions are reserved. SketchyBar alias/bracket components and
  popup/shadow/image properties are accepted but not implemented. The desktop
  plugin requires yabai or AeroSpace; yabai needs its scripting addition for
  click-to-focus, and new windows move with up to one tick of delay.
- Wasm sandboxing (v2) and a configurable theme file are on the roadmap.

See [TODO.md](TODO.md) for the prioritized backlog of fixes, missing features,
and modernization work.

## Security

FluxBar runs plugins, scripts, and click actions as your user. The control
socket is restricted to the current user, a locked PID file prevents duplicate
daemons, IPC and external commands have size and time limits, and the installer
validates plugin names and entry paths. `SIGINT`, `SIGTERM` and `fluxbar stop`
all use the graceful shutdown path.
Please report vulnerabilities privately using the process in
[SECURITY.md](SECURITY.md); do not open a public issue for a security report.

For an existing installation, make the configuration tree private:

```bash
chmod 700 ~/.config/fluxbar
chmod 600 ~/.config/fluxbar/fluxbar.toml
find ~/.config/fluxbar/plugins.d -type f -exec chmod 600 {} +
```

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for the
development workflow, review expectations, attribution rules, and the security
checklist. CI runs formatting, clippy, example builds, unit tests, the
process-level daemon integration suite, the MSRV check, and a relative Markdown
link check; the security workflow runs `cargo audit` for the workspace and both
example lockfiles, plus `cargo-deny` and `cargo-machete`. Tagged releases are
handled by the release workflow. See [docs/RELEASE.md](docs/RELEASE.md) for the
release process.
Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), and
user-visible changes are tracked in [CHANGELOG.md](CHANGELOG.md).

## Third-party notices

FluxBar is an independent implementation. SketchyBar, yabai, AeroSpace, and
Apple frameworks are referenced for compatibility or used through public APIs;
see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). If code from another
repository is adapted in the future, it must carry an attribution comment and
be listed there.

## License

MIT. See [LICENSE](LICENSE).
