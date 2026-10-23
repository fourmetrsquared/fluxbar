# FluxBar

**Blazing fast. Secure. Absolutely customizable.** A next-generation macOS
status bar written in Rust — a fast, scriptable alternative to SketchyBar with
a first-class plugin system.

FluxBar renders a native AppKit bar, exposes a JSON-RPC API over a Unix socket,
and runs persistent plugins that speak line-delimited JSON-RPC over
stdin/stdout. Everything can be changed live from the CLI without restarting the
daemon.

```
┌──────────────────────────────────────────────────────────────────────────┐
│  ◆ FluxBar   ▦ 12%   ▤ 9.2/32G        ♪ Song — Artist      ◍ 80%  ▮ 87%  │
└──────────────────────────────────────────────────────────────────────────┘
```

## Quick start

Requirements: macOS 12+, Rust 1.85+ (install via [rustup](https://rustup.rs)).
The `rust-toolchain.toml` pins stable and enables `rustfmt` + `clippy`.

```bash
git clone <your-repo> fluxbar
cd fluxbar

./build.sh                 # debug build + tests + example plugin
./build.sh install         # release build, installs to ~/.local/bin/fluxbar
fluxbar config init        # writes ~/.config/fluxbar/fluxbar.toml
fluxbar start              # launch the daemon and the bar
```

`./build.sh` is safe to run immediately after cloning. It locates the repository
root itself, checks the toolchain version, and prints actionable errors. Run
`./build.sh help` for all commands.

```text
./build.sh              Build (debug), build examples, run tests
./build.sh release      Optimised build (LTO, stripped)
./build.sh install      Release build + install to ~/.local/bin
./build.sh run start    Build and run the daemon
./build.sh test         Run the test suite
./build.sh check        fmt --check + clippy -D warnings + tests
./build.sh examples     Build the example plugin
./build.sh doctor       Print environment diagnostics
```

## CLI

```bash
fluxbar start                       # daemon + native bar
fluxbar start --headless            # daemon only (IPC/plugin testing)
fluxbar status                      # uptime, item/plugin counts, frames
fluxbar stop

fluxbar add weather --kind text --position right --label "18°"
fluxbar set weather label=19° icon=☀
fluxbar set cpu progress=42 visible=true
fluxbar remove weather
fluxbar list
fluxbar list --plugins

fluxbar trigger build_finished --payload '{"job": 3}'

fluxbar config path | init [--force] | validate | show
fluxbar plugin install github:user/repo
fluxbar plugin install ./examples/plugins/hello --no-build
fluxbar plugin install ./examples/plugins/hello --no-build --link  # development: keep sources in place
fluxbar plugin list | restart hello | update hello | remove hello
fluxbar paths
```

Global flags: `--socket <path>`, `--config <file>`, `-v/-vv/-vvv`.
Environment variables: `FLUXBAR_SOCKET`, `FLUXBAR_CONFIG`, `FLUXBAR_CONFIG_DIR`
and `RUST_LOG`.

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

Plugins are long-running processes. FluxBar starts them once and exchanges
JSON-RPC 2.0 messages over stdin/stdout — no process-per-update overhead. Crash
isolation and exponential-backoff restarts are built in. Installed plugins are
stored under `~/.config/fluxbar/plugins/`; use `--link` during development to
register a local plugin in place instead of copying it.

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

Any language works. Plugins are just executables that read one JSON object per
line from stdin and write one response per line to stdout. See
[docs/PLUGIN_GUIDE.md](docs/PLUGIN_GUIDE.md) for the protocol, manifest format,
installation flow, and a dependency-free Python example.

## What works today

- Native AppKit bar pinned below the menu bar, top or bottom placement.
- Items with icon + label + progress bar + vector graph, per-item styling,
  padding, backgrounds, borders, width constraints and alignment.
- Runtime add/update/remove/list over CLI and JSON-RPC.
- Built-in CPU, memory, battery/power, volume, app-focus, space (via command)
  and optional media events, plus configurable timers and custom events.
- Persistent plugins over stdio with the `fluxbar-sdk`, a plugin installer
  supporting `github:user/repo`, git URLs and local paths, and automatic
  restarts with backoff.
- Mouse click / scroll / enter / leave actions and `mouse.*` events.
- Animated transitions (bounds, colors, progress) with 30 easing functions;
  configurable duration and globally disableable.
- TOML config with deep-merged includes and modular `plugins.d` files.
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
- Blur radius is parsed but not applied; multi-display targeting is not wired.
- `plugin` mouse actions and item `script` polling are reserved.
- Wasm sandboxing (v2) and a configurable theme file are on the roadmap.

See [TODO.md](TODO.md) for the prioritized backlog of fixes, missing features,
and modernization work.

## License

MIT. See [LICENSE](LICENSE).
