//! Native macOS bar built on AppKit.
//!
//! The bar is a borderless, non-activating window pinned to the top (or
//! bottom) of the main screen. A custom `NSView` paints the frame produced by
//! `fluxbar-core` and forwards mouse events back to the daemon.

#![allow(unsafe_code)]

use crate::text::{measure_text_with_font, ns_font};
use fluxbar_core::animation::ease;
use fluxbar_core::config::{BarPosition, ItemShape, ProgressPosition};
use fluxbar_core::renderer::{Color, RenderFrame, RenderItem};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBezierPath, NSColor,
    NSEvent, NSEventModifierFlags, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSGradient, NSGraphicsContext, NSLineBreakMode, NSMutableParagraphStyle,
    NSParagraphStyleAttributeName, NSScreen, NSStatusWindowLevel, NSStringDrawing, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindingRule, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedStringKey, NSDate, NSDefaultRunLoopMode, NSDictionary,
    NSPoint, NSRect, NSSize, NSString,
};
use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void};
use std::sync::Once;
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
    Click {
        item: String,
        button: i64,
        modifier: String,
    },
    Scroll {
        item: String,
        delta: f64,
    },
    Enter {
        item: String,
    },
    Leave {
        item: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaintMode {
    Background,
    Content,
}

struct BarViewIvars {
    frame: RefCell<Option<Box<RenderFrame>>>,
    events: Sender<UiEvent>,
    hovered: RefCell<Option<String>>,
    mode: PaintMode,
    native_glass: bool,
    truncating_style: Retained<NSMutableParagraphStyle>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond main-thread use.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = BarViewIvars]
    struct BarView;

    impl BarView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            let Some(frame) = self.ivars().frame.borrow().clone() else {
                return;
            };
            paint_frame(self, &frame, self.ivars().mode, dirty);
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
    fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        events: Sender<UiEvent>,
        mode: PaintMode,
        native_glass: bool,
    ) -> Retained<Self> {
        let truncating_style = NSMutableParagraphStyle::new();
        truncating_style.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let ivars = BarViewIvars {
            frame: RefCell::new(None),
            events,
            hovered: RefCell::new(None),
            mode,
            native_glass,
            truncating_style,
        };
        let this = mtm.alloc::<Self>().set_ivars(ivars);
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizesSubviews(true);
        view
    }

    fn set_frame_data(&self, frame: RenderFrame) {
        let dirty = {
            let previous = self.ivars().frame.borrow();
            dirty_region(
                previous.as_deref(),
                &frame,
                self.ivars().mode,
                self.bounds().size.height,
            )
        };
        *self.ivars().frame.borrow_mut() = Some(Box::new(frame));
        match dirty {
            DirtyRegion::None => {}
            DirtyRegion::Rect(rect) => self.setNeedsDisplayInRect(rect),
            DirtyRegion::All => self.setNeedsDisplay(true),
        }
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
            let _ = self.ivars().events.send(UiEvent::Click {
                item,
                button,
                modifier: modifier_name(event.modifierFlags()).to_string(),
            });
        }
    }
}

