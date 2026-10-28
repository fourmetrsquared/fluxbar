//! Rendering abstraction and layout engine.
//!
//! The core computes a platform-independent [`RenderFrame`]; platform crates
//! (currently `fluxbar-macos`) only have to paint it.

use crate::config::{BarPosition, Config, EasingFunction, ItemAlign};
use crate::error::{FluxError, Result};
use crate::geometry::{EdgeInsets, Position, Rect, Size};
use crate::item::{Item, ItemKind};
use serde::{Deserialize, Serialize};

/// Straight RGBA color with premultiplied alpha disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const WHITE: Self = Self::rgba(255, 255, 255, 255);
    pub const BLACK: Self = Self::rgba(0, 0, 0, 255);

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Parse `#RRGGBB`, `#RRGGBBAA`, `0xAARRGGBB`, or a named basic color.
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.is_empty() {
            return Err(FluxError::invalid("empty color"));
        }
        if let Some(hex) = value.strip_prefix('#') {
            return Self::parse_rgb_or_rgba(hex);
        }
        if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            return Self::parse_argb(hex);
        }
        let lower = value.to_ascii_lowercase();
        let named = match lower.as_str() {
            "black" => Self::BLACK,
            "white" => Self::WHITE,
            "transparent" | "none" => Self::TRANSPARENT,
            "red" => Self::rgba(255, 59, 48, 255),
            "green" => Self::rgba(52, 199, 89, 255),
            "blue" => Self::rgba(0, 122, 255, 255),
            "yellow" => Self::rgba(255, 204, 0, 255),
            "orange" => Self::rgba(255, 149, 0, 255),
            "purple" => Self::rgba(175, 82, 222, 255),
            "pink" => Self::rgba(255, 45, 85, 255),
            "gray" | "grey" => Self::rgba(142, 142, 147, 255),
            other => {
                if let Some(stripped) = other.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')'))
                {
                    return Self::parse_csv(stripped, false);
                }
                if let Some(stripped) = other
                    .strip_prefix("rgba(")
                    .and_then(|s| s.strip_suffix(')'))
                {
                    return Self::parse_csv(stripped, true);
                }
                return Err(FluxError::invalid(format!("unknown color `{value}`")));
            }
        };
        Ok(named)
    }

    fn parse_rgb_or_rgba(hex: &str) -> Result<Self> {
        match hex.len() {
            6 => {
                let (r, g, b) = Self::split_rgb(hex)?;
                Ok(Self::rgba(r, g, b, 255))
            }
            8 => {
                let (r, g, b) = Self::split_rgb(&hex[0..6])?;
                let a = u8::from_str_radix(&hex[6..8], 16)
                    .map_err(|_| FluxError::invalid(format!("invalid color `#{hex}`")))?;
                Ok(Self::rgba(r, g, b, a))
            }
            _ => Err(FluxError::invalid(format!("invalid color `#{hex}`"))),
        }
    }

    fn parse_argb(hex: &str) -> Result<Self> {
        match hex.len() {
            6 => {
                let (r, g, b) = Self::split_rgb(hex)?;
                Ok(Self::rgba(r, g, b, 255))
            }
            8 => {
                let a = u8::from_str_radix(&hex[0..2], 16)
                    .map_err(|_| FluxError::invalid(format!("invalid color `0x{hex}`")))?;
                let (r, g, b) = Self::split_rgb(&hex[2..8])?;
                Ok(Self::rgba(r, g, b, a))
            }
            _ => Err(FluxError::invalid(format!("invalid color `0x{hex}`"))),
        }
    }

    fn split_rgb(hex: &str) -> Result<(u8, u8, u8)> {
        let r = u8::from_str_radix(&hex[0..2], 16)
            .map_err(|_| FluxError::invalid(format!("invalid color `{hex}`")))?;
        let g = u8::from_str_radix(&hex[2..4], 16)
            .map_err(|_| FluxError::invalid(format!("invalid color `{hex}`")))?;
        let b = u8::from_str_radix(&hex[4..6], 16)
            .map_err(|_| FluxError::invalid(format!("invalid color `{hex}`")))?;
        Ok((r, g, b))
    }

    fn parse_csv(value: &str, with_alpha: bool) -> Result<Self> {
        let parts: Vec<&str> = value.split(',').map(str::trim).collect();
        let expected = if with_alpha { 4 } else { 3 };
        if parts.len() != expected {
            return Err(FluxError::invalid(format!(
                "expected {expected} color components"
            )));
        }
        let channel = |index: usize| -> Result<u8> {
            let parsed = parts[index].parse::<f64>().map_err(|_| {
                FluxError::invalid(format!("invalid color component `{}`", parts[index]))
            })?;
            Ok(parsed.clamp(0.0, 255.0).round() as u8)
        };
        let r = channel(0)?;
        let g = channel(1)?;
        let b = channel(2)?;
        let a = if with_alpha {
            let alpha = parts[3]
                .parse::<f64>()
                .map_err(|_| FluxError::invalid(format!("invalid alpha `{}`", parts[3])))?;
            let alpha = if alpha <= 1.0 { alpha * 255.0 } else { alpha };
            alpha.clamp(0.0, 255.0).round() as u8
        } else {
            255
        };
        Ok(Self::rgba(r, g, b, a))
    }

    /// Convert to a `0xAARRGGBB` string understood by the macOS renderer.
    pub fn to_hex(self) -> String {
        format!("0x{:02X}{:02X}{:02X}{:02X}", self.a, self.r, self.g, self.b)
    }

    pub fn with_alpha(self, alpha: u8) -> Self {
        Self { a: alpha, ..self }
    }

    /// Scale the alpha channel by an opacity in `0.0..=1.0`.
    pub fn with_opacity(self, opacity: f64) -> Self {
        let alpha = (f64::from(self.a) * opacity.clamp(0.0, 1.0))
            .round()
            .clamp(0.0, 255.0) as u8;
        self.with_alpha(alpha)
    }
}

