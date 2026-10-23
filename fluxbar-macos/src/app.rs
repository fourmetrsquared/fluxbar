//! Native macOS bar built on AppKit.
//!
//! The bar is a borderless, non-activating window pinned to the top (or
//! bottom) of the main screen. A custom `NSView` paints the frame produced by
//! `fluxbar-core` and forwards mouse events back to the daemon.

#![allow(unsafe_code)]

use fluxbar_core::animation::ease;
use fluxbar_core::config::BarPosition;
use fluxbar_core::renderer::{Color, RenderFrame, RenderItem};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBezierPath, NSColor,
    NSEvent, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSGraphicsContext,
    NSScreen, NSStatusWindowLevel, NSStringDrawing, NSView, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSAttributedStringKey, NSDate, NSDefaultRunLoopMode, NSDictionary, NSPoint,
    NSRect, NSSize, NSString,
};
use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Commands sent from the daemon thread to the UI thread.
#[derive(Debug)]
pub enum UiCommand {
    Frame(Box<RenderFrame>),
    SetVisible(bool),
    Resize { width: f64, height: f64 },
    Shutdown,
}

/// Events produced by the UI thread and consumed by the daemon.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Click { item: String, button: i64 },
    Scroll { item: String, delta: f64 },
    Enter { item: String },
    Leave { item: String },
}

struct BarViewIvars {
    frame: RefCell<Option<Box<RenderFrame>>>,
    events: Sender<UiEvent>,
    hovered: RefCell<Option<String>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond main-thread use.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = BarViewIvars]
    struct BarView;

    impl BarView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let Some(frame) = self.ivars().frame.borrow().clone() else {
                return;
            };
            paint_frame(self, &frame);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.handle_click(event, 0);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.handle_click(event, 1);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let point = self.view_point(event);
            let Some(item) = self.item_at(point) else {
                return;
            };
            let delta = event.scrollingDeltaY();
            let _ = self.ivars().events.send(UiEvent::Scroll { item, delta });
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let point = self.view_point(event);
            let current = self.item_at(point);
            let mut hovered = self.ivars().hovered.borrow_mut();
            if *hovered != current {
                if let Some(previous) = hovered.take() {
                    let _ = self.ivars().events.send(UiEvent::Leave { item: previous });
                }
                if let Some(item) = &current {
                    let _ = self.ivars().events.send(UiEvent::Enter { item: item.clone() });
                }
                *hovered = current;
            }
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }
    }
);

impl BarView {
    fn new(mtm: MainThreadMarker, frame: NSRect, events: Sender<UiEvent>) -> Retained<Self> {
        let ivars = BarViewIvars {
            frame: RefCell::new(None),
            events,
            hovered: RefCell::new(None),
        };
        let this = mtm.alloc::<Self>().set_ivars(ivars);
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizesSubviews(true);
        view
    }

    fn set_frame_data(&self, frame: RenderFrame) {
        *self.ivars().frame.borrow_mut() = Some(Box::new(frame));
        // Trigger a repaint on the next run-loop pass.
        self.setNeedsDisplay(true);
    }

    fn view_point(&self, event: &NSEvent) -> NSPoint {
        let window_point = event.locationInWindow();
        self.convertPoint_fromView(window_point, None)
    }

    fn item_at(&self, point: NSPoint) -> Option<String> {
        let frame = self.ivars().frame.borrow();
        let frame = frame.as_ref()?;
        let local_x = point.x;
        // AppKit's origin is bottom-left; item bounds are top-left based.
        let local_y = self.bounds().size.height - point.y;
        frame
            .items
            .iter()
            .find(|item| {
                let r = &item.bounds;
                local_x >= r.x() - frame.bar_bounds.x()
                    && local_x <= r.max_x() - frame.bar_bounds.x()
                    && local_y >= r.y() - frame.bar_bounds.y()
                    && local_y <= r.max_y() - frame.bar_bounds.y()
            })
            .map(|item| item.name.clone())
    }

    fn handle_click(&self, event: &NSEvent, button: i64) {
        let point = self.view_point(event);
        if let Some(item) = self.item_at(point) {
            let _ = self.ivars().events.send(UiEvent::Click { item, button });
        }
    }
}

fn ns_color(color: Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(color.r) / 255.0,
        f64::from(color.g) / 255.0,
        f64::from(color.b) / 255.0,
        f64::from(color.a) / 255.0,
    )
}