fn modifier_name(flags: NSEventModifierFlags) -> &'static str {
    if flags.contains(NSEventModifierFlags::Command) {
        "cmd"
    } else if flags.contains(NSEventModifierFlags::Option) {
        "alt"
    } else if flags.contains(NSEventModifierFlags::Control) {
        "ctrl"
    } else if flags.contains(NSEventModifierFlags::Shift) {
        "shift"
    } else {
        "none"
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

fn attribute_dictionary(
    font: &NSFont,
    color: &NSColor,
    paragraph: &NSMutableParagraphStyle,
) -> Retained<NSDictionary<NSAttributedStringKey, AnyObject>> {
    // SAFETY: The keys are the standard AppKit attribute names and the values
    // are an NSFont / NSColor / NSParagraphStyle, matching the documented
    // types.
    unsafe {
        let keys: [&NSAttributedStringKey; 3] = [
            NSFontAttributeName,
            NSForegroundColorAttributeName,
            NSParagraphStyleAttributeName,
        ];
        let font_obj: &AnyObject = font.as_ref();
        let color_obj: &AnyObject = color.as_ref();
        let paragraph_obj: &AnyObject = paragraph.as_ref();
        let values: [&AnyObject; 3] = [font_obj, color_obj, paragraph_obj];
        NSDictionary::from_slices(&keys, &values)
    }
}

fn draw_text(
    text: &str,
    rect: NSRect,
    font: &NSFont,
    color: &NSColor,
    paragraph: &NSMutableParagraphStyle,
) {
    if text.is_empty() {
        return;
    }
    let string = NSString::from_str(text);
    let attributes = attribute_dictionary(font, color, paragraph);
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

#[derive(Debug, Clone, Copy)]
enum DirtyRegion {
    None,
    Rect(NSRect),
    All,
}

fn rects_intersect(a: NSRect, b: NSRect) -> bool {
    a.origin.x < b.origin.x + b.size.width
        && a.origin.x + a.size.width > b.origin.x
        && a.origin.y < b.origin.y + b.size.height
        && a.origin.y + a.size.height > b.origin.y
}

fn expand_rect(rect: NSRect, margin: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(rect.origin.x - margin, rect.origin.y - margin),
        NSSize::new(
            rect.size.width + margin * 2.0,
            rect.size.height + margin * 2.0,
        ),
    )
}

fn union_rect(current: Option<NSRect>, rect: NSRect) -> NSRect {
    current.map_or(rect, |current| {
        let min_x = current.origin.x.min(rect.origin.x);
        let min_y = current.origin.y.min(rect.origin.y);
        let max_x = (current.origin.x + current.size.width).max(rect.origin.x + rect.size.width);
        let max_y = (current.origin.y + current.size.height).max(rect.origin.y + rect.size.height);
        NSRect::new(
            NSPoint::new(min_x, min_y),
            NSSize::new(max_x - min_x, max_y - min_y),
        )
    })
}

/// Work out which part of a bar view actually needs painting.
///
/// AppKit calls `drawRect:` for the whole view when `setNeedsDisplay(true)` is
/// used. During an animated label update most items stay identical, so limiting
/// the invalidation to the changed item rectangles avoids repainting all text
/// and glass capsules at 60 fps.
fn dirty_region(
    previous: Option<&RenderFrame>,
    next: &RenderFrame,
    mode: PaintMode,
    view_height: f64,
) -> DirtyRegion {
    let Some(previous) = previous else {
        return DirtyRegion::All;
    };

    if mode == PaintMode::Background {
        let changed = previous.background != next.background
            || previous.opacity != next.opacity
            || previous.corner_radius != next.corner_radius
            || previous.blur_radius != next.blur_radius
            || previous.bar_bounds.width() != next.bar_bounds.width()
            || previous.bar_bounds.height() != next.bar_bounds.height();
        return if changed {
            DirtyRegion::All
        } else {
            DirtyRegion::None
        };
    }

    // Local item coordinates depend on the bar size. A resize needs a full
    // repaint; ordinary movement of the floating window does not.
    if previous.bar_bounds.width() != next.bar_bounds.width()
        || previous.bar_bounds.height() != next.bar_bounds.height()
    {
        return DirtyRegion::All;
    }

    let mut dirty = None;
    for item in &next.items {
        let rect = item_rect(item, next, view_height);
        if let Some(old) = previous.items.iter().find(|old| old.name == item.name) {
            if old != item {
                dirty = Some(union_rect(dirty, item_rect(old, previous, view_height)));
                dirty = Some(union_rect(dirty, rect));
            }
        } else {
            dirty = Some(union_rect(dirty, rect));
        }
    }
    for old in &previous.items {
        if !next.items.iter().any(|item| item.name == old.name) {
            dirty = Some(union_rect(dirty, item_rect(old, previous, view_height)));
        }
    }

    match dirty {
        Some(rect) => DirtyRegion::Rect(expand_rect(rect, 2.0)),
        None => DirtyRegion::None,
    }
}

fn paint_frame(view: &BarView, frame: &RenderFrame, mode: PaintMode, dirty: NSRect) {
    let bounds = view.bounds();
    let ctx = NSGraphicsContext::currentContext();
    let _ = ctx;

    if mode == PaintMode::Background {
        // The bar backdrop lives in its own view so per-item vibrancy can be
        // layered between it and the item content.
        let radius = frame.corner_radius.max(0.0);
        let bar_rect = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(bounds.size.width, bounds.size.height),
        );
        let background = ns_color(frame.background);
        let shape = ItemShape::Rounded;

        // macOS 26 renders the actual Liquid Glass material in a native view.
        // Keep its refractive edge exposed and only paint the tinted interior
        // inset by a few points; the glass view below supplies the distortion,
        // specular rim and background blur.
        if view.ivars().native_glass && frame.blur_radius > 0.0 {
            let inset = glass_edge_inset(bounds.size.height);
            let inner_rect = inset_glass_rect(bar_rect, inset);
            let inner_path = glass_shape_path(shape, inner_rect, (radius - inset).max(0.0));
            background.setFill();
            inner_path.fill();
            return;
        }

        let path = glass_shape_path(shape, bar_rect, radius);
        background.setFill();
        path.fill();

        // Fallback for macOS versions without NSGlassEffectView: a dark lower
        // refraction, a specular top highlight and a bright rim on every edge.
        if frame.blur_radius > 0.0 {
            paint_glass_shade(&path, bar_rect, frame.opacity);
            paint_glass_highlight(&path, bar_rect, frame.opacity);
            paint_glass_rim(shape, bar_rect, radius, frame.opacity);
        }
        return;
    }

    for item in &frame.items {
        let rect = item_rect(item, frame, bounds.size.height);
        if !rects_intersect(rect, dirty) {
            continue;
        }
        paint_item(
            item,
            frame,
            bounds.size.height,
            view.ivars().native_glass,
            &view.ivars().truncating_style,
        );
    }
}

fn item_rect(item: &RenderItem, frame: &RenderFrame, view_height: f64) -> NSRect {
    let x = item.bounds.x() - frame.bar_bounds.x();
    let top = item.bounds.y() - frame.bar_bounds.y();
    let height = item.bounds.height();
    let y = view_height - top - height;
    NSRect::new(NSPoint::new(x, y), NSSize::new(item.bounds.width(), height))
}

fn item_background_rect(item: &RenderItem, frame: &RenderFrame, view_height: f64) -> NSRect {
    let rect = item_rect(item, frame, view_height);
    NSRect::new(
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
    )
}

const PROGRESS_INSET_X: f64 = 4.0;
const PROGRESS_INSET_Y: f64 = 3.0;
const PROGRESS_GAP: f64 = 2.0;

/// Rect reserved for the progress strip. `None` means the item has no visible
/// progress, or the configured height is zero.
fn progress_strip_rect(item: &RenderItem, rect: NSRect) -> Option<NSRect> {
    if item.progress.is_none() || item.style.progress_height <= 0.0 {
        return None;
    }
    let width = (rect.size.width - PROGRESS_INSET_X * 2.0).max(0.0);
    if width <= 0.0 {
        return None;
    }
    match item.style.progress_position {
        ProgressPosition::Bottom => {
            let height = item
                .style
                .progress_height
                .min((rect.size.height - PROGRESS_INSET_Y * 2.0).max(0.0));
            if height <= 0.0 {
                return None;
            }
            Some(NSRect::new(
                NSPoint::new(
                    rect.origin.x + PROGRESS_INSET_X,
                    rect.origin.y + PROGRESS_INSET_Y,
                ),
                NSSize::new(width, height),
            ))
        }
        ProgressPosition::Top => {
            let height = item
                .style
                .progress_height
                .min((rect.size.height - PROGRESS_INSET_Y * 2.0).max(0.0));
            if height <= 0.0 {
                return None;
            }
            Some(NSRect::new(
                NSPoint::new(
                    rect.origin.x + PROGRESS_INSET_X,
                    rect.origin.y + rect.size.height - PROGRESS_INSET_Y - height,
                ),
                NSSize::new(width, height),
            ))
        }
        ProgressPosition::Background => Some(NSRect::new(
            NSPoint::new(
                rect.origin.x + PROGRESS_INSET_X,
                rect.origin.y + PROGRESS_INSET_Y,
            ),
            NSSize::new(width, (rect.size.height - PROGRESS_INSET_Y * 2.0).max(0.0)),
        )),
    }
}