/// Fully resolved style for an item (defaults already applied).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemStyle {
    pub label_color: Color,
    pub label_drawing: bool,
    pub label_font: Option<String>,
    pub label_font_size: f64,
    pub label_padding: EdgeInsets,
    pub icon_color: Color,
    pub icon_drawing: bool,
    pub icon_font: Option<String>,
    pub icon_font_size: f64,
    pub icon_padding: EdgeInsets,
    pub background: Option<Color>,
    pub background_drawing: bool,
    pub background_corner_radius: f64,
    pub background_shape: crate::config::ItemShape,
    pub background_blur_radius: f64,
    pub opacity: f64,
    pub progress_color: Color,
    pub progress_track_color: Color,
    pub progress_height: f64,
    pub progress_position: crate::config::ProgressPosition,
    pub background_height: Option<f64>,
    pub background_padding: EdgeInsets,
    pub border_color: Option<Color>,
    pub border_width: f64,
    pub padding: EdgeInsets,
    pub align: ItemAlign,
    pub width: f64,
    pub min_width: f64,
    pub max_width: f64,
}

impl Default for ItemStyle {
    fn default() -> Self {
        Self {
            label_color: Color::WHITE,
            label_drawing: true,
            label_font: None,
            label_font_size: 13.0,
            label_padding: EdgeInsets::horizontal(4.0),
            icon_color: Color::WHITE,
            icon_drawing: true,
            icon_font: None,
            icon_font_size: 13.0,
            icon_padding: EdgeInsets::horizontal(4.0),
            background: None,
            background_drawing: true,
            background_corner_radius: 0.0,
            background_shape: crate::config::ItemShape::Rounded,
            background_blur_radius: 0.0,
            opacity: 1.0,
            progress_color: Color::WHITE,
            progress_track_color: Color::rgba(255, 255, 255, 40),
            progress_height: 3.0,
            progress_position: crate::config::ProgressPosition::Bottom,
            background_height: None,
            background_padding: EdgeInsets::zero(),
            border_color: None,
            border_width: 0.0,
            padding: EdgeInsets::horizontal(4.0),
            align: ItemAlign::Center,
            width: 0.0,
            min_width: 0.0,
            max_width: 0.0,
        }
    }
}

