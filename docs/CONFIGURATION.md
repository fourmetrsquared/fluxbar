# Configuration reference

For an end-to-end tutorial in Russian — first start, bar customization, widgets,
layout, and plugin authoring — see [GUIDE_RU.md](GUIDE_RU.md). This page is the
field reference; the guide shows the workflows.

FluxBar reads `~/.config/fluxbar/fluxbar.toml` by default. The location honours
`XDG_CONFIG_HOME` and can be overridden with `fluxbar --config <file>` or
`FLUXBAR_CONFIG`. Use `fluxbar config path` to see the effective path.

When started without an explicit `--config`, FluxBar creates the configuration
directory, a default `fluxbar.toml`, and the `plugins/` and `plugins.d/`
subdirectories if they do not exist. An existing configuration is never
overwritten; `fluxbar config init` creates one explicitly.

```bash
fluxbar config init            # write a commented default config
fluxbar config init --force    # overwrite
fluxbar config validate        # parse + validate includes and timers
fluxbar config show            # print the effective (merged) configuration
```

## Includes and modular files

```toml
include = ["themes/catppuccin.toml", "work.toml"]
include_dir = "plugins.d"      # default: <config dir>/plugins.d
```

Includes are merged at the TOML level: a partial file only overrides the keys it
defines. `plugins.d/*.toml` files are merged in alphabetical order after the
explicit includes, which makes installed plugins override matching entries.
Includes are not recursive. Relative `include` and `include_dir` paths are
resolved against the directory containing the main config; absolute paths are
used as-is.

## `[bar]`

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `height` | float | `32` | Bar height in points (0–256). |
| `position` | `"top"` \| `"bottom"` | `"top"` | Placement on screen. The bar is pinned below the native menu bar for top placement. |
| `background` | color | `"0xCC1E1E2E"` | Bar background. |
| `opacity` | float | `1.0` | Multiplies the alpha channel of `background` only. |
| `corner_radius` | float | `0` | Rounded bar corners. |
| `blur_radius` | float | `0` | Native backdrop blur behind the bar. `0` disables it. On macOS 26 FluxBar uses Apple's `NSGlassEffectView` for real Liquid Glass edge refraction and tinting; older releases use `NSVisualEffectView` material tiers (`1–8`, `9–24`, `>24`) plus a painted specular sheen and rim. |
| `padding` | insets | `8` horizontal | Inner padding around all items. |
| `item_spacing` | float | `6` | Gap between items in a group. |
| `top_margin` | float | `0` | Gap between the visible screen edge and the bar. Applied to top and bottom placement. |
| `horizontal_margin` | float | `0` | Inset from both side edges. A non-zero value centers the bar as a floating capsule. |
| `full_width` | bool | `true` | Use the available screen width (screen minus `horizontal_margin`). `false` caps the bar at 1200 pt. Both variants are centered. |
| `hidden` | bool | `false` | Start with the bar hidden. |
| `display` | int | main display | Reserved for multi-monitor targeting. |
| `font` | string | system | Default label font family. |
| `font_size` | float | `13` | Default label size. |
| `foreground` | color | `"0xFFFFFFFF"` | Default label/icon color. |
| `icon_font` | string | label font | Default icon font family. |

Insets use the shape `{ top = 0, left = 0, bottom = 0, right = 0 }`.

For a floating Liquid Glass capsule, combine a vertical gap with side insets:

```toml
[bar]
height = 44
top_margin = 6
horizontal_margin = 12
corner_radius = 18
blur_radius = 22
```

The native window is positioned from `bar_bounds`, so the layout, painted
surface, backdrop blur and click hit-testing all use the same geometry.

### Colors

Colors accept `#RRGGBB`, `#RRGGBBAA`, `0xAARRGGBB`, `rgb(r,g,b)`,
`rgba(r,g,b,a)` plus a few names (`black`, `white`, `transparent`, `red`,
`green`, `blue`, `yellow`, `orange`, `purple`, `pink`, `gray`).