/// Content rect excluding a top/bottom progress strip. `background` leaves
/// the whole item available because the fill is painted behind the content.
fn item_content_rect(item: &RenderItem, rect: NSRect) -> NSRect {
    let Some(strip) = progress_strip_rect(item, rect) else {
        return rect;
    };
    match item.style.progress_position {
        ProgressPosition::Bottom => {
            let y = strip.origin.y + strip.size.height + PROGRESS_GAP;
            NSRect::new(
                NSPoint::new(rect.origin.x, y),
                NSSize::new(
                    rect.size.width,
                    (rect.origin.y + rect.size.height - y).max(0.0),
                ),
            )
        }
        ProgressPosition::Top => {
            let height = (strip.origin.y - PROGRESS_GAP - rect.origin.y).max(0.0);
            NSRect::new(rect.origin, NSSize::new(rect.size.width, height))
        }
        ProgressPosition::Background => rect,
    }
}

fn glass_shape_path(shape: ItemShape, rect: NSRect, radius: f64) -> Retained<NSBezierPath> {
    let radius = radius.max(0.0);
    match shape {
        ItemShape::Rectangle => NSBezierPath::bezierPathWithRect(rect),
        ItemShape::Rounded if radius > 0.0 => {
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius)
        }
        ItemShape::Rounded => NSBezierPath::bezierPathWithRect(rect),
        ItemShape::Circle => NSBezierPath::bezierPathWithOvalInRect(rect),
    }
}

/// Width of the native Liquid Glass edge left exposed around the tinted
/// interior. It scales with the surface so small item capsules keep their
/// proportions without the bar losing its soft, refractive rim.
fn glass_edge_inset(height: f64) -> f64 {
    (height * 0.16).clamp(3.0, 8.0)
}

fn inset_glass_rect(rect: NSRect, inset: f64) -> NSRect {
    let inset = inset.min(rect.size.width / 2.0).min(rect.size.height / 2.0);
    NSRect::new(
        NSPoint::new(rect.origin.x + inset, rect.origin.y + inset),
        NSSize::new(
            (rect.size.width - inset * 2.0).max(0.0),
            (rect.size.height - inset * 2.0).max(0.0),
        ),
    )
}

// Force AppKit to register its lazily loaded Objective-C classes.
// `filename` must be a valid NUL-terminated C string and `flag` valid dlopen
// flags.
#[link(name = "System")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
}

/// Look up Apple's Liquid Glass view at runtime. The class only exists on
/// macOS 26+, while FluxBar keeps running on older releases with the painted
/// fallback glass.
fn native_glass_view(frame: NSRect) -> Option<Retained<NSView>> {
    // AppKit lazily registers the class. Opening the already-loaded image with
    // RTLD_NOW makes the new macOS 26 classes visible to NSClassFromString /
    // AnyClass::get without linking against a newer SDK symbol directly.
    static APPKIT_REGISTRATION: Once = Once::new();
    APPKIT_REGISTRATION.call_once(|| {
        // SAFETY: `dlopen` receives a static, NUL-terminated framework path.
        unsafe {
            dlopen(
                c"/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit".as_ptr(),
                0x2 | 0x8, // RTLD_NOW | RTLD_GLOBAL
            );
        }
    });

    let class = AnyClass::get(c"NSGlassEffectView")?;
    // SAFETY: The class is an NSView subclass provided by AppKit. `alloc` and
    // `initWithFrame:` follow the standard two-step initialization contract.
    unsafe {
        let allocated: *mut AnyObject = msg_send![class, alloc];
        let initialized: *mut NSView = msg_send![allocated, initWithFrame: frame];
        let view = Retained::from_raw(initialized)?;
        // NSGlassEffectView manages its own compositing; forcing a layer here
        // can suppress the native glass pass.
        let _: () = msg_send![&*view, setStyle: 0_isize];
        Some(view)
    }
}

/// Last values applied to a native glass view. Reapplying identical frames and
/// tints every animation tick forces unnecessary Core Animation commits.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GlassState {
    frame: NSRect,
    corner_radius: f64,
    tint: Option<Color>,
    opacity: f64,
}

impl GlassState {
    fn new(frame: NSRect, corner_radius: f64, tint: Option<Color>, opacity: f64) -> Self {
        Self {
            frame,
            corner_radius: corner_radius.max(0.0),
            tint: tint.filter(|tint| tint.a > 0),
            opacity: opacity.clamp(0.0, 1.0),
        }
    }
}

fn configure_native_glass(
    view: &NSView,
    frame: NSRect,
    corner_radius: f64,
    tint: Option<Color>,
    opacity: f64,
) {
    view.setFrame(frame);
    view.setAlphaValue(opacity.clamp(0.0, 1.0));
    // SAFETY: `NSGlassEffectView` exposes `cornerRadius` and `tintColor`; this
    // is only called for views created by `native_glass_view`.
    unsafe {
        let _: () = msg_send![view, setCornerRadius: corner_radius.max(0.0)];
        match tint.filter(|tint| tint.a > 0) {
            Some(tint) => {
                let color = ns_color(tint);
                let _: () = msg_send![view, setTintColor: Some(&*color)];
            }
            None => {
                let _: () = msg_send![view, setTintColor: Option::<&NSColor>::None];
            }
        }
    }
}

