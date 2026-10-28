//! FluxBar core library.
//!
//! The crate is deliberately platform independent: platform crates implement
//! [`Renderer`] and system event sources, while the core owns state, layout,
//! the event bus, IPC and the plugin host.

pub mod animation;
pub mod builtin;
pub mod config;
pub mod error;
pub mod event;
pub mod geometry;
pub mod ipc;
pub mod item;
pub mod plugin;
pub mod renderer;
pub mod script;
pub mod state;
pub mod system;

pub use animation::{AnimatedValue, ease};
pub use config::{
    AnimationConfig, BarConfig, BarPosition, Config, EasingFunction, ItemAlign, ItemConfig,
    MouseAction, MouseActionType, PluginConfig, PluginRuntime, PluginType, TimerConfig,
};
pub use error::{FluxError, Result};
pub use event::{Event, EventBus, EventHandler, EventKind};
pub use geometry::{Anchor, EdgeInsets, Point, Position, Rect, Size};
pub use item::{Item, ItemId, ItemKind, ItemState, ItemUpdate};
pub use plugin::{ManagedPlugin, PluginHost, PluginPhase, PluginProcess, PluginResult, PluginSpec};
pub use renderer::{
    Color, ItemStyle, LayoutConfig, RenderFrame, RenderItem, Renderer, layout, measure_item,
    resolve_style,
};
pub use script::{
    SCRIPT_TIMEOUT, ScriptCommand, ScriptContext, ScriptInvocation, ScriptOutput, run_script,
    sketchybar_sender,
};
pub use state::{BarState, StateHandle};
pub use system::{SystemConfig, SystemMonitor, SystemSnapshot};

/// Crate version, exposed to plugins and the CLI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
