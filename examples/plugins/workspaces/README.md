# Workspaces

A FluxBar plugin that renders one tile per desktop / workspace. It supports
[yabai](https://github.com/koekeishiya/yabai) (native Mission Control spaces)
and [AeroSpace](https://github.com/nikitabobko/AeroSpace).

The plugin polls the window manager once per second, creates and removes tiles
as desktops are added or removed, and shows every existing desktop. Each tile
contains the desktop number and up to three characters of its name, for example
`1 web`. The focused desktop is rendered slightly brighter than the others.
Clicking a tile focuses that desktop.

## yabai

```bash
brew install koekeishiya/formulae/yabai
brew services start yabai
cargo build --release --manifest-path examples/plugins/workspaces/Cargo.toml
fluxbar plugin install examples/plugins/workspaces --no-build
```

Add the plugin and its configuration to `~/.config/fluxbar/fluxbar.toml` (or a
file in `plugins.d/`):

```toml
[plugins.workspaces]
type = "external"
command = "/Users/you/.config/fluxbar/plugins/workspaces/target/release/fluxbar-plugin-workspaces"
subscribe = []
tick_interval = 1
auto_restart = true

[plugins.workspaces.config]
backend = "auto"          # auto | yabai | aerospace
all_monitors = true
background = "0x1FFFFFFF"
selected_background = "0x52FFFFFF"
visible_background = "0x2EFFFFFF"
foreground = "0xC6FFFFFF"
selected_foreground = "0xFFFFFFFF"
blur_radius = 14
corner_radius = 13
width = 64
tile_height = 30

# Optional short names, keyed by space label or one-based index. Native spaces
# have no names, so either label them in yabai or map them here.
[plugins.workspaces.config.names]
"1" = "web"
"2" = "dev"
"3" = "term"
```

Yabai space labels are used automatically when present:

```bash
yabai -m space 1 --label web
```

`names` overrides the label and is truncated to three characters. Keys are the
space label or the space index. `icons`, `colors` and `tiles` use the same key,
so per-desktop styles work for both labelled and unlabelled spaces. Clicking a
tile runs `yabai -m space --focus <index>`; this requires yabai's scripting
addition.

## AeroSpace

```bash
brew install --cask nikitabobko/tap/aerospace
cargo build --release --manifest-path examples/plugins/workspaces/Cargo.toml
fluxbar plugin install examples/plugins/workspaces --no-build
```

Set `backend = "aerospace"` (or leave `auto`) and configure names. The tile
number is the workspace's position in `aerospace list-workspaces`; the name is
the AeroSpace workspace name, optionally overridden through `names`.

`backend = "auto"` prefers yabai when its daemon responds and falls back to
AeroSpace. Set the backend explicitly when both are installed.

## Troubleshooting

Window-manager commands time out after two seconds, so a wedged yabai daemon
cannot stall the plugin host. If yabai keeps hanging, restart its service:

```bash
yabai --restart-service
```

The plugin reconnects automatically on the next tick.

## Styling and rules

Global `opacity`, `blur_radius`, `corner_radius`, `shape`, `width` and
`tile_height` apply to every tile. `icons` and `colors` are shortcuts for
per-workspace icon/background overrides; the `tiles` table can override every
style field. For `shape = "circle"`, width is forced to the effective tile
height.

AeroSpace app rules move new windows with
`aerospace move-node-to-workspace --window-id <id> <workspace>`. Rules are
currently AeroSpace-only. The plugin keeps its generated item names in
`<config-dir>/workspaces.items` and removes stale tiles on the next start. Do not
declare `workspace.*` items under `[items]`.