fn item_inset_background_path(
    item: &RenderItem,
    rect: NSRect,
    inset: f64,
) -> Retained<NSBezierPath> {
    let inner = inset_glass_rect(rect, inset);
    let radius = match item.style.background_shape {
        ItemShape::Rectangle => 0.0,
        ItemShape::Rounded => (item.style.background_corner_radius - inset).max(0.0),
        ItemShape::Circle => inner.size.width.min(inner.size.height) / 2.0,
    };
    glass_shape_path(item.style.background_shape, inner, radius)
}

fn item_background_path(item: &RenderItem, rect: NSRect) -> Retained<NSBezierPath> {
    glass_shape_path(
        item.style.background_shape,
        rect,
        item.style.background_corner_radius,
    )
}

fn item_background_layer_radius(item: &RenderItem, rect: NSRect) -> f64 {
    match item.style.background_shape {
        ItemShape::Rectangle => 0.0,
        ItemShape::Rounded => item.style.background_corner_radius.max(0.0),
        ItemShape::Circle => rect.size.width.min(rect.size.height) / 2.0,
    }
}

/// Darken the bottom of the glass slightly. This simulates the refraction and
/// inner shadow that make a translucent surface feel thick around its edge.
fn paint_glass_shade(path: &NSBezierPath, rect: NSRect, opacity: f64) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    let opacity = opacity.clamp(0.0, 1.0);
    let clear = NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.0);
    let shade = NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.10 * opacity);
    let Some(gradient) =
        NSGradient::initWithStartingColor_endingColor(NSGradient::alloc(), &shade, &clear)
    else {
        return;
    };

    NSGraphicsContext::saveGraphicsState_class();
    path.addClip();
    gradient.drawInRect_angle(rect, 90.0);
    NSGraphicsContext::restoreGraphicsState_class();
}

/// Draw a soft, cool-white light from the top edge. Clipped to `path`, this is
/// the specular sheen that distinguishes Liquid Glass from a flat tint.
fn paint_glass_highlight(path: &NSBezierPath, rect: NSRect, opacity: f64) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    let opacity = opacity.clamp(0.0, 1.0);
    let clear = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.0);
    let soft = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.045 * opacity);
    let sheen = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.24 * opacity);
    let colors = NSArray::from_retained_slice(&[clear, soft, sheen]);
    let Some(gradient) = NSGradient::initWithColors(NSGradient::alloc(), &colors) else {
        return;
    };

    NSGraphicsContext::saveGraphicsState_class();
    path.addClip();
    // 90° runs from the bottom (transparent) to the top (bright).
    gradient.drawInRect_angle(rect, 90.0);
    NSGraphicsContext::restoreGraphicsState_class();
}

/// Bright inner rim with a vertical specular gradient. The ring is built with
/// an even-odd winding rule so the highlight never bleeds outside the rounded
/// shape. This is the edge treatment that makes the surface read as glass.
fn paint_glass_rim(shape: ItemShape, rect: NSRect, radius: f64, opacity: f64) {
    let max_inset = rect.size.width.min(rect.size.height) / 2.0;
    let inset = 1.25_f64.min(max_inset);
    if inset <= 0.0 {
        return;
    }

    let opacity = opacity.clamp(0.0, 1.0);
    let inner_rect = NSRect::new(
        NSPoint::new(rect.origin.x + inset, rect.origin.y + inset),
        NSSize::new(
            (rect.size.width - inset * 2.0).max(0.0),
            (rect.size.height - inset * 2.0).max(0.0),
        ),
    );
    let outer = glass_shape_path(shape, rect, radius);
    let inner = glass_shape_path(shape, inner_rect, (radius - inset).max(0.0));
    let ring = NSBezierPath::bezierPath();
    ring.appendBezierPath(&outer);
    ring.appendBezierPath(&inner);
    ring.setWindingRule(NSWindingRule::EvenOdd);

    let bottom = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.11 * opacity);
    let middle = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.30 * opacity);
    let top = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.56 * opacity);
    let colors = NSArray::from_retained_slice(&[bottom, middle, top]);
    let Some(gradient) = NSGradient::initWithColors(NSGradient::alloc(), &colors) else {
        return;
    };

    NSGraphicsContext::saveGraphicsState_class();
    ring.addClip();
    gradient.drawInRect_angle(rect, 90.0);
    NSGraphicsContext::restoreGraphicsState_class();
}

fn blur_material(radius: f64) -> NSVisualEffectMaterial {
    if radius <= 8.0 {
        NSVisualEffectMaterial::UnderWindowBackground
    } else if radius <= 24.0 {
        NSVisualEffectMaterial::HUDWindow
    } else {
        NSVisualEffectMaterial::FullScreenUI
    }
}

