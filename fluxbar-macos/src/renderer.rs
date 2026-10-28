//! [`Renderer`] implementation that forwards frames to the AppKit UI thread.

use crate::app::{UiCommand, UiEvent};
use crate::text::measure_text_with_font;
use fluxbar_core::error::{FluxError, Result};
use fluxbar_core::geometry::Size;
use fluxbar_core::renderer::{RenderFrame, Renderer, TextMetrics};
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
    screen: Mutex<Size>,
    bar_height: Mutex<f64>,
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
            screen: Mutex::new(Size::new(width, height)),
            bar_height: Mutex::new(0.0),
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
        if let Ok(mut height) = self.bar_height.lock() {
            *height = frame.bar_bounds.height();
        }
        self.send(UiCommand::Frame(Box::new(frame.clone())))
    }

    fn set_visible(&self, visible: bool) -> Result<()> {
        self.send(UiCommand::SetVisible(visible))
    }

    fn bar_height(&self) -> f64 {
        self.bar_height.lock().map(|height| *height).unwrap_or(32.0)
    }

    fn screen_size(&self) -> Size {
        self.screen
            .lock()
            .map(|size| *size)
            .unwrap_or(Size::new(1920.0, 1080.0))
    }
}

impl TextMetrics for MacRenderer {
    fn text_width(&self, text: &str, font: Option<&str>, font_size: f64) -> f64 {
        measure_text_with_font(text, font, font_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxbar_core::config::Config;
    use fluxbar_core::renderer::layout;
    use fluxbar_core::state::BarState;

    #[test]
    fn text_metrics_use_exact_widths_and_cache_lookups() {
        let (renderer, _commands, _events) = MacRenderer::new(1920.0, 1080.0);
        let wide = renderer.text_width("WWWW", None, 13.0);
        let narrow = renderer.text_width("iiii", None, 13.0);
        assert!(wide > narrow);
        assert_eq!(wide, renderer.text_width("WWWW", None, 13.0));
        assert!(renderer.text_width("日本語", None, 13.0) > 0.0);
        assert!(renderer.text_width("👋🏽", None, 13.0) > 0.0);
    }

    #[test]
    fn rendering_keeps_the_screen_size() {
        let (renderer, _commands, _events) = MacRenderer::new(1920.0, 1080.0);
        let screen = Size::new(1920.0, 1080.0);
        let frame = layout(&BarState::new(), &Config::default(), screen);

        renderer.render(&frame).unwrap();

        assert_eq!(renderer.screen_size(), screen);
        assert_eq!(renderer.bar_height(), Config::default().bar.height);
    }
}
