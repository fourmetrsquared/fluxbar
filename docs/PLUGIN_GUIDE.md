# Plugin guide

For a step-by-step tutorial in Russian — from an empty directory through
installation, iteration, and item updates — see [GUIDE_RU.md](GUIDE_RU.md).
This page is the protocol and reference documentation.

FluxBar supports two plugin models:

- **Persistent JSON-RPC plugins.** The daemon starts the process once and keeps
  it alive, exchanging newline-delimited JSON-RPC 2.0 messages over stdin and
  stdout. Rust authors can use [`fluxbar-sdk`](../fluxbar-sdk) and implement one
  trait; any other language can implement the small protocol directly.
- **Event-driven scripts.** `sh`, `bash` and Lua scripts are spawned per event
  (and per `tick_interval`). They can print a JSON update object or call the
  `sketchybar`/`fluxbar` compatibility CLI. This is the easiest path for porting
  existing SketchyBar plugins.

In both models crashes are isolated: the bar keeps running and JSON-RPC plugins
are restarted with exponential backoff (configurable, resets after 60 s of
stable operation). Script failures are logged per invocation and do not stop
the daemon.

## Lifecycle and protocol

FluxBar sends one request per line, for example:

```json
{"jsonrpc":"2.0","id":1,"method":"init","params":{"name":"hello","config":{"greeting":"Hi"},"subscribe":["*"],"version":"0.1.0"}}
```

The plugin must reply on one line:

```json
{"jsonrpc":"2.0","id":1,"result":null}
```

There are three host methods:

| Method | When | `params` |
| --- | --- | --- |
| `init` | Once after spawn | `name`, `config` (the `[plugins.<name>.config]` table), `subscribe`, `version` |
| `event` | For each subscribed event and each `tick_interval` | `kind`, `payload`, `timestamp`, `source` |
| `shutdown` | On daemon stop | `null` |

For `event`, respond with an object:

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "result": {
    "updates": {
      "hello": { "label": "Hi #7", "progress": 42.0, "graph": [1, 2, 3] }
    },
    "events": [
      { "name": "hello_finished", "payload": { "ok": true } }
    ],
    "log": "optional line forwarded to the daemon log"
  }
}
```

`updates` is keyed by item name and accepts any `ItemUpdate` field: `label`,
`icon`, `progress` (0–100), `graph` (array of numbers), `status`, `visible`,
`position` (`left`/`center`/`right`) and `kind`. Multiple fields may be set at
once; updates are merged into the item state.

`events` are emitted on the FluxBar bus, so other plugins and UI bindings can
react to them. An `id` must match the request id. Requests time out after 10
seconds; a timeout while waiting for an `event` response is treated as a crash.

Environment variables provided to every JSON-RPC plugin include
`FLUXBAR_PLUGIN_NAME`, `FLUXBAR_PROTOCOL=1`, `CONFIG_DIR`, `FLUXBAR_CONFIG_DIR`,
`FLUXBAR_SOCKET`, `FLUXBAR_BIN`, `SKETCHYBAR`, and a `PATH` with the generated
`<config-dir>/bin` shims prepended. Plugin stderr is forwarded to the daemon log
line by line.

## Rust SDK

Add the SDK as a path or git dependency:

```toml
[dependencies]
fluxbar-sdk = { path = "../fluxbar/fluxbar-sdk" }
tokio = { version = "1", features = ["rt", "macros"] }
```

```rust
use fluxbar_sdk::async_trait;
use fluxbar_sdk::{run, Plugin, PluginContext, PluginEvent, PluginOutput};

struct Clock;

#[async_trait]
impl Plugin for Clock {
    async fn on_init(&mut self, ctx: &PluginContext) -> fluxbar_sdk::Result<()> {
        // ctx.name(), ctx.config_str("format"), ...
        Ok(())
    }

    async fn on_event(
        &mut self,
        event: &PluginEvent,
        _ctx: &PluginContext,
    ) -> fluxbar_sdk::Result<PluginOutput> {
        let mut out = PluginOutput::new();
        out.set_label("clock", event.kind());
        Ok(out)
    }