/// A single drawable item in the frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderItem {
    pub name: String,
    pub kind: ItemKind,
    pub bounds: Rect,
    pub label: String,
    pub icon: String,
    pub progress: Option<f64>,
    pub graph: Vec<f64>,
    pub style: ItemStyle,
}

/// A complete frame ready for a platform renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderFrame {
    /// Bar rectangle in screen coordinates (points).
    pub bar_bounds: Rect,
    pub background: Color,
    /// Bar background opacity, kept separately so overlays such as the glass
    /// sheen fade together with the tinted surface.
    #[serde(default = "default_frame_opacity")]
    pub opacity: f64,
    pub corner_radius: f64,
    pub blur_radius: f64,
    pub items: Vec<RenderItem>,
    /// Bar placement on screen.
    pub position: BarPosition,
    /// State version this frame was produced from.
    pub version: u64,
    /// Screen height (needed to translate top coordinates for AppKit).
    pub screen_height: f64,
    /// Duration of transitions to this frame in milliseconds (`0` disables).
    #[serde(default = "default_animation_ms")]
    pub animation_duration_ms: u64,
    /// Easing applied to transitions.
    #[serde(default)]
    pub animation_easing: EasingFunction,
}

/// Default frame transition duration (milliseconds).
fn default_animation_ms() -> u64 {
    220
}

fn default_frame_opacity() -> f64 {
    1.0
}

/// Platform renderer interface.
pub trait Renderer: Send + Sync {
    /// Present a frame. Called from the UI thread on macOS.
    fn render(&self, frame: &RenderFrame) -> Result<()>;

    /// Show or hide the bar.
    fn set_visible(&self, visible: bool) -> Result<()>;

    /// The window height in points (used for layout).
    fn bar_height(&self) -> f64;

    /// Screen size in points.
    fn screen_size(&self) -> Size;
}

/// Layout configuration.
#[derive(Debug, Clone, Copy)]
pub struct LayoutConfig {
    pub bar_height: f64,
    pub bar_padding: EdgeInsets,
    pub item_spacing: f64,
    pub screen_width: f64,
    pub top_margin: f64,
}

/// Estimate the rendered width of a text string.
///
/// This is a fast heuristic used when no platform text measurement is
/// available. Renderers can adjust the final rect if they have exact metrics.
#[must_use]
pub fn estimate_text_width(text: &str, font_size: f64) -> f64 {
    let mut width = 0.0;
    for ch in text.chars() {
        width += match ch {
            ' ' => 0.30,
            'i' | 'l' | 'j' | 't' | 'f' | 'r' | '.' | ',' | ':' | ';' | '!' | '|' | '\'' => 0.32,
            'm' | 'w' | 'M' | 'W' => 0.95,
            c if c.is_ascii_uppercase() => 0.68,
            c if c.is_ascii_digit() => 0.58,
            c if c.is_ascii() => 0.52,
            // Icons and CJK characters are roughly square.
            _ => 1.0,
        };
    }
    width * font_size
}

/// Exact text metrics supplied by the platform renderer.
///
/// The daemon lays out frames off the UI thread, so it cannot use AppKit's
/// string drawing directly. Platform crates implement this trait with a
/// thread-safe measurement API such as CoreText; headless builds use the
/// heuristic fallback in [`EstimatedTextMetrics`].
pub trait TextMetrics {
    /// Width of `text` in points using the configured font and size.
    fn text_width(&self, text: &str, font: Option<&str>, font_size: f64) -> f64;
}

/// Heuristic metrics used when a platform cannot measure text.
#[derive(Debug, Clone, Copy, Default)]
pub struct EstimatedTextMetrics;

