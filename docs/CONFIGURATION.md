# Configuration reference

FluxBar reads `~/.config/fluxbar/fluxbar.toml` by default. The location honours
`XDG_CONFIG_HOME` and can be overridden with `fluxbar --config <file>` or
`FLUXBAR_CONFIG`. Use `fluxbar config path` to see the effective path.

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
Includes are not recursive.

## `[bar]`

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `height` | float | `32` | Bar height in points (0–256). |
| `position` | `"top"` \| `"bottom"` | `"top"` | Placement on screen. The bar is pinned below the native menu bar for top placement. |
| `background` | color | `"0xCC1E1E2E"` | Bar background. |
| `corner_radius` | float | `0` | Rounded bar corners. |
| `blur_radius` | float | `0` | Parsed but not applied yet. |
| `padding` | insets | `8` horizontal | Inner padding around all items. |
| `item_spacing` | float | `6` | Gap between items in a group. |
| `top_margin` | float | `0` | Extra margin between the bar and the screen edge. |
| `full_width` | bool | `true` | Span the full screen; otherwise capped at 1200 pt. |
| `hidden` | bool | `false` | Start with the bar hidden. |
| `display` | int | main display | Reserved for multi-monitor targeting. |
| `font` | string | system | Default label font family. |
| `font_size` | float | `13` | Default label size. |
| `foreground` | color | `"0xFFFFFFFF"` | Default label/icon color. |
| `icon_font` | string | label font | Default icon font family. |

Insets use the shape `{ top = 0, left = 0, bottom = 0, right = 0 }`.

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
| `background_corner_radius` | float | `0` | Background radius. |
| `background_height` | float | auto | Explicit background height. |
| `background_padding` | insets | `0` | Background inset relative to item bounds. |
| `border_color` | color | none | Border color. |
| `border_width` | float | `0` | Border width. |
| `align` | `"left"` \| `"center"` \| `"right"` | `"center"` | Content alignment. |
| `width`, `min_width`, `max_width` | float | auto | Width constraints; `0` means auto/unlimited. |
| `visible` | bool | `true` | Visibility toggle. |
| `drawing_order` | int | `0` | Reserved paint-order hint. |
| `click_action`, `scroll_action`, `enter_action`, `leave_action` | action | none | Mouse bindings (see below). |
| `plugin` | string | none | Plugin that owns the item (informational). |
| `properties` | table | `{}` | Arbitrary JSON values for tooling. |
| `script`, `update_interval` | string / int | none | Reserved for command-driven items. |

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

## Built-in kinds and events

| Kind | Updated by | Label example |
| --- | --- | --- |
| `cpu` | `cpu_change` (system monitor) | `12%` |
| `memory` | `memory_change` | `9.2/32G` |
| `battery` | `power_state` | `87%` |
| `volume` | `volume_change` | `80%` |
| `media` | `media_change` (enable `system.media`) | `Artist — Title` |
| `time` | internal minute clock | `18:04` |
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
duration = 220          # milliseconds
easing = "ease_out_expo"
disabled = false
```

Supported easing names: `linear`, `ease_in_quad`, `ease_out_quad`,
`ease_in_out_quad`, the same family for cubic/quart/quint/sine/circ/expo/back/
elastic/bounce, and `ease_in_out_expo` (default). The macOS renderer
interpolates item bounds, label/icon/background/border colors, progress values,
bar background and corner radius.

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
command = "fluxbar-plugin-sysmon"
args = ["--fast"]
working_dir = "/path"
env = { API_KEY = "..." }
subscribe = ["tick", "volume_change"]  # "*" = everything
tick_interval = 2                 # seconds; sends `tick`
auto_restart = true
max_restarts = 0                  # 0 = unlimited
restart_delay = 1                 # base seconds, doubles to 60
[plugins.sysmon.config]           # forwarded to the plugin's init
endpoint = "https://example.com"
```

`type` accepts `external`, `uds`, `wasm` and `builtin`, but only `external`
(stdio) is implemented today. A plugin that crashes is restarted with
exponential backoff; the backoff resets after a stable run of 60 seconds.

## `[events.custom.<name>]`

Optional documentation entries with a `description`; useful for making custom
event names self-documenting. They do not need to be declared to be triggered.

## Reloading

```bash
fluxbar trigger config_reload        # or: fluxbar plugin install ... (automatic)
```

Reloading re-reads the config, updates bar styling on the next frame, adds new
items and merges existing ones, and starts newly registered plugins. Items are
never deleted by a reload (use `fluxbar remove`). Timers, the system-monitor
intervals and plugins removed from the config keep their startup state until
the daemon is restarted.

## Runtime updates

Items created with `fluxbar add` live in memory. They survive config reloads if
the new config still contains them; items removed from the config are kept for
the current session. `fluxbar config init` writes a fully commented starting
point with the CPU/memory/battery/volume/clock items shown in the README.
