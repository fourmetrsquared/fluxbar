//! Geometry primitives used for layout and rendering.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A 2D point.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub const fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// A 2D size.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    pub const fn zero() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
        }
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// A rectangle described by an origin and a size.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Point::new(x, y),
            size: Size::new(width, height),
        }
    }

    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    pub const fn x(&self) -> f64 {
        self.origin.x
    }

    pub const fn y(&self) -> f64 {
        self.origin.y
    }

    pub const fn width(&self) -> f64 {
        self.size.width
    }

    pub const fn height(&self) -> f64 {
        self.size.height
    }

    pub fn min_x(&self) -> f64 {
        self.origin.x
    }

    pub fn max_x(&self) -> f64 {
        self.origin.x + self.size.width
    }

    pub fn min_y(&self) -> f64 {
        self.origin.y
    }

    pub fn max_y(&self) -> f64 {
        self.origin.y + self.size.height
    }

    pub fn center(&self) -> Point {
        Point::new(
            self.origin.x + self.size.width / 2.0,
            self.origin.y + self.size.height / 2.0,
        )
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.min_x()
            && point.x <= self.max_x()
            && point.y >= self.min_y()
            && point.y <= self.max_y()
    }

    /// Compute the intersection with another rectangle.
    #[must_use]
    pub fn intersect(&self, other: Rect) -> Option<Rect> {
        let min_x = self.min_x().max(other.min_x());
        let min_y = self.min_y().max(other.min_y());
        let max_x = self.max_x().min(other.max_x());
        let max_y = self.max_y().min(other.max_y());

        (min_x < max_x && min_y < max_y)
            .then(|| Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
    }

    /// Smallest rectangle containing both inputs.
    #[must_use]
    pub fn union(&self, other: Rect) -> Rect {
        let min_x = self.min_x().min(other.min_x());
        let min_y = self.min_y().min(other.min_y());
        let max_x = self.max_x().max(other.max_x());
        let max_y = self.max_y().max(other.max_y());

        Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    /// Shrink the rectangle by the given insets.
    #[must_use]
    pub fn inset(&self, insets: EdgeInsets) -> Rect {
        Rect::new(
            self.origin.x + insets.left,
            self.origin.y + insets.top,
            (self.size.width - insets.left - insets.right).max(0.0),
            (self.size.height - insets.top - insets.bottom).max(0.0),
        )
    }

    /// Translate the rectangle.
    #[must_use]
    pub fn offset(&self, dx: f64, dy: f64) -> Rect {
        Rect::new(
            self.origin.x + dx,
            self.origin.y + dy,
            self.size.width,
            self.size.height,
        )
    }

    /// Interpolate between two rectangles (`t` in `0..=1`).
    #[must_use]
    pub fn lerp(&self, other: Rect, t: f64) -> Rect {
        Rect::new(
            lerp(self.origin.x, other.origin.x, t),
            lerp(self.origin.y, other.origin.y, t),
            lerp(self.size.width, other.size.width, t),
            lerp(self.size.height, other.size.height, t),
        )
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.origin, self.size)
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Edge insets (padding / margin).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct EdgeInsets {
    #[serde(default)]
    pub top: f64,
    #[serde(default)]
    pub left: f64,
    #[serde(default)]
    pub bottom: f64,
    #[serde(default)]
    pub right: f64,
}

impl EdgeInsets {
    pub const fn new(top: f64, left: f64, bottom: f64, right: f64) -> Self {
        Self {
            top,
            left,
            bottom,
            right,
        }
    }

    pub const fn uniform(value: f64) -> Self {
        Self::new(value, value, value, value)
    }

    pub const fn horizontal(value: f64) -> Self {
        Self::new(0.0, value, 0.0, value)
    }

    pub const fn vertical(value: f64) -> Self {
        Self::new(value, 0.0, value, 0.0)
    }

    pub const fn zero() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0)
    }

    pub const fn horizontal_sum(&self) -> f64 {
        self.left + self.right
    }

    pub const fn vertical_sum(&self) -> f64 {
        self.top + self.bottom
    }
}

/// Which side of the bar an item belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    #[default]
    Left,
    Center,
    Right,
}

impl Position {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

impl std::str::FromStr for Position {
    type Err = FluxError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "left" => Ok(Self::Left),
            "center" | "centre" => Ok(Self::Center),
            "right" => Ok(Self::Right),
            other => Err(FluxError::invalid(format!("unknown position `{other}`"))),
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

use crate::error::FluxError;

/// Anchor point used when placing the bar on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    #[default]
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains() {
        let rect = Rect::new(0.0, 0.0, 100.0, 30.0);
        assert!(rect.contains(Point::new(50.0, 15.0)));
        assert!(!rect.contains(Point::new(150.0, 15.0)));
    }

    #[test]
    fn rect_intersection() {
        let a = Rect::new(0.0, 0.0, 100.0, 30.0);
        let b = Rect::new(50.0, 10.0, 100.0, 30.0);
        let intersection = a.intersect(b).expect("rectangles should intersect");
        assert_eq!(intersection, Rect::new(50.0, 10.0, 50.0, 20.0));
        assert!(a.intersect(Rect::new(200.0, 200.0, 5.0, 5.0)).is_none());
    }

    #[test]
    fn rect_lerp() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(10.0, 10.0, 20.0, 20.0);
        assert_eq!(a.lerp(b, 0.5), Rect::new(5.0, 5.0, 15.0, 15.0));
    }

    #[test]
    fn position_parsing() {
        assert_eq!("left".parse::<Position>().unwrap(), Position::Left);
        assert_eq!("CENTER".parse::<Position>().unwrap(), Position::Center);
        assert!("middle".parse::<Position>().is_err());
    }
}
