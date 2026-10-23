//! [`Renderer`] implementation that forwards frames to the AppKit UI thread.

use crate::app::{UiCommand, UiEvent};
use fluxbar_core::error::{FluxError, Result};
use fluxbar_core::geometry::Size;
use fluxbar_core::renderer::{RenderFrame, Renderer};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender};

/// Renderer handle used by the daemon.
///
/// All methods simply enqueue commands; the UI thread applies them on its own
/// run loop. This keeps `fluxbar-core` free of AppKit calls and guarantees
/// thread affinity.
pub struct MacRenderer {
    commands: Sender<UiCommand>,
    events: Mutex<Option<Receiver<UiEvent>>>,
    size: Mutex<Size>,
}

impl MacRenderer {
    /// Create a renderer together with the command and event channels.
    ///
    /// `commands` is consumed by [`crate::run_bar`]; `events` is consumed by
    /// the daemon to react to mouse input.
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

    /// Take ownership of the event receiver (can only be called once).
    pub fn take_events(&self) -> Option<Receiver<UiEvent>> {
        self.events.lock().ok().and_then(|mut slot| slot.take())
    }

    /// Send a command to the UI thread.
    pub fn send(&self, command: UiCommand) -> Result<()> {
        self.commands
            .send(command)
            .map_err(|_| FluxError::Renderer("UI thread is not running".into()))
    }
}

impl Renderer for MacRenderer {
    fn render(&self, frame: &RenderFrame) -> Result<()> {
        {
            if let Ok(mut size) = self.size.lock() {
                *size = Size::new(frame.bar_bounds.width(), frame.bar_bounds.height());
            }
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