fn paint_item(
    item: &RenderItem,
    frame: &RenderFrame,
    view_height: f64,
    native_glass: bool,
    paragraph: &NSMutableParagraphStyle,
) {
    let rect = item_rect(item, frame, view_height);

    // On macOS 26 the backdrop is a native NSGlassEffectView. Paint only the
    // tinted interior, inset so the glass edge remains visible and refracts
    // the content behind the window.
    let padded = item_background_rect(item, frame, view_height);
    let use_native_glass = native_glass && item.style.background_blur_radius > 0.0;
    if use_native_glass {
        if let Some(background) = item.style.background {
            let inset = glass_edge_inset(padded.size.height);
            ns_color(background).setFill();
            item_inset_background_path(item, padded, inset).fill();
        }
    } else {
        if let Some(background) = item.style.background {
            let color = ns_color(background);
            color.setFill();
            item_background_path(item, padded).fill();
        }

        // Blurred item surfaces get the same painted glass treatment as the
        // bar on systems without native Liquid Glass.
        if item.style.background_blur_radius > 0.0 {
            let path = item_background_path(item, padded);
            paint_glass_shade(&path, padded, item.style.opacity);
            paint_glass_highlight(&path, padded, item.style.opacity);
        }

        // Border.
        if let Some(border) = item.style.border_color {
            if item.style.border_width > 0.0 {
                let color = ns_color(border);
                let path = item_background_path(item, padded);
                path.setLineWidth(item.style.border_width);
                color.setStroke();
                path.stroke();
            }
        }

        // Draw the glass rim after the optional flat border so the edge stays
        // specular instead of turning into a uniform outline.
        if item.style.background_blur_radius > 0.0 {
            paint_glass_rim(
                item.style.background_shape,
                padded,
                item.style.background_corner_radius,
                item.style.opacity,
            );
        }
    }

    let content_rect = item_content_rect(item, rect);

    // Progress is painted before the graph and text. Bottom/top strips reserve
    // space through `content_rect`; background fills sit behind the content.
    if let Some(progress) = item.progress {
        if let Some(progress_rect) = progress_strip_rect(item, rect) {
            paint_progress(
                progress,
                progress_rect,
                item.style.progress_color,
                item.style.progress_track_color,
            );
        }
    }

    // Graph.
    if item.kind == fluxbar_core::ItemKind::Graph && item.graph.len() > 1 {
        paint_graph(&item.graph, content_rect, &item.style.label_color);
    }

    // Content layout: icon then label, centered as a group. Layout uses the
    // same CoreText metrics as painting, but fixed/max widths can still leave
    // less room than the text needs. Clamp the text areas so painted content
    // never overflows the item bounds.
    let icon_font = ns_font(item.style.icon_font.as_deref(), item.style.icon_font_size);
    let label_font = ns_font(item.style.label_font.as_deref(), item.style.label_font_size);

    let icon_text = item.icon.as_str();
    let label_text = item.label.as_str();
    let icon_padding = item.style.icon_padding.horizontal_sum();
    let label_padding = item.style.label_padding.horizontal_sum();
    let icon_text_width = if icon_text.is_empty() || !item.style.icon_drawing {
        0.0
    } else {
        measure_text_with_font(
            icon_text,
            item.style.icon_font.as_deref(),
            item.style.icon_font_size,
        )
    };
    let label_text_width = if label_text.is_empty() || !item.style.label_drawing {
        0.0
    } else {
        measure_text_with_font(
            label_text,
            item.style.label_font.as_deref(),
            item.style.label_font_size,
        )
    };
    let icon_width = if icon_text_width > 0.0 {
        icon_text_width + icon_padding
    } else {
        0.0
    };
    let label_width = if label_text_width > 0.0 {
        label_text_width + label_padding
    } else {
        0.0
    };

    let content_left = content_rect.origin.x + item.style.padding.left;
    let content_right = content_rect.origin.x + content_rect.size.width - item.style.padding.right;
    let available = (content_right - content_left).max(0.0);
    let (icon_width, label_width) = fit_content_widths(icon_width, label_width, available);
    let total = icon_width + label_width;

    let start = match item.style.align {
        fluxbar_core::ItemAlign::Left => content_left,
        fluxbar_core::ItemAlign::Center => content_left + (available - total) / 2.0,
        fluxbar_core::ItemAlign::Right => content_right - total,
    };

    let text_height = item.style.label_font_size.max(item.style.icon_font_size);
    let text_y = content_rect.origin.y + (content_rect.size.height - text_height) / 2.0;

    if icon_text_width > 0.0 && icon_width > 0.0 {
        let icon_rect = NSRect::new(
            NSPoint::new(start + item.style.icon_padding.left, text_y),
            NSSize::new((icon_width - icon_padding).max(0.0), text_height),
        );
        let color = ns_color(item.style.icon_color);
        draw_text(icon_text, icon_rect, &icon_font, &color, paragraph);
    }

    if label_text_width > 0.0 && label_width > 0.0 {
        let label_rect = NSRect::new(
            NSPoint::new(start + icon_width + item.style.label_padding.left, text_y),
            NSSize::new((label_width - label_padding).max(0.0), text_height),
        );
        let color = ns_color(item.style.label_color);
        draw_text(label_text, label_rect, &label_font, &color, paragraph);
    }
}