impl TextMetrics for EstimatedTextMetrics {
    fn text_width(&self, text: &str, _font: Option<&str>, font_size: f64) -> f64 {
        estimate_text_width(text, font_size)
    }
}

/// Compute the layout frame for all items.
#[must_use]
pub fn layout(
    state: &crate::state::BarState,
    config: &crate::config::Config,
    screen: Size,
) -> RenderFrame {
    layout_with_metrics(state, config, screen, &EstimatedTextMetrics)
}

/// Compute a layout using exact platform text metrics.
#[must_use]
pub fn layout_with_metrics<M: TextMetrics + ?Sized>(
    state: &crate::state::BarState,
    config: &crate::config::Config,
    screen: Size,
    metrics: &M,
) -> RenderFrame {
    let bar_height = config.bar.height;
    let y = match config.bar.position {
        BarPosition::Top => config.bar.top_margin,
        BarPosition::Bottom => (screen.height - bar_height - config.bar.top_margin).max(0.0),
    };
    let available_width = (screen.width - 2.0 * config.bar.horizontal_margin.max(0.0)).max(0.0);
    let bar_width = if config.bar.full_width {
        available_width
    } else {
        available_width.min(1200.0)
    };
    // A floating capsule stays centered, including the capped non-full-width
    // variant. Keeping the origin in the frame also positions the native
    // window, so layout and painting share one coordinate model.
    let bar_x = (screen.width - bar_width) / 2.0;
    let bar_bounds = Rect::new(bar_x, y, bar_width, bar_height);

    let mut items = Vec::new();
    let background = Color::parse(&config.bar.background)
        .unwrap_or(Color::BLACK)
        .with_opacity(config.bar.opacity);

    let inner = bar_bounds.inset(config.bar.padding);

    for position in [Position::Left, Position::Center, Position::Right] {
        let group: Vec<&Item> = state
            .items_at(position)
            .into_iter()
            .filter(|item| item.state.visible)
            .collect();
        let sizes: Vec<Size> = group
            .iter()
            .map(|item| measure_item_with(item, config, metrics))
            .collect();
        let total_width: f64 = sizes.iter().map(|s| s.width).sum::<f64>()
            + config.bar.item_spacing * (sizes.len().saturating_sub(1) as f64);

        let start_x = match position {
            Position::Left => inner.x(),
            Position::Center => inner.center().x - total_width / 2.0,
            Position::Right => inner.max_x() - total_width,
        };

        let mut cursor = start_x;
        for (item, size) in group.iter().zip(sizes.iter()) {
            let bounds = Rect::new(cursor, inner.y(), size.width, inner.height());
            items.push(render_item(item, config, bounds));
            cursor += size.width + config.bar.item_spacing;
        }
    }

    RenderFrame {
        bar_bounds,
        background,
        opacity: config.bar.opacity,
        corner_radius: config.bar.corner_radius,
        blur_radius: config.bar.blur_radius,
        items,
        position: config.bar.position,
        version: state.version,
        screen_height: screen.height,
        animation_duration_ms: if config.animations.disabled {
            0
        } else {
            config.animations.duration
        },
        animation_easing: config.animations.easing,
    }
}

fn render_item(item: &Item, config: &Config, bounds: Rect) -> RenderItem {
    let style = resolve_style(item, config);
    RenderItem {
        name: item.name.clone(),
        kind: item.kind,
        bounds,
        label: item.state.label.clone(),
        icon: item.state.icon.clone(),
        progress: item.state.progress,
        graph: item.state.graph.clone(),
        style,
    }
}

