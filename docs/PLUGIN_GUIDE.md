# Plugin guide

For a step-by-step tutorial in Russian — from an empty directory through
installation, iteration, and item updates — see [GUIDE_RU.md](GUIDE_RU.md).
This page is the protocol and reference documentation.

FluxBar plugins are ordinary executables. The daemon starts each plugin once and
keeps it alive, exchanging newline-delimited JSON-RPC 2.0 messages over stdin
and stdout. There is no process spawn per update.

- Rust authors can use [ `fluxbar-sdk` ](../fluxbar-sdk) and implement one trait.
- Any other language can implement the small protocol directly.
- crashes are isolated: the bar keeps running and the plugin is restarted with
  exponential backoff (configurable, resets after 60 s of stable operation).

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

Environment variables provided to every plugin:
`FLUXBAR_PLUGIN_NAME` and `FLUXBAR_PROTOCOL=1`. Plugin stderr is forwarded to
the daemon log line by line.

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

## Plugin manifest

A repository can include `fluxbar-plugin.toml` at its root:

```toml
name = "hello"
description = "Minimal example"
build = "cargo build --release"       # optional, executed in the plugin dir
entry = "target/release/hello"        # relative path or "python3 plugin.py"
subscribe = ["*"]                     # "*" or event names
tick_interval = 2                     # optional seconds
```

Without a manifest the installer infers:

| Project | Build | Entry |
| --- | --- | --- |
| `Cargo.toml` | `cargo build --release` | `target/release/<package>` |
| `package.json` | `npm install` | `index.js` |
| `main.py` | none | `main.py` |

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
  `fluxbar trigger config_reload` after starting it.
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
  `command`, `env` or `working_dir` changed, restart the daemon so the new spec
  is loaded.

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
- `max_restarts = 0` means unlimited; set a positive value to cap attempts.
- A plugin that runs for at least 60 seconds gets a fresh backoff.
- The supervisor forwards plugin stderr to the daemon log. `fluxbar list
  --plugins` shows name, running state, restarts and pid; the JSON-RPC
  `plugin.list` response also includes `last_error`.
