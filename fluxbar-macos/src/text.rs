//! Shared AppKit/CoreText text metrics.
//!
//! Layout runs on the daemon thread, so it cannot call `NSStringDrawing`
//! directly. CoreText gives the same typographic metrics as AppKit while
//! remaining safe to use off the main thread.

#![allow(unsafe_code)]

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSFont, NSFontAttributeName};
use objc2_core_foundation::CFAttributedString;
use objc2_core_text::CTLine;
use objc2_foundation::{NSAttributedString, NSAttributedStringKey, NSDictionary, NSString};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TextKey {
    text: String,
    font: Option<String>,
    font_size_bits: u64,
}

thread_local! {
    /// Painting measures labels every animation tick. CoreText line creation
    /// is significantly more expensive than a hash lookup, so keep a small
    /// per-thread cache of resolved widths.
    static TEXT_WIDTHS: RefCell<HashMap<TextKey, f64>> = RefCell::new(HashMap::new());
}

/// Resolve a configured font name with the same fallback AppKit uses when the
/// font is unavailable.
pub(crate) fn ns_font(name: Option<&str>, size: f64) -> Retained<NSFont> {
    if let Some(name) = name
        && let Some(font) = NSFont::fontWithName_size(&NSString::from_str(name), size)
    {
        return font;
    }
    NSFont::systemFontOfSize(size)
}

/// Measure `text` with the exact font used for painting.
fn measure_text(text: &str, font: &NSFont) -> f64 {
    if text.is_empty() {
        return 0.0;
    }

    let string = NSString::from_str(text);
    // SAFETY: The key is AppKit's documented font attribute and the value is
    // an NSFont, which is the expected object type.
    let attributes = unsafe {
        let keys: [&NSAttributedStringKey; 1] = [NSFontAttributeName];
        let values: [&AnyObject; 1] = [font.as_ref()];
        NSDictionary::from_slices(&keys, &values)
    };
    // SAFETY: `initWithString:attributes:` receives a valid string and an
    // attribute dictionary containing only supported keys.
    let attributed = unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &string,
            Some(&attributes),
        )
    };
    let attributed: &CFAttributedString = attributed.as_ref();
    // SAFETY: CoreText creates an owned line for the valid attributed string.
    let line = unsafe { CTLine::with_attributed_string(attributed) };
    // SAFETY: Passing null pointers means we only need the return value, the
    // typographic width.
    unsafe { line.typographic_bounds(ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) as f64 }
        .max(0.0)
}

/// Resolve a font by configuration name and measure `text` with it.
pub(crate) fn measure_text_with_font(text: &str, font_name: Option<&str>, font_size: f64) -> f64 {
    let key = TextKey {
        text: text.to_string(),
        font: font_name.map(str::to_string),
        font_size_bits: font_size.to_bits(),
    };
    if let Some(width) = TEXT_WIDTHS.with(|cache| cache.borrow().get(&key).copied()) {
        return width;
    }

    let font = ns_font(font_name, font_size);
    let width = measure_text(text, &font);
    TEXT_WIDTHS.with(|cache| {
        let mut cache = cache.borrow_mut();
        // Long-running bars can see many changing labels. A bounded cache
        // keeps the fast path without retaining every historical string.
        if cache.len() >= 1024 {
            cache.clear();
        }
        cache.insert(key, width);
    });
    width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_requested_font_metrics() {
        let wide = measure_text_with_font("WWWW", None, 13.0);
        let narrow = measure_text_with_font("iiii", None, 13.0);
        assert!(wide > narrow);
        assert!(narrow > 0.0);
    }

    #[test]
    fn measures_unicode_and_long_text() {
        for text in ["Привет", "日本語", "👋🏽", "e\u{301}", &"x".repeat(512)] {
            assert!(measure_text_with_font(text, None, 13.0) > 0.0, "{text:?}");
        }
    }

    #[test]
    fn missing_font_falls_back_to_the_system_font() {
        assert!(measure_text_with_font("fallback", Some("Definitely Not A Font"), 13.0) > 0.0);
    }
}
