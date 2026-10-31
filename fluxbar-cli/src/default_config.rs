//! The default configuration written by `fluxbar config init`.

/// Rich, commented example configuration.
pub const DEFAULT_CONFIG: &str = r##"# FluxBar configuration
# Docs: https://github.com/fourmetrsquared/fluxbar
#
# Everything can be changed at runtime with the CLI:
#   fluxbar add battery --kind battery --position right
#   fluxbar set battery label=87%
#   fluxbar remove battery

[bar]
height = 44
position = "top"
background = "0x2E0A0A0F"
opacity = 1.0
corner_radius = 18
blur_radius = 22
# Floating glass capsule: a small gap below the menu bar and side margins.
top_margin = 6
horizontal_margin = 12
full_width = true
item_spacing = 8
padding = { top = 0, left = 12, bottom = 0, right = 12 }
font = "SF Pro"
font_size = 14
foreground = "0xFFFFFFFF"

# Liquid Glass: each item is a translucent capsule over the blurred bar.
# background_height + background_padding.bottom center the capsule vertically.
[defaults]
label_font_size = 13
icon_font_size = 14
label_color = "0xE6FFFFFF"
icon_color = "0xE6FFFFFF"
label_padding = { top = 0, left = 5, bottom = 0, right = 5 }
icon_padding = { top = 0, left = 4, bottom = 0, right = 4 }
padding = { top = 0, left = 9, bottom = 0, right = 9 }
background = "0x1FFFFFFF"
background_corner_radius = 13
background_blur_radius = 14
background_height = 30
background_padding = { top = 0, left = 0, bottom = 7, right = 0 }
# Painted border. On macOS 26 blurred surfaces use the native Liquid Glass rim
# instead, so this border is only visible on older systems.
border_color = "0x26FFFFFF"
border_width = 1

# --- Items ------------------------------------------------------------------
# Items are laid out in three groups: left, center, right.

[items.app]
position = "left"
icon = "◆"
label = "FluxBar"
label_color = "0xFFFFFFFF"
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
# Persistent JSON-RPC plugins are started once. Shell/Lua plugins use
# runtime = "sh" | "bash" | "lua" and are run per event.
# Install one with: fluxbar plugin install github:user/repo
#
# [plugins.sysmon]
# type = "external"
# command = "fluxbar-plugin-sysmon"
# tick_interval = 2
# subscribe = ["tick"]
# auto_restart = true
#
# The bundled desktop-tile plugin for yabai and AeroSpace is documented in
# examples/plugins/workspaces/README.md.

# --- Timers -----------------------------------------------------------------
# Timers emit a custom event on an interval. Plugins can subscribe to it.

[events.timers.second]
interval = 1
event = "tick"
enabled = true

# --- Animations -------------------------------------------------------------
# `spring` is a UIKit-style spring: quick start, tiny overshoot, smooth settle.
[animations]
duration = 480
easing = "spring"
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