## `[defaults]`

Any `ItemConfig` key can appear here. Unset keys on an item inherit from this
table, which in turn inherits from the built-in defaults. This is the quickest
way to set a consistent label/icon font or padding.

## `[items.<name>]`

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `kind` | string | `"text"` | `text`, `icon`, `progress`, `graph`, `cpu`, `memory`, `battery`, `media`, `volume`, `brightness`, `time`, `custom`. |
| `position` | `"left"` \| `"center"` \| `"right"` | `"left"` | Bar group. |
| `label` | string | `""` | Static text; built-in events update dynamic kinds. |
| `icon` | string | `""` | Glyph text (Nerd Font / Unicode) drawn before the label. |
| `label_font`, `icon_font` | string | inherits | Font family. |
| `label_font_size`, `icon_font_size` | float | `13` | Font sizes. |
| `label_color`, `icon_color` | color | `0xFFFFFFFF` | Text colors. |
| `label_padding`, `icon_padding`, `padding` | insets | `4` horizontal | Spacing around text, icons and the whole item. |
| `background` | color | none | Item background. |
| `background_corner_radius` | float | `0` | Background radius for `background_shape = "rounded"`. |
| `background_shape` | `"rectangle"` \| `"rounded"` \| `"circle"` | `"rounded"` | Background geometry. `"circle"` draws an ellipse; use a square item for a true circle. |
| `background_blur_radius` | float | `0` | Per-item backdrop blur on macOS while the background is drawn. `0` disables it. On macOS 26 each blurred item uses a native `NSGlassEffectView` with a refractive rim; older releases use the same material tiers as `bar.blur_radius` plus the painted sheen and rim. |
| `opacity` | float | `1.0` | Opacity applied to the label, icon, background, border and progress bar of this item. |
| `progress_color` | color | label color | Progress fill color. |
| `progress_track_color` | color | 16% of fill | Progress track color. |
| `progress_height` | float | `3` | Height in points of a `top`/`bottom` progress strip; `0` hides it. Valid range is `0..=64`. |
| `progress_position` | `"bottom"` \| `"top"` \| `"background"` | `"bottom"` | Reserved progress strip. `background` paints a full-height fill behind the text and graph. |
| `background_height` | float | auto | Explicit background height. |
| `background_padding` | insets | `0` | Background inset relative to item bounds. |
| `border_color` | color | none | Painted border color. Blurred surfaces on macOS 26 use the native Liquid Glass rim instead, so this border is not painted for them. |
| `border_width` | float | `0` | Painted border width. See `border_color` for native-glass behavior. |
| `align` | `"left"` \| `"center"` \| `"right"` | `"center"` | Content alignment. |
| `width`, `min_width`, `max_width` | float | auto | Width constraints; `0` means auto/unlimited. On macOS text is measured with the resolved CoreText font; fixed/`max_width` items truncate long labels with a tail ellipsis. |
| `visible` | bool | `true` | Visibility toggle. |
| `drawing_order` | int | `0` | Ordering within the position group (lower comes first, then name). Not a global z-index. |
| `click_action`, `scroll_action`, `enter_action`, `leave_action` | action | none | Mouse bindings (see below). |
| `plugin` | string | none | Plugin that owns the item (informational). |
| `properties` | table | `{}` | Arbitrary JSON values for tooling. |
| `script` | string | none | Shell command run on matching events and/or every `update_interval` seconds. |
| `click_script` | string | none | Shell command run when this item is clicked. Receives the same environment as `script`. |
| `subscribe` | string[] | `[]` | Events that invoke `script`. `"*"` matches all non-internal events; `"routine"` receives explicit update events. |
| `update_interval` | int | none | Seconds between automatic `routine` runs of `script`. |

Mouse actions:

```toml
click_action  = { type = "shell", command = "open -a Terminal" }
scroll_action = { type = "trigger", event = "volume_scroll", payload = { delta = 1 } }
enter_action  = { type = "trigger", event = "mouse.entered" }
```

