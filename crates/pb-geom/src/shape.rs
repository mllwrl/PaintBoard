use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI, SQRT_2, TAU};

use kurbo::{Affine, BezPath, Ellipse, ParamCurveNearest, Point, Rect, RoundedRect, Shape, Vec2};

/// Geometric shapes. Each one is drawn to fill its object's (unrotated) bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Diamond,
    Triangle,
    Star,
    /// Block arrow pointing right; rotate it for other directions.
    Arrow,
    DoubleArrow,
    Cloud,
}

/// Corner radius of a rounded rectangle, relative to its shorter side.
const ROUNDING: f64 = 0.15;
/// Inner radius of the star relative to the outer one (a regular pentagram).
const STAR_INNER: f64 = 0.382;
/// Arrow shaft thickness relative to the arrow's height.
const ARROW_SHAFT: f64 = 0.5;
/// Number of bumps around a cloud.
const CLOUD_BUMPS: usize = 9;

impl ShapeKind {
    pub const ALL: [ShapeKind; 9] = [
        Self::Rectangle,
        Self::RoundedRectangle,
        Self::Ellipse,
        Self::Diamond,
        Self::Triangle,
        Self::Star,
        Self::Arrow,
        Self::DoubleArrow,
        Self::Cloud,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::RoundedRectangle => "Rounded rectangle",
            Self::Ellipse => "Ellipse",
            Self::Diamond => "Diamond",
            Self::Triangle => "Triangle",
            Self::Star => "Star",
            Self::Arrow => "Arrow",
            Self::DoubleArrow => "Double arrow",
            Self::Cloud => "Cloud",
        }
    }

    /// The next kind in [`ShapeKind::ALL`], wrapping around.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&k| k == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Closed outline filling `rect`.
    pub fn outline(self, rect: Rect) -> BezPath {
        let rect = rect.abs();
        let (w, h) = (rect.width(), rect.height());
        let c = rect.center();
        match self {
            Self::Rectangle => rect.to_path(0.1),
            Self::RoundedRectangle => {
                RoundedRect::from_rect(rect, w.min(h) * ROUNDING).to_path(0.1)
            }
            Self::Ellipse => Ellipse::from_rect(rect).to_path(0.1),
            Self::Diamond => polygon(&[
                Point::new(c.x, rect.y0),
                Point::new(rect.x1, c.y),
                Point::new(c.x, rect.y1),
                Point::new(rect.x0, c.y),
            ]),
            Self::Triangle => polygon(&[
                Point::new(c.x, rect.y0),
                Point::new(rect.x1, rect.y1),
                Point::new(rect.x0, rect.y1),
            ]),
            Self::Star => {
                let points: Vec<_> = (0..10)
                    .map(|i| {
                        let r = if i % 2 == 0 { 1.0 } else { STAR_INNER };
                        let a = -FRAC_PI_2 + i as f64 * PI / 5.0;
                        Point::new(r * a.cos(), r * a.sin())
                    })
                    .collect();
                fit(polygon(&points), rect)
            }
            Self::Arrow => {
                let head = (w * 0.5).min(h * 0.8);
                let (s0, s1) = shaft(rect);
                let neck = rect.x1 - head;
                polygon(&[
                    Point::new(rect.x0, s0),
                    Point::new(neck, s0),
                    Point::new(neck, rect.y0),
                    Point::new(rect.x1, c.y),
                    Point::new(neck, rect.y1),
                    Point::new(neck, s1),
                    Point::new(rect.x0, s1),
                ])
            }
            Self::DoubleArrow => {
                let head = (w * 0.35).min(h * 0.8);
                let (s0, s1) = shaft(rect);
                let (left, right) = (rect.x0 + head, rect.x1 - head);
                polygon(&[
                    Point::new(rect.x0, c.y),
                    Point::new(left, rect.y0),
                    Point::new(left, s0),
                    Point::new(right, s0),
                    Point::new(right, rect.y0),
                    Point::new(rect.x1, c.y),
                    Point::new(right, rect.y1),
                    Point::new(right, s1),
                    Point::new(left, s1),
                    Point::new(left, rect.y1),
                ])
            }
            Self::Cloud => fit(cloud(), rect),
        }
    }

    /// The largest convenient rectangle inside the shape filling `rect`, where its text
    /// goes. Centered on the shape's body, so a triangle's text sits low and an arrow's on
    /// its shaft.
    pub fn text_area(self, rect: Rect) -> Rect {
        let rect = rect.abs();
        let (w, h) = (rect.width(), rect.height());
        let c = rect.center();
        // Width and height as fractions of the bounds, around the center.
        let centered = |fw: f64, fh: f64| Rect::from_center_size(c, (w * fw, h * fh));
        match self {
            Self::Rectangle => rect,
            Self::RoundedRectangle => {
                let r = w.min(h) * ROUNDING * (1.0 - FRAC_1_SQRT_2);
                rect.inset(-r)
            }
            Self::Ellipse => centered(FRAC_1_SQRT_2, FRAC_1_SQRT_2),
            Self::Diamond => centered(0.5, 0.5),
            // The widest box under the apex: half the width, the lower half.
            Self::Triangle => Rect::new(rect.x0 + w / 4.0, c.y, rect.x1 - w / 4.0, rect.y1),
            Self::Star => {
                // A square in the inner pentagon: around the star's center, which is below
                // the middle of its bounds.
                let side = STAR_INNER * (PI / 5.0).cos() * SQRT_2;
                let (sw, sh) = (2.0 * (PI / 10.0).cos(), 1.0 + (PI / 5.0).cos());
                let center = Point::new(c.x, rect.y0 + h / sh);
                Rect::from_center_size(center, (w * side / sw, h * side / sh))
            }
            Self::Arrow => {
                let head = (w * 0.5).min(h * 0.8);
                let (s0, s1) = shaft(rect);
                Rect::new(rect.x0, s0, rect.x1 - head / 2.0, s1)
            }
            Self::DoubleArrow => {
                let head = (w * 0.35).min(h * 0.8);
                let (s0, s1) = shaft(rect);
                Rect::new(rect.x0 + head / 2.0, s0, rect.x1 - head / 2.0, s1)
            }
            Self::Cloud => centered(0.6, 0.55),
        }
    }

    /// Whether `point` (in the same space as `rect`) is on the shape or within `pad` of
    /// its outline.
    pub fn hit(self, rect: Rect, point: Point, pad: f64) -> bool {
        if !rect.abs().inflate(pad, pad).contains(point) {
            return false;
        }
        if self == Self::Rectangle {
            return true;
        }
        let path = self.outline(rect);
        path.contains(point)
            || path
                .segments()
                .any(|seg| seg.nearest(point, 1e-3).distance_sq <= pad * pad)
    }
}