fn ns_font(name: Option<&str>, size: f64) -> Retained<NSFont> {
    if let Some(name) = name {
        if let Some(font) = NSFont::fontWithName_size(&NSString::from_str(name), size) {
            return font;
        }
    }
    NSFont::systemFontOfSize(size)
}

fn attribute_dictionary(
    font: &NSFont,
    color: &NSColor,
) -> Retained<NSDictionary<NSAttributedStringKey, AnyObject>> {
    // SAFETY: The keys are the standard AppKit attribute names and the values
    // are an NSFont / NSColor, matching the documented types.
    unsafe {
        let keys: [&NSAttributedStringKey; 2] =
            [NSFontAttributeName, NSForegroundColorAttributeName];
        let font_obj: &AnyObject = font.as_ref();
        let color_obj: &AnyObject = color.as_ref();
        let values: [&AnyObject; 2] = [font_obj, color_obj];
        NSDictionary::from_slices(&keys, &values)
    }
}

fn draw_text(text: &str, rect: NSRect, font: &NSFont, color: &NSColor, align: i64) {
    if text.is_empty() {
        return;
    }
    let string = NSString::from_str(text);
    let attributes = attribute_dictionary(font, color);
    let _ = align;
    // SAFETY: We pass a valid dictionary of supported attributes.
    unsafe {
        string.drawInRect_withAttributes(
            objc2_core_foundation::CGRect {
                origin: objc2_core_foundation::CGPoint {
                    x: rect.origin.x,
                    y: rect.origin.y,
                },
                size: objc2_core_foundation::CGSize {
                    width: rect.size.width,
                    height: rect.size.height,
                },
            },
            Some(&attributes),
        );
    }
}

fn paint_frame(view: &BarView, frame: &RenderFrame) {
    let bounds = view.bounds();
    let ctx = NSGraphicsContext::currentContext();
    let _ = ctx;

    // Bar background.
    let radius = frame.corner_radius.max(0.0);
    let bar_rect = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(bounds.size.width, bounds.size.height),
    );
    let background = ns_color(frame.background);
    if radius > 0.0 {
        let path =
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bar_rect, radius, radius);
        background.setFill();
        path.fill();
    } else {
        background.setFill();
        NSBezierPath::fillRect(bar_rect);
    }

    for item in &frame.items {
        paint_item(item, frame, bounds.size.height);
    }
}

fn paint_item(item: &RenderItem, frame: &RenderFrame, view_height: f64) {
    let x = item.bounds.x() - frame.bar_bounds.x();
    let top = item.bounds.y() - frame.bar_bounds.y();
    let height = item.bounds.height();
    let y = view_height - top - height;
    let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(item.bounds.width(), height));

    // Item background.
    if let Some(background) = item.style.background {
        let color = ns_color(background);
        let radius = item.style.background_corner_radius.max(0.0);
        let padded = NSRect::new(
            NSPoint::new(
                rect.origin.x + item.style.background_padding.left,
                rect.origin.y + item.style.background_padding.bottom,
            ),
            NSSize::new(
                (rect.size.width - item.style.background_padding.horizontal_sum()).max(0.0),
                item.style
                    .background_height
                    .unwrap_or(rect.size.height - item.style.background_padding.vertical_sum())
                    .max(0.0),
            ),
        );
        color.setFill();
        if radius > 0.0 {
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(padded, radius, radius).fill();
        } else {
            NSBezierPath::fillRect(padded);
        }
    }

    // Border.
    if let Some(border) = item.style.border_color {
        if item.style.border_width > 0.0 {
            let color = ns_color(border);
            let radius = item.style.background_corner_radius.max(0.0);
            let path =
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius);
            path.setLineWidth(item.style.border_width);
            color.setStroke();
            path.stroke();
        }
    }

    // Graph.
    if item.kind == fluxbar_core::ItemKind::Graph && item.graph.len() > 1 {
        paint_graph(&item.graph, rect, &item.style.label_color);
    }

    // Progress.
    if let Some(progress) = item.progress {
        paint_progress(progress, rect, frame);
    }

    // Content layout: icon then label, centered as a group.
    let icon_font = ns_font(item.style.icon_font.as_deref(), item.style.icon_font_size);
    let label_font = ns_font(item.style.label_font.as_deref(), item.style.label_font_size);

    let icon_width = if item.icon.is_empty() {
        0.0
    } else {
        measure_text(&item.icon, &icon_font) + item.style.icon_padding.horizontal_sum()
    };
    let label_width = if item.label.is_empty() {
        0.0
    } else {
        measure_text(&item.label, &label_font) + item.style.label_padding.horizontal_sum()
    };

    let total = icon_width + label_width;
    let start = match item.style.align {
        fluxbar_core::ItemAlign::Left => rect.origin.x + item.style.padding.left,
        fluxbar_core::ItemAlign::Center => rect.origin.x + (rect.size.width - total) / 2.0,
        fluxbar_core::ItemAlign::Right => {
            rect.origin.x + rect.size.width - total - item.style.padding.right
        }
    };

    let text_height = item.style.label_font_size.max(item.style.icon_font_size);
    let text_y = rect.origin.y + (rect.size.height - text_height) / 2.0;

    if !item.icon.is_empty() {
        let icon_rect = NSRect::new(
            NSPoint::new(start + item.style.icon_padding.left, text_y),
            NSSize::new(icon_width, text_height),
        );
        let color = ns_color(item.style.icon_color);
        draw_text(&item.icon, icon_rect, &icon_font, &color, 0);
    }

    if !item.label.is_empty() {
        let label_rect = NSRect::new(
            NSPoint::new(start + icon_width + item.style.label_padding.left, text_y),
            NSSize::new(label_width, text_height),
        );
        let color = ns_color(item.style.label_color);
        draw_text(&item.label, label_rect, &label_font, &color, 0);
    }
}