/// Resolve an item's style using bar defaults.
#[must_use]
pub fn resolve_style(item: &Item, config: &Config) -> ItemStyle {
    let item_config = &item.config;
    let opacity = item_config.opacity.clamp(0.0, 1.0);
    let label_color = Color::parse(&item_config.label_color)
        .or_else(|_| Color::parse(&config.bar.foreground))
        .unwrap_or(Color::WHITE)
        .with_opacity(opacity);
    let progress_color = item_config
        .progress_color
        .as_deref()
        .and_then(|color| Color::parse(color).ok())
        .map(|color| color.with_opacity(opacity))
        .unwrap_or(label_color);
    let progress_track_color = item_config
        .progress_track_color
        .as_deref()
        .and_then(|color| Color::parse(color).ok())
        .map(|color| color.with_opacity(opacity))
        .unwrap_or_else(|| {
            let alpha = (f64::from(progress_color.a) * 0.16)
                .round()
                .clamp(0.0, 255.0) as u8;
            progress_color.with_alpha(alpha)
        });
    let progress_height = item_config.progress_height.unwrap_or(3.0).clamp(0.0, 64.0);
    let icon_color = Color::parse(&item_config.icon_color)
        .or_else(|_| Color::parse(&config.bar.foreground))
        .unwrap_or(Color::WHITE)
        .with_opacity(opacity);
    let background = item_config
        .background_drawing
        .then_some(item_config.background.as_deref())
        .flatten()
        .and_then(|c| Color::parse(c).ok())
        .map(|color| color.with_opacity(opacity));
    let border_color = item_config
        .border_color
        .as_deref()
        .and_then(|c| Color::parse(c).ok())
        .map(|color| color.with_opacity(opacity));

    ItemStyle {
        label_color,
        label_drawing: item_config.label_drawing,
        label_font: item_config
            .label_font
            .clone()
            .or_else(|| config.bar.font.clone()),
        label_font_size: if item_config.label_font_size > 0.0 {
            item_config.label_font_size
        } else {
            config.bar.font_size
        },
        label_padding: item_config.label_padding,
        icon_color,
        icon_drawing: item_config.icon_drawing,
        icon_font: item_config
            .icon_font
            .clone()
            .or_else(|| config.bar.icon_font.clone()),
        icon_font_size: if item_config.icon_font_size > 0.0 {
            item_config.icon_font_size
        } else {
            config.bar.font_size
        },
        icon_padding: item_config.icon_padding,
        background,
        background_drawing: item_config.background_drawing,
        background_corner_radius: item_config.background_corner_radius,
        background_shape: item_config.background_shape,
        background_blur_radius: item_config.background_blur_radius,
        opacity,
        progress_color,
        progress_track_color,
        progress_height,
        progress_position: item_config.progress_position.unwrap_or_default(),
        background_height: item_config.background_height,
        background_padding: item_config.background_padding,
        border_color,
        border_width: item_config.border_width,
        padding: item_config.padding,
        align: item_config.align,
        width: item_config.width,
        min_width: item_config.min_width,
        max_width: item_config.max_width,
    }
}

/// Measure an item's preferred size using estimated text metrics.
#[must_use]
pub fn measure_item(item: &Item, config: &Config) -> Size {
    measure_item_with(item, config, &EstimatedTextMetrics)
}