/// Top and bottom of an arrow's shaft.
fn shaft(rect: Rect) -> (f64, f64) {
    let half = rect.height() * ARROW_SHAFT / 2.0;
    (rect.center().y - half, rect.center().y + half)
}

fn polygon(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for &p in &points[1..] {
        path.line_to(p);
    }
    path.close_path();
    path
}

/// Scales and moves `path` so that its bounding box is `rect`.
fn fit(path: BezPath, rect: Rect) -> BezPath {
    let from = path.bounding_box();
    let scale = Vec2::new(
        rect.width() / from.width().max(f64::EPSILON),
        rect.height() / from.height().max(f64::EPSILON),
    );
    let affine = Affine::translate(rect.origin().to_vec2())
        * Affine::scale_non_uniform(scale.x, scale.y)
        * Affine::translate(-from.origin().to_vec2());
    affine * path
}

/// Unit cloud: bumps bulging out between points on an ellipse.
fn cloud() -> BezPath {
    let at = |i: usize| {
        let a = i as f64 * TAU / CLOUD_BUMPS as f64;
        Point::new(a.cos(), 0.8 * a.sin())
    };
    let mut path = BezPath::new();
    path.move_to(at(0));
    for i in 0..CLOUD_BUMPS {
        let (p0, p1) = (at(i), at(i + 1));
        // Control points pushed outward from the chord give a round bump.
        let out = p0.midpoint(p1).to_vec2() * 0.45;
        path.curve_to(p0 + out, p1 + out, p1);
    }
    path.close_path();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect::new(10.0, 20.0, 110.0, 80.0);

    #[test]
    fn every_outline_fills_its_bounds() {
        for kind in ShapeKind::ALL {
            let bbox = kind.outline(R).bounding_box();
            for (a, b) in [
                (bbox.x0, R.x0),
                (bbox.y0, R.y0),
                (bbox.x1, R.x1),
                (bbox.y1, R.y1),
            ] {
                assert!((a - b).abs() < 1e-6, "{kind:?}: {bbox:?}");
            }
        }
    }

    #[test]
    fn hit_follows_the_outline() {
        let corner = Point::new(12.0, 22.0);
        assert!(ShapeKind::Rectangle.hit(R, corner, 0.0));
        assert!(!ShapeKind::Ellipse.hit(R, corner, 0.0));
        assert!(!ShapeKind::Diamond.hit(R, corner, 0.0));
        assert!(ShapeKind::Ellipse.hit(R, R.center(), 0.0));
        // Just outside the ellipse's left end, but within the padding.
        assert!(ShapeKind::Ellipse.hit(R, Point::new(8.0, 50.0), 3.0));
        assert!(!ShapeKind::Ellipse.hit(R, Point::new(8.0, 50.0), 1.0));
    }

    #[test]
    fn text_areas_lie_inside_their_shapes() {
        for kind in ShapeKind::ALL {
            let area = kind.text_area(R);
            assert!(area.width() > 0.0 && area.height() > 0.0, "{kind:?}");
            // Corners pulled in a hair, as some of them touch the outline.
            let inner = area.inset(-1e-6);
            for corner in [
                Point::new(inner.x0, inner.y0),
                Point::new(inner.x1, inner.y0),
                Point::new(inner.x1, inner.y1),
                Point::new(inner.x0, inner.y1),
            ] {
                assert!(kind.outline(R).contains(corner), "{kind:?}: {area:?}");
            }
        }
    }

    #[test]
    fn arrow_head_and_shaft() {
        // Above the shaft near the tail is empty; the tip is solid.
        assert!(!ShapeKind::Arrow.hit(R, Point::new(15.0, 25.0), 0.0));
        assert!(ShapeKind::Arrow.hit(R, Point::new(105.0, 50.0), 0.0));
        assert!(!ShapeKind::DoubleArrow.hit(R, Point::new(60.0, 25.0), 0.0));
    }

    #[test]
    fn kinds_cycle() {
        let mut kind = ShapeKind::Rectangle;
        for _ in 0..ShapeKind::ALL.len() {
            kind = kind.next();
        }
        assert_eq!(kind, ShapeKind::Rectangle);
    }
}