/// Shrink label space first, then icon space, when content cannot fit. This
/// mirrors the usual status-bar priority: an icon remains recognizable while
/// a long label is truncated with an ellipsis.
fn fit_content_widths(icon_width: f64, label_width: f64, available: f64) -> (f64, f64) {
    let icon_width = icon_width.max(0.0);
    let label_width = label_width.max(0.0);
    let available = available.max(0.0);
    if icon_width + label_width <= available {
        return (icon_width, label_width);
    }
    if icon_width >= available {
        return (available, 0.0);
    }
    (icon_width, (available - icon_width).max(0.0))
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

fn progress_fraction(progress: f64) -> f64 {
    if progress.is_finite() {
        progress.clamp(0.0, 100.0) / 100.0
    } else {
        0.0
    }
}

fn paint_progress(progress: f64, rect: NSRect, color: Color, track_color: Color) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    let fraction = progress_fraction(progress);
    let radius = (rect.size.height / 2.0).max(0.0);

    ns_color(track_color).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();

    let fill_width = rect.size.width * fraction;
    if fill_width <= 0.0 {
        return;
    }
    let fill = NSRect::new(rect.origin, NSSize::new(fill_width, rect.size.height));
    let fill_radius = radius.min(fill_width / 2.0);
    ns_color(color).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(fill, fill_radius, fill_radius).fill();
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
    frame.opacity = from.opacity + (to.opacity - from.opacity) * t;
    frame.corner_radius = from.corner_radius + (to.corner_radius - from.corner_radius) * t;

    for item in &mut frame.items {
        let Some(previous) = from.items.iter().find(|old| old.name == item.name) else {
            // New tiles fade in instead of popping into the bar.
            let fade = t.clamp(0.0, 1.0);
            item.style.opacity *= fade;
            item.style.label_color = item.style.label_color.with_opacity(fade);
            item.style.icon_color = item.style.icon_color.with_opacity(fade);
            item.style.background = item.style.background.map(|bg| bg.with_opacity(fade));
            item.style.border_color = item
                .style
                .border_color
                .map(|border| border.with_opacity(fade));
            item.style.progress_color = item.style.progress_color.with_opacity(fade);
            item.style.progress_track_color = item.style.progress_track_color.with_opacity(fade);
            continue;
        };
        item.bounds = previous.bounds.lerp(item.bounds, t);
        item.style.opacity =
            previous.style.opacity + (item.style.opacity - previous.style.opacity) * t;
        item.style.label_color = lerp_color(previous.style.label_color, item.style.label_color, t);
        item.style.icon_color = lerp_color(previous.style.icon_color, item.style.icon_color, t);
        item.style.progress_color =
            lerp_color(previous.style.progress_color, item.style.progress_color, t);
        item.style.progress_track_color = lerp_color(
            previous.style.progress_track_color,
            item.style.progress_track_color,
            t,
        );
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

/// Convert a top-based margin into an AppKit window origin (bottom-left).
fn window_origin_y(
    visible_frame: NSRect,
    top_margin: f64,
    height: f64,
    position: BarPosition,
) -> f64 {
    match position {
        BarPosition::Top => {
            visible_frame.origin.y + visible_frame.size.height - height - top_margin
        }
        BarPosition::Bottom => visible_frame.origin.y + top_margin,
    }
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
    background_view: Retained<BarView>,
    blur: Retained<NSVisualEffectView>,
    bar_glass: Option<Retained<NSView>>,
    bar_glass_state: Option<GlassState>,
    item_blur_layer: Retained<NSView>,
    item_blurs: Vec<Retained<NSVisualEffectView>>,
    item_glasses: Vec<Retained<NSView>>,
    item_glass_states: Vec<Option<GlassState>>,
    native_glass: bool,
    mtm: MainThreadMarker,
    visible_frame: NSRect,
    x: f64,
    y: f64,
    top_margin: f64,
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
                visible_frame.origin.x,
                visible_frame.origin.y + visible_frame.size.height - height,
            ),
            NSSize::new(width, height),
        );

        // SAFETY: This is the standard NSWindow two-step allocation and
        // initialization sequence with a valid rectangle and style mask.
        let window: Retained<BarWindow> = unsafe {
            msg_send![
                mtm.alloc::<BarWindow>(),
                initWithContentRect: content_rect,
                styleMask: NSWindowStyleMask::Borderless,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };

        // The window is owned by the application for its whole lifetime.
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

        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));
        let container = NSView::initWithFrame(mtm.alloc::<NSView>(), frame);
        let bar_glass = native_glass_view(frame);
        let native_glass = bar_glass.is_some();
        if let Some(glass) = &bar_glass {
            glass.setHidden(true);
        }
        let blur = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
        blur.setMaterial(NSVisualEffectMaterial::HUDWindow);
        blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        blur.setState(NSVisualEffectState::Active);
        blur.setWantsLayer(true);
        blur.setHidden(true);

        let background_view = BarView::new(
            mtm,
            frame,
            events.clone(),
            PaintMode::Background,
            native_glass,
        );
        let item_blur_layer = NSView::initWithFrame(mtm.alloc::<NSView>(), frame);
        let view = BarView::new(mtm, frame, events, PaintMode::Content, native_glass);

        window.setContentView(Some(&container));
        if let Some(glass) = &bar_glass {
            container.addSubview(glass);
        }
        container.addSubview(&blur);
        container.addSubview(&background_view);
        container.addSubview(&item_blur_layer);
        container.addSubview(&view);
        window.makeFirstResponder(Some(&view));
        window.orderFrontRegardless();

        info!(
            width,
            height,
            native_glass = %native_glass,
            "macOS bar window created"
        );

        Ok(Self {
            window,
            view,
            background_view,
            blur,
            bar_glass,
            bar_glass_state: None,
            item_blur_layer,
            item_blurs: Vec::new(),
            item_glasses: Vec::new(),
            item_glass_states: Vec::new(),
            native_glass,
            mtm,
            visible_frame,
            x: 0.0,
            y: visible_frame.origin.y + visible_frame.size.height - height,
            top_margin: 0.0,
            width,
            height,
            position: BarPosition::Top,
            displayed: None,
            target: None,
            animation: None,
        })
    }

    /// Enable the native vibrancy backdrop when `blur_radius` is non-zero.
    ///
    /// `NSVisualEffectView` exposes semantic materials rather than a numeric
    /// blur radius, so the configured radius selects an intensity tier.
    fn update_blur(&self, radius: f64, corner_radius: f64) {
        if self.native_glass {
            self.blur.setHidden(true);
            return;
        }
        if radius <= 0.0 {
            self.blur.setHidden(true);
            return;
        }

        self.blur.setMaterial(blur_material(radius));
        self.blur.setHidden(false);
        if let Some(layer) = self.blur.layer() {
            layer.setCornerRadius(corner_radius.max(0.0));
            layer.setMasksToBounds(true);
        }
    }

    /// Keep the native Liquid Glass backdrop aligned with the current bar and
    /// tint. Only its edge is visible; the interior is painted above it.
    fn update_bar_glass(&mut self, frame: &RenderFrame) {
        let Some(glass) = &self.bar_glass else {
            return;
        };
        if frame.blur_radius <= 0.0 {
            glass.setHidden(true);
            self.bar_glass_state = None;
            return;
        }

        glass.setHidden(false);
        let state = GlassState::new(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(self.width, self.height)),
            frame.corner_radius,
            Some(frame.background),
            frame.opacity,
        );
        if self.bar_glass_state != Some(state) {
            configure_native_glass(
                glass,
                state.frame,
                state.corner_radius,
                state.tint,
                state.opacity,
            );
            self.bar_glass_state = Some(state);
        }
    }

    /// Reuse native Liquid Glass views for per-item backdrops. Their tinted
    /// interior is painted by `BarView`; the native rim provides the optical
    /// edge deformation.
    fn update_item_glass(&mut self, frame: &RenderFrame) {
        let mut index = 0;
        for item in &frame.items {
            if !item.style.background_drawing || item.style.background_blur_radius <= 0.0 {
                continue;
            }
            let rect = item_background_rect(item, frame, self.height);
            if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
                continue;
            }

            if index >= self.item_glasses.len() {
                let Some(glass) = native_glass_view(rect) else {
                    continue;
                };
                glass.setHidden(true);
                self.item_blur_layer.addSubview(&glass);
                self.item_glasses.push(glass);
                self.item_glass_states.push(None);
            }

            let state = GlassState::new(
                rect,
                item_background_layer_radius(item, rect),
                item.style.background,
                item.style.opacity,
            );
            if self.item_glass_states.get(index).copied().flatten() != Some(state) {
                configure_native_glass(
                    &self.item_glasses[index],
                    state.frame,
                    state.corner_radius,
                    state.tint,
                    state.opacity,
                );
                if let Some(slot) = self.item_glass_states.get_mut(index) {
                    *slot = Some(state);
                }
            }
            self.item_glasses[index].setHidden(false);
            index += 1;
        }

        for index in index..self.item_glasses.len() {
            self.item_glasses[index].setHidden(true);
            if let Some(state) = self.item_glass_states.get_mut(index) {
                *state = None;
            }
        }
    }

    fn update_item_backdrops(&mut self, frame: &RenderFrame) {
        if self.native_glass {
            for effect in &self.item_blurs {
                effect.setHidden(true);
            }
            self.update_item_glass(frame);
        } else {
            for glass in &self.item_glasses {
                glass.setHidden(true);
            }
            self.update_item_blurs(frame);
        }
    }

    /// Reuse a pool of item backdrops so per-item blur can animate without
    /// allocating a new effect view every display frame.
    fn update_item_blurs(&mut self, frame: &RenderFrame) {
        let mut index = 0;
        for item in &frame.items {
            if !item.style.background_drawing || item.style.background_blur_radius <= 0.0 {
                continue;
            }
            let rect = item_background_rect(item, frame, self.height);
            if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
                continue;
            }

            if index >= self.item_blurs.len() {
                let effect =
                    NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(self.mtm), rect);
                effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
                effect.setState(NSVisualEffectState::Active);
                effect.setWantsLayer(true);
                effect.setHidden(false);
                self.item_blur_layer.addSubview(&effect);
                self.item_blurs.push(effect);
            }

            let effect = &self.item_blurs[index];
            effect.setFrame(rect);
            effect.setMaterial(blur_material(item.style.background_blur_radius));
            effect.setAlphaValue(item.style.opacity.clamp(0.0, 1.0));
            effect.setHidden(false);
            if let Some(layer) = effect.layer() {
                layer.setCornerRadius(item_background_layer_radius(item, rect));
                layer.setMasksToBounds(true);
            }
            index += 1;
        }

        for effect in self.item_blurs.iter().skip(index) {
            effect.setHidden(true);
        }
    }

    fn display_frame(&mut self, frame: RenderFrame) {
        self.background_view.set_frame_data(frame.clone());
        self.view.set_frame_data(frame.clone());
        self.update_bar_glass(&frame);
        self.update_item_backdrops(&frame);
    }

    /// Handle a single UI command.
    pub fn handle(&mut self, command: UiCommand) {
        match command {
            UiCommand::Frame(frame) => {
                let frame = *frame;
                let height = frame.bar_bounds.height();
                let width = frame.bar_bounds.width();
                let x = frame.bar_bounds.x();
                let top_margin = match frame.position {
                    BarPosition::Top => frame.bar_bounds.y(),
                    BarPosition::Bottom => {
                        (frame.screen_height - frame.bar_bounds.max_y()).max(0.0)
                    }
                };
                let target_y = self.origin_y(top_margin, height, frame.position);
                if (height - self.height).abs() > 0.5
                    || (width - self.width).abs() > 0.5
                    || (x - self.x).abs() > 0.5
                    || (target_y - self.y).abs() > 0.5
                    || frame.position != self.position
                {
                    self.place(x, top_margin, width, height, frame.position);
                }
                self.update_blur(frame.blur_radius, frame.corner_radius);
                // The floating glass bar casts a soft system shadow. Keep it
                // disabled for opaque, edge-to-edge bars.
                self.window.setHasShadow(frame.blur_radius > 0.0);

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
                        self.display_frame(frame);
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
            UiCommand::Resize { width, height } => {
                self.place(self.x, self.top_margin, width, height, self.position);
            }
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
            self.display_frame(target);
        } else {
            let frame =
                interpolate_frame(&animation.from, &target, ease(target.animation_easing, t));
            self.displayed = Some(frame.clone());
            self.display_frame(frame);
            self.animation = Some(animation);
        }
    }

    fn origin_y(&self, top_margin: f64, height: f64, position: BarPosition) -> f64 {
        window_origin_y(self.visible_frame, top_margin, height, position)
    }

    fn place(&mut self, x: f64, top_margin: f64, width: f64, height: f64, position: BarPosition) {
        let y = self.origin_y(top_margin, height, position);
        self.window.setFrame_display(
            NSRect::new(
                NSPoint::new(self.visible_frame.origin.x + x, y),
                NSSize::new(width, height),
            ),
            true,
        );
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));
        self.blur.setFrame(frame);
        if let Some(glass) = &self.bar_glass {
            glass.setFrame(frame);
        }
        self.background_view.setFrame(frame);
        self.item_blur_layer.setFrame(frame);
        self.view.setFrame(frame);
        self.x = x;
        self.y = y;
        self.top_margin = top_margin;
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
            opacity: 1.0,
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
    fn margins_move_the_window_for_top_and_bottom_bars() {
        let visible = NSRect::new(NSPoint::new(0.0, 25.0), NSSize::new(1440.0, 875.0));

        assert_eq!(
            window_origin_y(visible, 6.0, 44.0, BarPosition::Top),
            25.0 + 875.0 - 44.0 - 6.0
        );
        assert_eq!(
            window_origin_y(visible, 6.0, 44.0, BarPosition::Bottom),
            25.0 + 6.0
        );
    }

    #[test]
    fn native_glass_edge_preserves_proportions() {
        assert!((glass_edge_inset(44.0) - 7.04).abs() < 0.001);
        assert!((glass_edge_inset(30.0) - 4.8).abs() < 0.001);
        assert!((glass_edge_inset(10.0) - 3.0).abs() < 0.001);

        let rect = NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(100.0, 44.0));
        let inner = inset_glass_rect(rect, 4.0);
        assert_eq!(inner.origin.x, 14.0);
        assert_eq!(inner.origin.y, 24.0);
        assert_eq!(inner.size.width, 92.0);
        assert_eq!(inner.size.height, 36.0);

        let tiny = inset_glass_rect(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(4.0, 2.0)),
            10.0,
        );
        assert_eq!(tiny.size.width, 2.0);
        assert_eq!(tiny.size.height, 0.0);
    }

    #[test]
    fn unchanged_frames_skip_repaint() {
        let frame = frame(0.0, Color::WHITE);
        assert!(matches!(
            dirty_region(Some(&frame), &frame, PaintMode::Content, 30.0),
            DirtyRegion::None
        ));
        assert!(matches!(
            dirty_region(Some(&frame), &frame, PaintMode::Background, 30.0),
            DirtyRegion::None
        ));
    }

    #[test]
    fn label_change_invalidates_only_the_item() {
        let previous = frame(0.0, Color::WHITE);
        let mut next = previous.clone();
        next.items[0].label = "2%".into();

        let DirtyRegion::Rect(dirty) =
            dirty_region(Some(&previous), &next, PaintMode::Content, 30.0)
        else {
            panic!("label change should invalidate the content view");
        };
        assert!(dirty.origin.x <= -2.0);
        assert!(dirty.origin.y <= -2.0);
        assert!(dirty.size.width >= 54.0);
        assert!(dirty.size.height >= 34.0);
        assert!(matches!(
            dirty_region(Some(&previous), &next, PaintMode::Background, 30.0),
            DirtyRegion::None
        ));
    }

    #[test]
    fn bar_changes_invalidate_the_background() {
        let previous = frame(0.0, Color::WHITE);
        let mut next = previous.clone();
        next.background = Color::WHITE;
        assert!(matches!(
            dirty_region(Some(&previous), &next, PaintMode::Background, 30.0),
            DirtyRegion::All
        ));

        next = previous.clone();
        next.bar_bounds = Rect::new(0.0, 0.0, 1000.0, 44.0);
        assert!(matches!(
            dirty_region(Some(&previous), &next, PaintMode::Content, 44.0),
            DirtyRegion::All
        ));
    }

    #[test]
    fn progress_strips_reserve_space_without_overlapping_content() {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(100.0, 30.0));
        let mut item = frame(0.0, Color::WHITE).items.remove(0);
        item.style.progress_height = 4.0;

        item.style.progress_position = ProgressPosition::Bottom;
        let strip = progress_strip_rect(&item, rect).unwrap();
        let content = item_content_rect(&item, rect);
        assert_eq!(strip.origin.y, 3.0);
        assert_eq!(strip.size.height, 4.0);
        assert_eq!(content.origin.y, 9.0);
        assert!(content.origin.y >= strip.origin.y + strip.size.height);

        item.style.progress_position = ProgressPosition::Top;
        let strip = progress_strip_rect(&item, rect).unwrap();
        let content = item_content_rect(&item, rect);
        assert_eq!(strip.origin.y + strip.size.height, 27.0);
        assert!(content.origin.y + content.size.height <= strip.origin.y);

        item.style.progress_position = ProgressPosition::Background;
        assert_eq!(item_content_rect(&item, rect), rect);
        assert!(progress_strip_rect(&item, rect).is_some());

        item.style.progress_height = 0.0;
        assert!(progress_strip_rect(&item, rect).is_none());
        assert_eq!(item_content_rect(&item, rect), rect);
    }

    #[test]
    fn progress_values_are_finite_and_clamped() {
        assert_eq!(progress_fraction(0.0), 0.0);
        assert_eq!(progress_fraction(-10.0), 0.0);
        assert_eq!(progress_fraction(42.0), 0.42);
        assert_eq!(progress_fraction(120.0), 1.0);
        assert_eq!(progress_fraction(f64::NAN), 0.0);
        assert_eq!(progress_fraction(f64::INFINITY), 0.0);
    }

    #[test]
    fn content_widths_are_clamped_to_the_item() {
        assert_eq!(fit_content_widths(20.0, 30.0, 100.0), (20.0, 30.0));
        assert_eq!(fit_content_widths(20.0, 100.0, 60.0), (20.0, 40.0));
        assert_eq!(fit_content_widths(100.0, 20.0, 60.0), (60.0, 0.0));
        assert_eq!(fit_content_widths(10.0, 10.0, 0.0), (0.0, 0.0));
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

    #[test]
    fn new_items_fade_in() {
        let mut from = frame(0.0, Color::WHITE);
        from.items.clear();
        let to = frame(0.0, Color::WHITE);

        let midway = interpolate_frame(&from, &to, 0.5);
        let item = &midway.items[0];
        assert_eq!(item.style.opacity, 0.5);
        assert_eq!(item.style.label_color.a, 128);
    }
}
