//! FluxBar native macOS rendering.
//!
//! On macOS the crate provides a real AppKit bar window. On other platforms it
//! falls back to a headless renderer so the workspace keeps building and the
//! platform-independent parts can be tested everywhere.
//!
//! # Safety
//!
//! This crate contains the AppKit/Objective-C interop; the rest of the
//! workspace denies `unsafe_code`. See `docs/SAFETY.md` for the invariants
//! behind the `msg_send!`, `dlopen`, CoreText and main-thread calls.

#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod renderer;
#[cfg(target_os = "macos")]
mod text;

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
