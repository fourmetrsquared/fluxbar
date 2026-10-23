//! FluxBar native macOS rendering.
//!
//! On macOS the crate provides a real AppKit bar window. On other platforms it
//! falls back to a headless renderer so the workspace keeps building and the
//! platform-independent parts can be tested everywhere.

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod renderer;

#[cfg(target_os = "macos")]
pub use app::{
    FRAME_INTERVAL, MacApp, UiCommand, UiEvent, main_screen_size, run_bar, run_headless,
};
#[cfg(target_os = "macos")]
pub use renderer::MacRenderer;

#[cfg(not(target_os = "macos"))]
mod headless;
#[cfg(not(target_os = "macos"))]
pub use headless::{
    FRAME_INTERVAL, MacRenderer, UiCommand, UiEvent, main_screen_size, run_bar, run_headless,
};
