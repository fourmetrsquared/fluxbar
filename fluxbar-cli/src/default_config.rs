//! The default configuration written by `fluxbar config init`.

/// Rich, commented example configuration.
pub const DEFAULT_CONFIG: &str = r##"# FluxBar configuration
# Docs: https://github.com/fluxbar/fluxbar
#
# Everything can be changed at runtime with the CLI:
#   fluxbar add battery --kind battery --position right
#   fluxbar set battery label=87%
#   fluxbar remove battery

[bar]
height = 32
position = "top"
background = "0xCC1E1E2E"
corner_radius = 0
blur_radius = 0
top_margin = 0
full_width = true
item_spacing = 6
padding = { top = 0, left = 8, bottom = 0, right = 8 }
font = "SF Pro"
font_size = 13
foreground = "0xFFFFFFFF"

[defaults]
label_font_size = 13
icon_font_size = 14
label_padding = { top = 0, left = 4, bottom = 0, right = 4 }
icon_padding = { top = 0, left = 4, bottom = 0, right = 4 }
padding = { top = 0, left = 4, bottom = 0, right = 4 }

# --- Items ------------------------------------------------------------------
# Items are laid out in three groups: left, center, right.

[items.app]
position = "left"
icon = "◆"
label = "FluxBar"
label_color = "0xFF89B4FA"
background = "0x33FFFFFF"
background_corner_radius = 6
background_padding = { top = 0, left = 6, bottom = 0, right = 6 }
click_action = { type = "shell", command = "open -a Terminal" }

[items.cpu]
position = "left"
kind = "cpu"
icon = "▦"
label = "--%"

[items.memory]
position = "left"
kind = "memory"
icon = "▤"
label = "--"

[items.media]
position = "center"
kind = "media"
icon = "♪"
label = ""

[items.volume]
position = "right"
kind = "volume"
icon = "◍"
label = "--%"
click_action = { type = "shell", command = "osascript -e 'set volume output muted not (output muted of (get volume settings))'" }

[items.battery]
position = "right"
kind = "battery"
icon = "▮"
label = "--%"

[items.clock]
position = "right"
kind = "time"
icon = "◷"
label = "--:--"

# --- Plugins ----------------------------------------------------------------
# Persistent plugins are started once and communicate over JSON-RPC.
# Install one with: fluxbar plugin install github:user/repo
#
# [plugins.sysmon]
# type = "external"
# command = "fluxbar-plugin-sysmon"
# tick_interval = 2
# subscribe = ["tick"]
# auto_restart = true

# --- Timers -----------------------------------------------------------------
# Timers emit a custom event on an interval. Plugins can subscribe to it.

[events.timers.second]
interval = 1
event = "tick"
enabled = true

# --- Animations -------------------------------------------------------------
[animations]
duration = 220
easing = "ease_out_expo"
disabled = false

# --- Built-in system monitor ------------------------------------------------
[system]
poll_interval = 2
battery_interval = 30
volume_interval = 5
emit_system_events = true
media = false
# space_command = "aerospace list-workspaces --focused"
"##;