fn measure_text(text: &str, font: &NSFont) -> f64 {
    let string = NSString::from_str(text);
    // SAFETY: Passing an empty attribute dictionary is valid for measuring.
    let size = unsafe { string.sizeWithAttributes(None) };
    let _ = font;
    size.width
}

fn paint_graph(data: &[f64], rect: NSRect, color: &Color) {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(f64::EPSILON);
    let step = rect.size.width / (data.len().saturating_sub(1).max(1) as f64);

    let path = NSBezierPath::bezierPath();
    for (index, value) in data.iter().enumerate() {
        let x = rect.origin.x + index as f64 * step;
        let normalized = (value - min) / range;
        let y = rect.origin.y + normalized * rect.size.height;
        let point = NSPoint::new(x, y);
        if index == 0 {
            path.moveToPoint(point);
        } else {
            path.lineToPoint(point);
        }
    }
    ns_color(*color).setStroke();
    path.setLineWidth(1.5);
    path.stroke();
}

fn paint_progress(progress: f64, rect: NSRect, frame: &RenderFrame) {
    let clamped = progress.clamp(0.0, 100.0) / 100.0;
    let track = NSRect::new(
        NSPoint::new(rect.origin.x + 4.0, rect.origin.y + 3.0),
        NSSize::new((rect.size.width - 8.0).max(0.0), 3.0),
    );
    ns_color(Color::rgba(255, 255, 255, 40)).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(track, 1.5, 1.5).fill();

    let fill = NSRect::new(
        track.origin,
        NSSize::new(track.size.width * clamped, track.size.height),
    );
    ns_color(frame.items.first().map_or(Color::WHITE, |_| Color::WHITE)).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(fill, 1.5, 1.5).fill();
}

// Borderless windows normally refuse key status; the bar needs it for mouse
// tracking and scroll events.
define_class!(
    // SAFETY: NSWindow supports subclassing; overrides only change window flags.
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    struct BarWindow;

    impl BarWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }
    }
);