    async fn on_tick(&mut self, _ctx: &PluginContext) -> fluxbar_sdk::Result<PluginOutput> {
        Ok(PluginOutput::new())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> fluxbar_sdk::Result<()> {
    run(Clock).await
}
```

Only `on_event` is required. `run()` handles framing, the init/shutdown
handshake, and error reporting. A blocking variant (`run_blocking`) is available
for synchronous plugins.

Build and install the bundled example:

```bash
cargo build --release --manifest-path examples/plugins/hello/Cargo.toml
fluxbar plugin install examples/plugins/hello --no-build
```

Then add an item named `hello` (and optionally `hello-graph`) to the config, or
run `fluxbar add hello --kind text --position center`. The installer copies the
plugin to `~/.config/fluxbar/plugins/hello`, registers it in
`~/.config/fluxbar/plugins.d/hello.toml`, and asks a running daemon to reload
its configuration, so the plugin starts immediately. Without a running daemon,
the registration is picked up on the next start. Pass `--link` if you want to
keep editing the source in place:

```bash
fluxbar plugin install examples/plugins/hello --no-build
```

## Python (no SDK)

The protocol is small enough to implement with the standard library. See
[examples/plugins/hello-python/plugin.py](../examples/plugins/hello-python/plugin.py)
for a complete example. The essential loop:

```python
import json, sys

for line in sys.stdin:
    request = json.loads(line)
    method, request_id = request["method"], request["id"]

    if method == "init":
        result = None
    elif method == "shutdown":
        reply = {"jsonrpc": "2.0", "id": request_id, "result": None}
        print(json.dumps(reply), flush=True)
        break
    else:
        kind = request.get("params", {}).get("kind", "")
        result = {"updates": {"hello": {"label": f"saw {kind}"}}}

    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}), flush=True)
```

The installer detects `main.py` and writes `command = "python3"` with the script
path in `args`. Node plugins are detected through `package.json` (`npm install`
is used as the build step). Interpreter arguments are kept separate so the
daemon never has to parse a shell command line.

## Script plugins (sh, bash and Lua)

Script plugins are registered like any other plugin, but use a script runtime
instead of a long-lived JSON-RPC process:

```toml
[plugins.workspace_tick]
type = "external"
runtime = "sh"
command = "~/.config/fluxbar/plugins/workspace-tick/tick.sh"
subscribe = ["space_change", "routine"]
tick_interval = 1
```

`runtime` may be `sh`, `bash` or `lua`. When omitted, FluxBar infers it from a
`.sh`, `.bash` or `.lua` entry, or from `sh`/`bash`/`lua` in `command`. The
script is run once for `plugin_started`, for each subscribed event, and for each
`tick_interval` tick. Lua requires a `lua` executable in `PATH`; sh uses
`/bin/sh`, bash uses `/bin/bash`. `auto_restart`, `max_restarts` and
`restart_delay` apply only to JSON-RPC plugins; script invocations are
independent processes.

The environment is the same as for item scripts:

| Variable | Meaning |
| --- | --- |
| `NAME` | Plugin name |
| `SENDER` | SketchyBar-style event name (`routine`, `front_app_switched`, ...) |
| `INFO` | Event payload rendered as text |
| `FLUXBAR_EVENT` / `FLUXBAR_PAYLOAD` | Raw event name and JSON payload |
| `CONFIG_DIR` / `FLUXBAR_CONFIG_DIR` | Configuration directory |
| `FLUXBAR_SOCKET` | Daemon socket |
| `FLUXBAR_BIN` | Path to the running `fluxbar` executable |
| `SKETCHYBAR` | Path to the generated compatibility shim |
| `PATH` | Includes `<config-dir>/bin` |

A script can print a JSON update object:

```sh
#!/bin/sh
# ~/.config/fluxbar/plugins/workspace-tick/tick.sh
printf '{"workspace-tick":{"label":"%s"}}\n' "$(date +%H:%M)"
```

or call the compatibility CLI:

```sh
#!/bin/bash
set -euo pipefail
sketchybar --set workspace-tick label="$(date +%H:%M)" icon="◷"
```

```lua
#!/usr/bin/env lua
local sender = os.getenv("SENDER") or "routine"
io.write(string.format('{"workspace-tick":{"label":"%s"}}\n', sender))
```

`sketchybar` and `fluxbar` shims are created in `<config-dir>/bin` and point at
the current FluxBar executable, so a plugin does not need to hard-code a path.
Wildcard `subscribe = ["*"]` does not receive internal `item_*` events; this
prevents a script that calls `sketchybar --set` from triggering itself forever.

## SketchyBar compatibility

The `fluxbar sketchybar` command translates the common SketchyBar CLI surface.
The shims make existing plugin scripts work without changes:

| Command | Support |
| --- | --- |
| `--bar key=value...` | `height`, `position`, `color`/`background`, `opacity`, `blur_radius`, `corner_radius`, padding, `y_offset` (top margin), `margin` (horizontal margin / floating capsule), `hidden`, fonts, `foreground`. `display` is reserved. Unsupported keys are ignored with a warning. |
| `--default key=value...` | Defaults for future items. |
| `--add item NAME POSITION` | Creates a text item. `space` creates a custom item; it is not a native Mission Control space. |
| `--add event NAME` | Registers a custom event name. |
| `--set NAME key=value...` | Label/icon/progress/drawing/position/colors, background color, corner radius, shape, blur radius, height, padding, border, width, opacity, script/click_script/update_freq, drawing order. |
| `--remove`, `--rename`, `--move` | Supported for items. |
| `--subscribe`, `--unsubscribe` | Item event subscriptions. |
| `--trigger`, `--update`, `--reload`, `--query` | Supported. `--query` prints JSON item/bar state. |
| `--add alias`, `--add bracket` | Accepted/ignored; not implemented. |
| popup, shadow and image properties | Accepted/ignored; not implemented. |

The compatibility layer is deliberately not a complete SketchyBar clone. The
unsupported surface is tracked in [TODO.md](../TODO.md). Existing scripts should
still run: unknown item/bar properties produce a warning and are skipped rather
than aborting the command.

## Workspaces / desktops (yabai and AeroSpace)

`examples/plugins/workspaces` is a complete desktop-tile plugin. macOS does not
expose a public API for Mission Control space discovery or switching, so the
plugin polls a window manager: yabai for native Mission Control spaces, or
AeroSpace for its workspaces.

The plugin shows one tile per existing desktop and updates the set as desktops
are added or removed. Each tile contains the desktop number and up to three
characters of its name (`1 web`); numeric names are not repeated. The focused
desktop is rendered with the brighter `selected_*` palette. Clicking a tile
focuses that desktop.

### yabai

```bash
brew install koekeishiya/formulae/yabai
brew services start yabai
cargo build --release --manifest-path examples/plugins/workspaces/Cargo.toml
fluxbar plugin install examples/plugins/workspaces --no-build
```

Native spaces have no names. Set a yabai label (`yabai -m space 1 --label web`)
or map an index to a name in `names`. Both are truncated to three characters on
the tile. Yabai's scripting addition is required for `--focus` to work.

### AeroSpace

```bash
brew install --cask nikitabobko/tap/aerospace
cargo build --release --manifest-path examples/plugins/workspaces/Cargo.toml
fluxbar plugin install examples/plugins/workspaces --no-build
```

The number is the workspace position in `aerospace list-workspaces`; the name
is the AeroSpace workspace name, optionally overridden via `names`. App rules
move new windows with
`aerospace move-node-to-workspace --window-id <id> <workspace>`.

Add this to `~/.config/fluxbar/fluxbar.toml` (or a file in `plugins.d/`) and
run `fluxbar config reload` (or restart the daemon):

```toml
[plugins.workspaces]
type = "external"
command = "/Users/you/.config/fluxbar/plugins/workspaces/target/release/fluxbar-plugin-workspaces"
subscribe = []
tick_interval = 1
auto_restart = true