`type` is `shell` or `trigger` (`plugin` is reserved). Every mouse interaction
also emits a `mouse.clicked`, `mouse.scrolled`, `mouse.entered` or `mouse.left`
custom event with `{ item, button/delta }` payload, so plugins can react even
when no action is bound.

### Item scripts

```toml
[items.weather]
position = "right"
label = "--"
script = "~/.config/fluxbar/scripts/weather.sh"
subscribe = ["system_woke", "weather_refresh"]
update_interval = 300
click_script = "open -a Weather"
```

Scripts inherit the FluxBar environment and time out after 60 seconds. Every
item script runs once at daemon startup to initialize its widget. A plain
command runs through `/bin/sh`; a path ending in `.bash` uses `/bin/bash`, and a
path ending in `.lua` runs with `lua`. They may:

- print a JSON object with any `ItemUpdate` fields (`label`, `icon`, `progress`,
  `graph`, `status`, `visible`, `position`, `kind`);
- call `sketchybar` or `fluxbar`; the compatibility shims are on `PATH`.

Plain text on stdout is ignored (kept for debug logging), so a shell script must
emit JSON or use `sketchybar --set "$NAME" label=...`.

Environment variables include `NAME`, `SENDER`, `INFO`, `CONFIG_DIR`,
`FLUXBAR_CONFIG_DIR`, `FLUXBAR_SOCKET`, `FLUXBAR_EVENT`, `FLUXBAR_PAYLOAD`,
`SKETCHYBAR` and `FLUXBAR_BIN`. Mouse events add `BUTTON`, `MODIFIER` and
`SCROLL_DELTA`. A wildcard `subscribe = ["*"]` deliberately excludes internal
`item_*` events to prevent update loops.

## Built-in kinds and events

| Kind | Updated by | Label example |
| --- | --- | --- |
| `cpu` | `cpu_change` (system monitor) | `12%` |
| `memory` | `memory_change` | `9.2/32G` |
| `battery` | `power_state` | `87%` |
| `volume` | `volume_change` | `80%` |
| `media` | `media_change` (enable `system.media`) | `Artist — Title` |
| `time` | internal 1-second clock (label changes per minute) | `18:04` |
| `brightness` | reserved (event type defined, no poller) | — |
| `text` / `icon` / `progress` / `graph` / `custom` | CLI, IPC or plugins | — |

Built-in event names: `space_change`, `app_focus`, `volume_change`,
`brightness_change`, `power_state`, `media_change`, `cpu_change`,
`memory_change`, plus custom names. Trigger custom events with
`fluxbar trigger <name> --payload '<json>'` or from a plugin.

## `[events.timers.<name>]`

```toml
[events.timers.second]
interval = 1
event = "tick"
payload = { source = "timer" }  # optional
enabled = true
```

Timers emit the named event on the bus. Plugins subscribe to it with
`subscribe = ["tick"]`.

## `[animations]`

```toml
[animations]
duration = 480          # milliseconds
easing = "spring"
disabled = false
```

Supported easing names: `linear`, `spring`, `ease_in_quad`, `ease_out_quad`,
`ease_in_out_quad`, the same family for cubic/quart/quint/sine/circ/expo/back/
elastic/bounce, and `ease_in_out_expo` (default). `spring` is an underdamped
UIKit-style spring with a small overshoot. The macOS renderer interpolates item
bounds, label/icon/background/border/progress colors, progress values, bar background
and corner radius.

## `[system]`

```toml
[system]
poll_interval = 2        # CPU/memory seconds
battery_interval = 30
volume_interval = 5
emit_system_events = true
media = false            # AppleScript media queries (opt-in)
space_command = "aerospace list-workspaces --focused"
space_interval = 2
```

`space_command` can be any executable that prints a workspace id (yabai,
aerospace, custom scripts). `media` enables AppleScript-based now-playing
queries, which may prompt for automation permission.

## `[plugins.<name>]`

