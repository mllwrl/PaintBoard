//! Reactions people put on objects, and their icons.
//!
//! Icons are drawn as paths rather than emoji, so they need no emoji font.

use std::f64::consts::{FRAC_PI_2, PI};

use kurbo::{BezPath, Circle, Point, Rect, Shape};

use crate::Rgba;

/// Reactions on one object, each with who put it there.
pub type Reactions = Vec<(Reaction, Vec<std::sync::Arc<str>>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Reaction {
    Plus,
    Heart,
    Smile,
    Star,
    Check,
    Exclaim,
}

impl Reaction {
    pub const ALL: [Reaction; 6] = [
        Self::Plus,
        Self::Heart,
        Self::Smile,
        Self::Star,
        Self::Check,
        Self::Exclaim,
    ];

    /// Name stored in the document. Never change existing names.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plus => "plus",
            Self::Heart => "heart",
            Self::Smile => "smile",
            Self::Star => "star",
            Self::Check => "check",
            Self::Exclaim => "exclaim",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }

    /// For people: tooltips and the like.
    pub fn name(self) -> &'static str {
        match self {
            Self::Plus => "Agree",
            Self::Heart => "Love it",
            Self::Smile => "Ha",
            Self::Star => "Important",
            Self::Check => "Done",
            Self::Exclaim => "Attention",
        }
    }

    pub fn color(self) -> Rgba {
        match self {
            Self::Plus => Rgba::rgb(0x25, 0x63, 0xEB),
            Self::Heart => Rgba::rgb(0xE5, 0x48, 0x4D),
            Self::Smile => Rgba::rgb(0xEA, 0x8A, 0x0C),
            Self::Star => Rgba::rgb(0xF5, 0x9E, 0x0B),
            Self::Check => Rgba::rgb(0x16, 0xA3, 0x4A),
            Self::Exclaim => Rgba::rgb(0xDC, 0x26, 0x26),
        }
    }

    /// The icon in a square `rect`: a path to fill and a path to stroke with a line
    /// `rect.width() / 8` wide (either may be empty).
    pub fn icon(self, rect: Rect) -> (BezPath, BezPath) {
        let s = rect.width().min(rect.height());
        let c = rect.center();
        let at = |x: f64, y: f64| Point::new(c.x + x * s, c.y + y * s);
        let mut fill = BezPath::new();
        let mut stroke = BezPath::new();
        match self {
            Self::Plus => {
                stroke.move_to(at(-0.32, 0.0));
                stroke.line_to(at(0.32, 0.0));
                stroke.move_to(at(0.0, -0.32));
                stroke.line_to(at(0.0, 0.32));
            }
            Self::Heart => {
                fill.move_to(at(0.0, 0.38));
                fill.curve_to(at(-0.55, 0.0), at(-0.42, -0.5), at(0.0, -0.18));
                fill.curve_to(at(0.42, -0.5), at(0.55, 0.0), at(0.0, 0.38));
                fill.close_path();
            }
            Self::Smile => {
                stroke.extend(Circle::new(c, s * 0.4).path_elements(0.1));
                fill.extend(Circle::new(at(-0.14, -0.1), s * 0.06).path_elements(0.1));
                fill.extend(Circle::new(at(0.14, -0.1), s * 0.06).path_elements(0.1));
                stroke.move_to(at(-0.2, 0.08));
                stroke.quad_to(at(0.0, 0.32), at(0.2, 0.08));
            }
            Self::Star => {
                for i in 0..10 {
                    let r = if i % 2 == 0 { 0.45 } else { 0.19 };
                    let a = -FRAC_PI_2 + f64::from(i) * PI / 5.0;
                    let p = at(r * a.cos(), r * a.sin() + 0.04);
                    if i == 0 {
                        fill.move_to(p);
                    } else {
                        fill.line_to(p);
                    }
                }
                fill.close_path();
            }
            Self::Check => {
                stroke.move_to(at(-0.32, 0.02));
                stroke.line_to(at(-0.08, 0.26));
                stroke.line_to(at(0.34, -0.24));
            }
            Self::Exclaim => {
                fill.extend(
                    kurbo::RoundedRect::from_rect(
                        Rect::from_points(at(-0.07, -0.4), at(0.07, 0.14)),
                        s * 0.07,
                    )
                    .path_elements(0.1),
                );
                fill.extend(Circle::new(at(0.0, 0.32), s * 0.08).path_elements(0.1));
            }
        }
        (fill, stroke)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_icons_stay_in_their_box() {
        let rect = Rect::new(10.0, 10.0, 30.0, 30.0);
        for r in Reaction::ALL {
            assert_eq!(Reaction::parse(r.as_str()), Some(r));
            let (fill, stroke) = r.icon(rect);
            assert!(!fill.is_empty() || !stroke.is_empty(), "{r:?}");
            for path in [fill, stroke] {
                if !path.is_empty() {
                    let b = path.bounding_box();
                    assert!(rect.inflate(1.0, 1.0).contains_rect(b), "{r:?}: {b:?}");
                }
            }
        }
    }
}
