//! Headless fallback used on non-macOS platforms.
//!
//! It renders nothing but keeps the same API surface, which makes the daemon
//! developable and testable on Linux.

use fluxbar_core::error::{FluxError, Result};
use fluxbar_core::geometry::Size;
use fluxbar_core::renderer::{RenderFrame, Renderer};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

/// Commands accepted by the headless renderer.
#[derive(Debug)]
pub enum UiCommand {
    Frame(Box<RenderFrame>),
    SetVisible(bool),
    Resize { width: f64, height: f64 },
    Shutdown,
}

/// Events emitted by the headless renderer (never).
#[derive(Debug, Clone)]
pub enum UiEvent {
    Click { item: String, button: i64 },
    Scroll { item: String, delta: f64 },
    Enter { item: String },
    Leave { item: String },
}

/// No-op renderer used when AppKit is unavailable.
pub struct MacRenderer {
    commands: Sender<UiCommand>,
    events: Mutex<Option<Receiver<UiEvent>>>,
    size: Mutex<Size>,
}

impl MacRenderer {
    pub fn new(width: f64, height: f64) -> (Self, Receiver<UiCommand>, Sender<UiEvent>) {
        let (command_tx, command_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let renderer = Self {
            commands: command_tx,
            events: Mutex::new(Some(event_rx)),
            size: Mutex::new(Size::new(width, height)),
        };
        (renderer, command_rx, event_tx)
    }

    pub fn take_events(&self) -> Option<Receiver<UiEvent>> {
        self.events.lock().ok().and_then(|mut slot| slot.take())
    }

    pub fn send(&self, command: UiCommand) -> Result<()> {
        self.commands
            .send(command)
            .map_err(|_| FluxError::Renderer("headless UI thread stopped".into()))
    }
}

impl Renderer for MacRenderer {
    fn render(&self, frame: &RenderFrame) -> Result<()> {
        if let Ok(mut size) = self.size.lock() {
            *size = Size::new(frame.bar_bounds.width(), frame.bar_bounds.height());
        }
        self.send(UiCommand::Frame(Box::new(frame.clone())))
    }

    fn set_visible(&self, visible: bool) -> Result<()> {
        self.send(UiCommand::SetVisible(visible))
    }

    fn bar_height(&self) -> f64 {
        self.size.lock().map(|size| size.height).unwrap_or(32.0)
    }

    fn screen_size(&self) -> Size {
        self.size
            .lock()
            .map(|size| *size)
            .unwrap_or(Size::new(1920.0, 1080.0))
    }
}

/// Drain frames until the channel closes. Returns immediately on non-macOS.
pub fn run_bar(
    _width: f64,
    _height: f64,
    commands: Receiver<UiCommand>,
    _events: Sender<UiEvent>,
) -> Result<(), String> {
    while let Ok(command) = commands.recv() {
        match command {
            UiCommand::Shutdown => break,
            UiCommand::Frame(frame) => {
                tracing::debug!(items = frame.items.len(), "headless frame");
            }
            _ => {}
        }
    }
    Ok(())
}

/// Drain UI commands without rendering anything.
pub fn run_headless(commands: Receiver<UiCommand>) -> Result<(), String> {
    while let Ok(command) = commands.recv() {
        if matches!(command, UiCommand::Shutdown) {
            break;
        }
    }
    Ok(())
}

/// No screens available in headless mode.
pub fn main_screen_size() -> Option<(f64, f64)> {
    None
}

/// Frame interval mirrors the macOS implementation.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