/// Measure an item's preferred size with platform text metrics.
#[must_use]
pub fn measure_item_with<M: TextMetrics + ?Sized>(
    item: &Item,
    config: &Config,
    metrics: &M,
) -> Size {
    let style = resolve_style(item, config);
    let label_width = if item.state.label.is_empty() || !style.label_drawing {
        0.0
    } else {
        metrics.text_width(
            &item.state.label,
            style.label_font.as_deref(),
            style.label_font_size,
        ) + style.label_padding.horizontal_sum()
    };
    let icon_width = if item.state.icon.is_empty() || !style.icon_drawing {
        0.0
    } else {
        metrics.text_width(
            &item.state.icon,
            style.icon_font.as_deref(),
            style.icon_font_size,
        ) + style.icon_padding.horizontal_sum()
    };
    let content_width = label_width + icon_width + style.padding.horizontal_sum();

    let width = if style.width > 0.0 {
        style.width
    } else {
        content_width
    };
    let width = width.max(style.min_width);
    let width = if style.max_width > 0.0 {
        width.min(style.max_width)
    } else {
        width
    };

    Size::new(width, config.bar.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BarConfig, Config};

    #[test]
    fn parses_colors() {
        assert_eq!(Color::parse("#FFFFFF").unwrap(), Color::WHITE);
        assert_eq!(Color::parse("#00000000").unwrap(), Color::TRANSPARENT);
        assert_eq!(
            Color::parse("0xFF112233").unwrap(),
            Color::rgba(0x11, 0x22, 0x33, 0xFF)
        );
        assert_eq!(Color::parse("red").unwrap(), Color::rgba(255, 59, 48, 255));
        assert_eq!(
            Color::parse("rgba(1,2,3,0.5)").unwrap(),
            Color::rgba(1, 2, 3, 128)
        );
        assert!(Color::parse("nope").is_err());
    }

    #[test]
    fn layout_places_groups() {
        use crate::item::Item;
        use crate::state::BarState;

        let config = Config::default();
        let mut state = BarState::new();
        let mut left = Item::new("left", config.item("left"));
        left.position = Position::Left;
        left.state.label = "L".into();
        let mut right = Item::new("right", config.item("right"));
        right.position = Position::Right;
        right.state.label = "R".into();
        let mut center = Item::new("center", config.item("center"));
        center.position = Position::Center;
        center.state.label = "C".into();

        state.items.insert("left".into(), left);
        state.items.insert("right".into(), right);
        state.items.insert("center".into(), center);

        let frame = layout(&state, &config, Size::new(1920.0, 1080.0));
        assert_eq!(frame.items.len(), 3);
        let left_item = frame.items.iter().find(|i| i.name == "left").unwrap();
        let right_item = frame.items.iter().find(|i| i.name == "right").unwrap();
        let center_item = frame.items.iter().find(|i| i.name == "center").unwrap();

        assert!(left_item.bounds.x() < center_item.bounds.x());
        assert!(center_item.bounds.x() < right_item.bounds.x());
        assert!((right_item.bounds.max_x() - (1920.0 - config.bar.padding.right)).abs() < 1.0);
    }

    #[test]
    fn text_width_estimate() {
        assert!(estimate_text_width("WWW", 13.0) > estimate_text_width("iii", 13.0));
    }

    struct FixedMetrics;

    impl TextMetrics for FixedMetrics {
        fn text_width(&self, text: &str, _font: Option<&str>, _font_size: f64) -> f64 {
            text.chars().count() as f64 * 10.0
        }
    }

    #[test]
    fn layout_uses_platform_text_metrics() {
        use crate::item::Item;

        let config = Config::default();
        let mut item_config = config.item("metrics");
        item_config.label_padding = EdgeInsets::horizontal(4.0);
        item_config.padding = EdgeInsets::horizontal(4.0);
        let mut item = Item::new("metrics", item_config);
        item.state.label = "AB".into();

        let size = measure_item_with(&item, &config, &FixedMetrics);
        // Two 10pt characters, plus 4pt label padding and 4pt item padding on
        // each side.
        assert!((size.width - 36.0).abs() < 0.01);

        let mut state = crate::state::BarState::new();
        state.items.insert("metrics".into(), item);
        let frame = layout_with_metrics(&state, &config, Size::new(1000.0, 800.0), &FixedMetrics);
        let bounds = frame.items[0].bounds;
        assert!((bounds.width() - 36.0).abs() < 0.01);
    }

    #[test]
    fn floating_bar_is_centered_with_margins() {
        let config = Config {
            bar: BarConfig {
                top_margin: 6.0,
                horizontal_margin: 16.0,
                ..BarConfig::default()
            },
            ..Config::default()
        };
        let state = crate::state::BarState::new();
        let frame = layout(&state, &config, Size::new(1000.0, 800.0));
        assert!((frame.bar_bounds.x() - 16.0).abs() < 0.01);
        assert!((frame.bar_bounds.y() - 6.0).abs() < 0.01);
        assert!((frame.bar_bounds.width() - 968.0).abs() < 0.01);
    }

    #[test]
    fn capped_bar_is_centered() {
        let config = Config {
            bar: BarConfig {
                full_width: false,
                ..BarConfig::default()
            },
            ..Config::default()
        };
        let state = crate::state::BarState::new();
        let frame = layout(&state, &config, Size::new(1920.0, 1080.0));
        assert!((frame.bar_bounds.width() - 1200.0).abs() < 0.01);
        assert!((frame.bar_bounds.x() - 360.0).abs() < 0.01);
    }

    #[test]
    fn bar_can_be_bottom_positioned() {
        let config = Config {
            bar: BarConfig {
                position: BarPosition::Bottom,
                height: 30.0,
                top_margin: 5.0,
                ..BarConfig::default()
            },
            ..Config::default()
        };
        let state = crate::state::BarState::new();
        let frame = layout(&state, &config, Size::new(1000.0, 800.0));
        assert!((frame.bar_bounds.y() - 765.0).abs() < 1.0);
    }

    #[test]
    fn item_opacity_shape_and_blur_are_resolved() {
        use crate::config::ItemShape;
        use crate::item::Item;

        let config = Config::default();
        let mut item_config = config.item("tile");
        item_config.background = Some("0xFF112233".into());
        item_config.opacity = 0.5;
        item_config.background_shape = ItemShape::Circle;
        item_config.background_blur_radius = 18.0;
        let item = Item::new("tile", item_config.clone());

        let style = resolve_style(&item, &config);
        assert_eq!(style.opacity, 0.5);
        assert_eq!(style.background_shape, ItemShape::Circle);
        assert_eq!(style.background_blur_radius, 18.0);
        assert!(style.background_drawing);
        assert_eq!(style.background.unwrap().a, 128);
        assert_eq!(style.label_color.a, 128);

        item_config.background_drawing = false;
        let hidden = Item::new("hidden", item_config);
        let style = resolve_style(&hidden, &config);
        assert!(!style.background_drawing);
        assert!(style.background.is_none());
    }

    #[test]
    fn progress_style_defaults_to_the_label_color() {
        use crate::config::ProgressPosition;
        use crate::item::Item;

        let config = Config::default();
        let mut item_config = config.item("progress");
        item_config.label_color = "0xFF112233".into();
        item_config.progress_height = Some(5.5);
        item_config.progress_position = Some(ProgressPosition::Top);
        let item = Item::new("progress", item_config.clone());

        let style = resolve_style(&item, &config);
        assert_eq!(style.progress_color, Color::rgba(0x11, 0x22, 0x33, 255));
        assert_eq!(
            style.progress_track_color,
            Color::rgba(0x11, 0x22, 0x33, 41)
        );
        assert_eq!(style.progress_height, 5.5);
        assert_eq!(style.progress_position, ProgressPosition::Top);

        item_config.progress_color = Some("0xFFABCDEF".into());
        item_config.progress_track_color = Some("0xFF102030".into());
        item_config.opacity = 0.5;
        let item = Item::new("progress", item_config);
        let style = resolve_style(&item, &config);
        assert_eq!(style.progress_color, Color::rgba(0xAB, 0xCD, 0xEF, 128));
        assert_eq!(
            style.progress_track_color,
            Color::rgba(0x10, 0x20, 0x30, 128)
        );
    }

    #[test]
    fn layout_carries_animation_settings() {
        let state = crate::state::BarState::new();
        let mut config = Config::default();
        config.animations.duration = 480;
        config.animations.easing = EasingFunction::EaseOutExpo;
        let frame = layout(&state, &config, Size::new(1000.0, 800.0));
        assert_eq!(frame.animation_duration_ms, 480);
        assert_eq!(frame.animation_easing, EasingFunction::EaseOutExpo);

        config.animations.disabled = true;
        let frame = layout(&state, &config, Size::new(1000.0, 800.0));
        assert_eq!(frame.animation_duration_ms, 0);
    }
}