/// Linearly interpolate two colors (used for animated transitions).
fn lerp_color(from: Color, to: Color, t: f64) -> Color {
    let channel = |a: u8, b: u8| -> u8 {
        (f64::from(a) + (f64::from(b) - f64::from(a)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgba(
        channel(from.r, to.r),
        channel(from.g, to.g),
        channel(from.b, to.b),
        channel(from.a, to.a),
    )
}

/// Build an intermediate frame between two layouts.
fn interpolate_frame(from: &RenderFrame, to: &RenderFrame, t: f64) -> RenderFrame {
    let mut frame = to.clone();
    frame.background = lerp_color(from.background, to.background, t);
    frame.corner_radius = from.corner_radius + (to.corner_radius - from.corner_radius) * t;

    for item in &mut frame.items {
        let Some(previous) = from.items.iter().find(|old| old.name == item.name) else {
            continue;
        };
        item.bounds = previous.bounds.lerp(item.bounds, t);
        item.style.label_color = lerp_color(previous.style.label_color, item.style.label_color, t);
        item.style.icon_color = lerp_color(previous.style.icon_color, item.style.icon_color, t);
        if let (Some(a), Some(b)) = (previous.style.background, item.style.background) {
            item.style.background = Some(lerp_color(a, b, t));
        }
        if let (Some(a), Some(b)) = (previous.style.border_color, item.style.border_color) {
            item.style.border_color = Some(lerp_color(a, b, t));
        }
        if let (Some(a), Some(b)) = (previous.progress, item.progress) {
            item.progress = Some(a + (b - a) * t);
        }
    }

    frame
}

/// In-flight transition between two layout frames.
struct FrameAnimation {
    from: RenderFrame,
    started: Instant,
}

/// Owns the application window and runs the AppKit loop.
pub struct MacApp {
    window: Retained<BarWindow>,
    view: Retained<BarView>,
    visible_frame: NSRect,
    width: f64,
    height: f64,
    position: BarPosition,
    displayed: Option<RenderFrame>,
    target: Option<RenderFrame>,
    animation: Option<FrameAnimation>,
}

impl MacApp {
    /// Create the bar window. Must be called on the main thread.
    pub fn new(
        mtm: MainThreadMarker,
        width: f64,
        height: f64,
        events: Sender<UiEvent>,
    ) -> Result<Self, String> {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();

        let screen = NSScreen::mainScreen(mtm).ok_or_else(|| "no main screen".to_string())?;
        let visible_frame = screen.visibleFrame();

        let content_rect = NSRect::new(
            NSPoint::new(
                0.0,
                visible_frame.origin.y + visible_frame.size.height - height,
            ),
            NSSize::new(width, height),
        );

        let window: Retained<BarWindow> = unsafe {
            msg_send![
                mtm.alloc::<BarWindow>(),
                initWithContentRect: content_rect,
                styleMask: NSWindowStyleMask::Borderless,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };

        unsafe { window.setReleasedWhenClosed(false) };
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(false);
        window.setIgnoresMouseEvents(false);
        window.setAcceptsMouseMovedEvents(true);
        window.setLevel(NSStatusWindowLevel);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        let view = BarView::new(
            mtm,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
            events,
        );
        window.setContentView(Some(&view));
        window.makeFirstResponder(Some(&view));
        window.orderFrontRegardless();

        info!(width, height, "macOS bar window created");

        Ok(Self {
            window,
            view,
            visible_frame,
            width,
            height,
            position: BarPosition::Top,
            displayed: None,
            target: None,
            animation: None,
        })
    }

    /// Handle a single UI command.
    pub fn handle(&mut self, command: UiCommand) {
        match command {
            UiCommand::Frame(frame) => {
                let frame = *frame;
                let height = frame.bar_bounds.height();
                let width = frame.bar_bounds.width();
                if (height - self.height).abs() > 0.5
                    || (width - self.width).abs() > 0.5
                    || frame.position != self.position
                {
                    self.place(width, height, frame.position);
                }

                let duration = frame.animation_duration_ms;
                let from = self.displayed.clone();
                self.target = Some(frame.clone());

                match (from, duration) {
                    (Some(from), duration) if duration > 0 => {
                        self.animation = Some(FrameAnimation {
                            from,
                            started: Instant::now(),
                        });
                        self.tick();
                    }
                    _ => {
                        // First frame or animations disabled: show immediately.
                        self.animation = None;
                        self.displayed = Some(frame.clone());
                        self.view.set_frame_data(frame);
                    }
                }
            }
            UiCommand::SetVisible(visible) => {
                if visible {
                    self.window.orderFrontRegardless();
                } else {
                    self.window.orderOut(None);
                }
            }
            UiCommand::Resize { width, height } => self.place(width, height, self.position),
            UiCommand::Shutdown => {}
        }
    }

    /// Advance the current transition, if any.
    fn tick(&mut self) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let Some(animation) = self.animation.take() else {
            return;
        };

        let duration = target.animation_duration_ms.max(1) as f64 / 1000.0;
        let t = (animation.started.elapsed().as_secs_f64() / duration).clamp(0.0, 1.0);

        if t >= 1.0 {
            self.displayed = Some(target.clone());
            self.view.set_frame_data(target);
        } else {
            let frame =
                interpolate_frame(&animation.from, &target, ease(target.animation_easing, t));
            self.displayed = Some(frame.clone());
            self.view.set_frame_data(frame);
            self.animation = Some(animation);
        }
    }

    fn place(&mut self, width: f64, height: f64, position: BarPosition) {
        let y = match position {
            BarPosition::Top => {
                self.visible_frame.origin.y + self.visible_frame.size.height - height
            }
            BarPosition::Bottom => self.visible_frame.origin.y,
        };
        self.window.setFrame_display(
            NSRect::new(NSPoint::new(0.0, y), NSSize::new(width, height)),
            true,
        );
        self.view.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, height),
        ));
        self.width = width;
        self.height = height;
        self.position = position;
    }

    /// Run the manual event loop until a shutdown command arrives.
    pub fn run(mut self, commands: Receiver<UiCommand>) {
        let mtm = MainThreadMarker::new().expect("run() must be called on the main thread");
        let app = NSApplication::sharedApplication(mtm);

        loop {
            let mut shutdown = false;
            loop {
                match commands.try_recv() {
                    Ok(UiCommand::Shutdown) => {
                        shutdown = true;
                        break;
                    }
                    Ok(command) => self.handle(command),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        shutdown = true;
                        break;
                    }
                }
            }
            if shutdown {
                break;
            }

            self.tick();

            let until = NSDate::dateWithTimeIntervalSinceNow(FRAME_INTERVAL.as_secs_f64());
            // SAFETY: Standard AppKit event pump with a valid run-loop mode.
            let event = unsafe {
                app.nextEventMatchingMask_untilDate_inMode_dequeue(
                    objc2_app_kit::NSEventMask::Any,
                    Some(&until),
                    NSDefaultRunLoopMode,
                    true,
                )
            };
            if let Some(event) = event {
                app.sendEvent(&event);
            }
            app.updateWindows();
        }

        warn!("UI loop exiting");
    }
}