```toml
[plugins.sysmon]
type = "external"                 # stdio JSON-RPC
runtime = "jsonrpc"               # jsonrpc | sh | bash | lua
command = "fluxbar-plugin-sysmon"
args = ["--fast"]
working_dir = "/path"
env = { API_KEY = "..." }
subscribe = ["tick", "volume_change"]  # "*" = everything except internal item_*
tick_interval = 2                 # seconds; sends `tick`
auto_restart = true
max_restarts = 0                  # 0 = unlimited
restart_delay = 1                 # base seconds, doubles to 60
[plugins.sysmon.config]           # forwarded to the plugin's init
endpoint = "https://example.com"
```

`type` accepts `external`, `uds`, `wasm` and `builtin`, but only `external`
(stdio) is implemented today. `runtime = "sh"`, `"bash"` or `"lua"` switches to
the event-driven script model: FluxBar runs the script for `plugin_started`,
matching bus events and `tick_interval`, instead of keeping a JSON-RPC process
alive. When `runtime` is omitted, it is inferred from the command/entry
extension (`.sh`, `.bash`, `.lua`) or from an interpreter in `command`/`args`.
A JSON-RPC plugin that crashes is restarted with exponential backoff; the
backoff resets after a stable run of 60 seconds. Script plugins do not use
`auto_restart`; each event starts an independent script process.

Script plugins receive `NAME`, `SENDER`, `INFO`, `CONFIG_DIR`,
`FLUXBAR_CONFIG_DIR`, `FLUXBAR_SOCKET`, `FLUXBAR_BIN`, `SKETCHYBAR` and the
compatibility shims on `PATH`. They may print a JSON update object or call
`sketchybar` directly. See
[PLUGIN_GUIDE.md](PLUGIN_GUIDE.md#script-plugins-sh-bash-and-lua) for examples.

For yabai / AeroSpace desktop tiles, see the bundled
[workspaces plugin](../examples/plugins/workspaces/README.md) and the complete
configuration in
[PLUGIN_GUIDE.md](PLUGIN_GUIDE.md#workspaces--desktops-yabai-and-aerospace).

This table is read when the daemon starts a plugin. A validated
`config.reload` reconciles plugin specs: new plugins start, removed plugins
stop, and changed `subscribe`/`tick_interval`/`config`/`command`/`env`/
`working_dir` specs are restarted. `fluxbar plugin restart` reuses the already
loaded spec. See [GUIDE_RU.md](GUIDE_RU.md#7-редактирование-плагинов) for the
workflow.

## `[events.custom.<name>]`

Optional documentation entries with a `description`; useful for making custom
event names self-documenting. They do not need to be declared to be triggered.

## Reloading

```bash
fluxbar config reload              # or: fluxbar plugin install ... (automatic)
```

Reloading is transactional: FluxBar parses and validates the complete new file
before changing live state. It then reconciles config-owned items, plugins,
timers, the system monitor, bar geometry and `bar.hidden`. New items are added,
changed `kind`/`position`/non-empty static `label`/`icon` are applied, and items
removed from the file are removed from the bar. Items created at runtime via
`item.add` keep their ownership and survive reloads. Inherited `[defaults]`
styling is refreshed, while runtime `label`/`icon`/`background` overrides are
kept when the item section does not set them. Plugin specs are compared, not
just names: removed plugins stop, changed specs restart, unchanged processes
stay alive. A failed validation leaves the previous configuration active. See
[GUIDE_RU.md](GUIDE_RU.md#9-что-применяется-при-reload) for the field-by-field
table.

## Runtime updates

Items created with `fluxbar add` live in memory and are runtime-owned: they
survive config reloads even if the file does not contain them. Config-owned
items removed from the file are removed from the bar. `fluxbar set` changes
runtime state fields such as `label`, `icon`, `progress`, `visible` and
`position`; use `fluxbar sketchybar --set` for static style properties such as
colors, background, opacity and fonts. `fluxbar config init` writes a fully
commented starting point with the CPU/memory/battery/volume/clock items shown in
the README.