[plugins.workspaces.config]
# auto prefers yabai when its daemon answers, then AeroSpace.
backend = "auto"          # auto | yabai | aerospace
all_monitors = true
icon = ""                 # optional; the number is part of the label
background = "0x1FFFFFFF"
selected_background = "0x52FFFFFF"
visible_background = "0x2EFFFFFF"
foreground = "0xC6FFFFFF"
selected_foreground = "0xFFFFFFFF"
opacity = 1.0
blur_radius = 14
corner_radius = 13
shape = "rounded"
width = 64
tile_height = 30
padding_left = 4
padding_right = 4
move_existing = false

# Short names keyed by workspace name (AeroSpace) or space label / index (yabai).
[plugins.workspaces.config.names]
"1" = "web"
"2" = "term"
"3" = "chat"

[plugins.workspaces.config.icons]
"1" = "W"
"2" = "T"
"3" = "C"

[plugins.workspaces.config.colors]
"3" = "0x663A6EA5"

# Full per-tile overrides. Any omitted field inherits the global value.
[plugins.workspaces.config.tiles."2"]
icon = "T"
color = "0x663A6EA5"
foreground = "0xFFFFFFFF"
opacity = 0.65
blur_radius = 0
corner_radius = 0
shape = "rectangle"
width = 56
tile_height = 24