/// Run the bar UI loop. Must be called from the main thread.
pub fn run_bar(
    width: f64,
    height: f64,
    commands: Receiver<UiCommand>,
    events: Sender<UiEvent>,
) -> Result<(), String> {
    let mtm =
        MainThreadMarker::new().ok_or_else(|| "must be called on the main thread".to_string())?;
    debug!("initialising macOS app");
    let app = MacApp::new(mtm, width, height, events)?;
    app.run(commands);
    Ok(())
}

/// Drain UI commands without creating a window (used by `--headless`).
pub fn run_headless(commands: Receiver<UiCommand>) -> Result<(), String> {
    while let Ok(command) = commands.recv() {
        if matches!(command, UiCommand::Shutdown) {
            break;
        }
    }
    Ok(())
}

/// Query the main screen size. Must be called on the main thread.
pub fn main_screen_size() -> Option<(f64, f64)> {
    let mtm = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(mtm)?;
    let frame = screen.frame();
    Some((frame.size.width, frame.size.height))
}

/// Keep `Duration` in the public API for callers that schedule repaints.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[cfg(test)]
mod tests {
    use super::*;
    use fluxbar_core::config::EasingFunction;
    use fluxbar_core::geometry::Rect;
    use fluxbar_core::renderer::{ItemStyle, RenderFrame, RenderItem};

    fn frame(x: f64, color: Color) -> RenderFrame {
        RenderFrame {
            bar_bounds: Rect::new(0.0, 0.0, 1000.0, 30.0),
            background: Color::BLACK,
            corner_radius: 0.0,
            blur_radius: 0.0,
            items: vec![RenderItem {
                name: "cpu".into(),
                kind: fluxbar_core::ItemKind::Cpu,
                bounds: Rect::new(x, 0.0, 50.0, 30.0),
                label: "1%".into(),
                icon: String::new(),
                progress: Some(10.0),
                graph: Vec::new(),
                style: ItemStyle {
                    label_color: color,
                    ..ItemStyle::default()
                },
            }],
            position: BarPosition::Top,
            version: 1,
            screen_height: 900.0,
            animation_duration_ms: 220,
            animation_easing: EasingFunction::Linear,
        }
    }

    #[test]
    fn colors_interpolate() {
        let color = lerp_color(Color::rgba(0, 0, 0, 0), Color::rgba(255, 100, 50, 255), 0.5);
        assert_eq!(color, Color::rgba(128, 50, 25, 128));
    }

    #[test]
    fn frames_interpolate_bounds_and_colors() {
        let from = frame(0.0, Color::rgba(0, 0, 0, 255));
        let to = frame(100.0, Color::rgba(255, 255, 255, 255));
        let midway = interpolate_frame(&from, &to, 0.5);

        let item = &midway.items[0];
        assert!((item.bounds.x() - 50.0).abs() < 0.01);
        assert_eq!(item.style.label_color, Color::rgba(128, 128, 128, 255));
        assert_eq!(item.progress, Some(10.0));
    }
}