# AeroSpace only: app bundle id or app name -> workspace. `*` wildcards work.
[plugins.workspaces.config.rules]
"com.apple.Safari" = "2"
"com.apple.Terminal" = "3"
"com.microsoft.VSCode" = "3"
```

Global `opacity`, `blur_radius`, `corner_radius`, `shape`, `width` and
`tile_height` apply to every tile. `icons` and `colors` are shortcuts for
per-workspace icon/background overrides; the `tiles` table can override every
style field, including opacity, blur, shape, corner radius, colors, icon and
size. A per-tile `color`/`foreground` also wins over the focused/visible
palette. For `shape = "circle"`, width is forced to the effective tile height.

AeroSpace rules are matched against new windows on each poll. The plugin does
not guess an app's purpose: configure explicit bundle-id/app-name rules.
`move_existing = true` also applies rules to windows that were already open when
the plugin started. Matching is poll-based (about one second), so a newly opened
window can appear briefly on its launch workspace before it moves.

The plugin keeps its generated item names in `<config-dir>/workspaces.items` and
removes stale tiles on the next start. Do not declare `workspace.*` items in
`[items]`.

## Plugin manifest

A repository can include `fluxbar-plugin.toml` at its root:

```toml
name = "hello"
description = "Minimal example"
build = "cargo build --release"       # optional, executed in the plugin dir
entry = "target/release/hello"        # relative path or "python3 plugin.py"
runtime = "jsonrpc"                   # jsonrpc | sh | bash | lua
subscribe = ["*"]                     # "*" or event names
tick_interval = 2                     # optional seconds
```

Without a manifest the installer infers:

| Project | Build | Entry |
| --- | --- | --- |
| `Cargo.toml` | `cargo build --release` | `target/release/<package>` |
| `package.json` | `npm install` | `index.js` |
| `main.py` | none | `main.py` |
| single `.sh` / `.bash` / `.lua` file | none | the file itself |

Single-file script installs are supported with
`fluxbar plugin install ./tick.sh --no-build`; the installer registers the
correct interpreter and runtime. A manifest may also set `runtime = "sh"`,
`"bash"` or `"lua"` explicitly.

The registered config looks like:

```toml
[plugins.hello]
type = "external"
command = "/absolute/path/to/hello"
args = []
subscribe = ["*"]
tick_interval = 2
auto_restart = true
```

## Installation sources

```bash
fluxbar plugin install github:user/repo
fluxbar plugin install https://github.com/user/repo.git
fluxbar plugin install ./path/to/plugin --no-build
fluxbar plugin install ./path/to/plugin --no-build --link  # development
fluxbar plugin install github:user/repo --name custom-name
```

- Git sources are cloned into `~/.config/fluxbar/plugins/<name>` and built.
- Local sources are **copied** into `~/.config/fluxbar/plugins/<name>` along
  with their entry point, so plugins and configuration live together. The
  original source is left untouched. With `--no-build`, build the plugin first
  or make sure its entry point already exists.
- Use `--link` for local development: the registration points at the source
  directory and changes are picked up after rebuild/restart.
- Install attempts to reload a running daemon automatically; otherwise run
  `fluxbar config reload` after starting it.
- `update` runs `git pull --ff-only` for clones and re-runs the build command.
  Copied local plugins are re-synced from their original source.
- `remove` deletes the registration and installed sources. Linked local sources
  are left untouched.
- `restart` asks the running daemon to restart a plugin without restarting the
  bar (`fluxbar plugin restart hello`). It reuses the spec loaded when the
  daemon started.
- `update` does **not** restart the plugin. For a linked or copied plugin the
  usual loop is: edit → build (`cargo build --release` or the manifest command)
  → `fluxbar plugin update hello` (rebuilds; copied installs are re-synced) →
  `fluxbar plugin restart hello`. If `subscribe`, `tick_interval`, `config`,
  `command`, `env` or `working_dir` changed, run
  `fluxbar config reload` (or restart the daemon) so the new spec is
  reconciled.

> **Trust model:** plugins are arbitrary executables running as your user.
> A manifest `build` command is executed with `/bin/sh -c` during install when
> building is enabled; `--no-build` skips it. Review untrusted repositories, and
> do not point `--socket` at a shared directory. See
> [SECURITY.md](../SECURITY.md).

## Testing plugins

Run the daemon headless and drive it from another terminal:

```bash
fluxbar start --headless --verbose
fluxbar list --plugins
fluxbar list
fluxbar trigger my_event --payload '{"value":1}'
```

You can also pipe a single request into a plugin to check its stdout framing.
Keep stdout reserved for JSON responses; write diagnostics to stderr.

## Fault tolerance

- Each plugin runs in its own process; a crash never panics the bar.
- `auto_restart = true` restarts after a crash, starting at `restart_delay`
  seconds and doubling up to 60 seconds.
- `max_restarts = 0` means unlimited; set a positive value to cap consecutive
  crash-loop attempts.
- A plugin that runs for at least 60 seconds resets both the backoff and the
  consecutive restart counter.
- A failed `init` is treated as a startup failure and follows the same restart
  policy; it does not leave a broken plugin marked running.
- The supervisor forwards plugin stderr to the daemon log. `fluxbar list
  --plugins` shows the explicit `starting`/`running`/`backoff`/`stopped`/
  `failed` state, restarts and pid; the JSON-RPC `plugin.list` response also
  includes `last_error`.
- `plugin.stop` and `plugin.restart` use the JSON-RPC `shutdown` handshake and
  wait up to five seconds before aborting the process. Direct notifications
  (`PluginHost::notify`) go only to the named plugin instead of being broadcast.
