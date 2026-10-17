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
    // Diagram shapes.
    /// Flowchart start and end: a rectangle with round ends.
    Terminator,
    /// Flowchart data: slanted sides.
    Parallelogram,
    /// Flowchart manual operation: narrower at the top.
    Trapezoid,
    /// Flowchart preparation.
    Hexagon,
    /// A page with a wavy bottom edge.
    Document,
    /// A rectangle with a line inside each end.
    PredefinedProcess,
    /// A rectangle whose top slopes up to the right.
    ManualInput,
    /// A database or data store.
    Cylinder,
    /// Flat on the left, round on the right.
    Delay,
    /// A pentagon pointing down, continuing a flowchart elsewhere.
    OffPage,
    /// A page with a folded corner (UML note, BPMN data object).
    Note,
    /// A folder with a tab (UML package).
    Package,
    /// A box with name, attributes and operations compartments (UML class, ER table).
    UmlClass,
    /// A stick figure (UML actor, a user).
    Actor,
    /// A BPMN pool: a band for its name on the left.
    Pool,
    /// A rack server.
    Server,
    /// A computer screen on a stand.
    Monitor,
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
    /// Every kind: the basic ones first.
    pub const ALL: [ShapeKind; 26] = [
        Self::Rectangle,
        Self::RoundedRectangle,
        Self::Ellipse,
        Self::Diamond,
        Self::Triangle,
        Self::Star,
        Self::Arrow,
        Self::DoubleArrow,
        Self::Cloud,
        Self::Terminator,
        Self::Parallelogram,
        Self::Trapezoid,
        Self::Hexagon,
        Self::Document,
        Self::PredefinedProcess,
        Self::ManualInput,
        Self::Cylinder,
        Self::Delay,
        Self::OffPage,
        Self::Note,
        Self::Package,
        Self::UmlClass,
        Self::Actor,
        Self::Pool,
        Self::Server,
        Self::Monitor,
    ];

    /// The kinds the shape key (S) cycles through.
    pub const BASIC: [ShapeKind; 9] = [
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
            Self::Terminator => "Terminator",
            Self::Parallelogram => "Parallelogram",
            Self::Trapezoid => "Trapezoid",
            Self::Hexagon => "Hexagon",
            Self::Document => "Document",
            Self::PredefinedProcess => "Predefined process",
            Self::ManualInput => "Manual input",
            Self::Cylinder => "Cylinder",
            Self::Delay => "Delay",
            Self::OffPage => "Off-page connector",
            Self::Note => "Note",
            Self::Package => "Package",
            Self::UmlClass => "Class",
            Self::Actor => "Actor",
            Self::Pool => "Pool",
            Self::Server => "Server",
            Self::Monitor => "Monitor",
        }
    }

    /// The next basic kind (see [`ShapeKind::BASIC`]), wrapping around; the first one for
    /// the others.
    pub fn next(self) -> Self {
        match Self::BASIC.iter().position(|&k| k == self) {
            Some(i) => Self::BASIC[(i + 1) % Self::BASIC.len()],
            None => Self::BASIC[0],
        }
    }

    /// Width and height of a shape placed with a click, in world units.
    pub fn default_size(self) -> (f64, f64) {
        match self {
            Self::Terminator | Self::Parallelogram | Self::Trapezoid | Self::Hexagon => {
                (160.0, 70.0)
            }
            Self::Document
            | Self::PredefinedProcess
            | Self::ManualInput
            | Self::Delay
            | Self::Note
            | Self::Package => (150.0, 100.0),
            Self::Cylinder => (100.0, 120.0),
            Self::OffPage => (80.0, 90.0),
            Self::UmlClass => (180.0, 160.0),
            Self::Actor => (60.0, 110.0),
            Self::Pool => (480.0, 180.0),
            Self::Server => (90.0, 120.0),
            Self::Monitor => (130.0, 110.0),
            Self::Arrow | Self::DoubleArrow => (160.0, 80.0),
            _ => (100.0, 100.0),
        }
    }

    /// Lines drawn inside the shape on top of its fill, in the outline color: the
    /// compartments of a class, the rim of a cylinder, an actor's body. `None` for most.
    pub fn details(self, rect: Rect) -> Option<BezPath> {
        let rect = rect.abs();
        let (w, h) = (rect.width(), rect.height());
        let c = rect.center();
        let mut path = BezPath::new();
        let mut line = |a: Point, b: Point| {
            path.move_to(a);
            path.line_to(b);
        };
        match self {
            Self::PredefinedProcess => {
                for x in [rect.x0 + w * SIDE_BAND, rect.x1 - w * SIDE_BAND] {
                    line(Point::new(x, rect.y0), Point::new(x, rect.y1));
                }
            }
            Self::Cylinder => {
                let e = cylinder_rim(rect);
                let mut rim = BezPath::new();
                rim.move_to(Point::new(rect.x0, rect.y0 + e));
                half_ellipse(&mut rim, rect.x0, rect.x1, rect.y0 + e, e, false);
                return Some(rim);
            }
            Self::Note => {
                let f = note_fold(rect);
                path.move_to(Point::new(rect.x1 - f, rect.y0));
                path.line_to(Point::new(rect.x1 - f, rect.y0 + f));
                path.line_to(Point::new(rect.x1, rect.y0 + f));
            }
            Self::Package => {
                let (tw, t) = package_tab(rect);
                line(
                    Point::new(rect.x0, rect.y0 + t),
                    Point::new(rect.x0 + tw, rect.y0 + t),
                );
            }
            Self::UmlClass => {
                for f in CLASS_LINES {
                    let y = rect.y0 + h * f;
                    line(Point::new(rect.x0, y), Point::new(rect.x1, y));
                }
            }
            Self::Pool => {
                let x = rect.x0 + pool_band(rect);
                line(Point::new(x, rect.y0), Point::new(x, rect.y1));
            }
            Self::Server => {
                for i in 1..3 {
                    let y = rect.y0 + h * f64::from(i) / 3.0;
                    line(Point::new(rect.x0, y), Point::new(rect.x1, y));
                }
                for i in 0..3 {
                    let y = rect.y0 + h * (f64::from(i) + 0.5) / 3.0;
                    line(
                        Point::new(rect.x1 - w * 0.3, y),
                        Point::new(rect.x1 - w * 0.12, y),
                    );
                }
            }
            Self::Actor => {
                let (head, r) = actor_head(rect);
                let neck = Point::new(c.x, head.y + r);
                let hip = Point::new(c.x, rect.y0 + h * 0.66);
                let shoulders = rect.y0 + h * 0.4;
                line(neck, hip);
                line(
                    Point::new(rect.x0, shoulders),
                    Point::new(rect.x1, shoulders),
                );
                line(hip, Point::new(rect.x0 + w * 0.08, rect.y1));
                line(hip, Point::new(rect.x1 - w * 0.08, rect.y1));
            }
            _ => return None,
        }
        Some(path)
    }

    /// Whether the shape is a figure drawn mostly with lines (an actor), hit anywhere in
    /// its bounds and with its text not inside its outline.
    pub fn is_figure(self) -> bool {
        self == Self::Actor
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
            Self::Terminator => RoundedRect::from_rect(rect, w.min(h) / 2.0).to_path(0.1),
            Self::Parallelogram => {
                let s = slant(rect);
                polygon(&[
                    Point::new(rect.x0 + s, rect.y0),
                    Point::new(rect.x1, rect.y0),
                    Point::new(rect.x1 - s, rect.y1),
                    Point::new(rect.x0, rect.y1),
                ])
            }
            Self::Trapezoid => {
                let s = slant(rect);
                polygon(&[
                    Point::new(rect.x0 + s, rect.y0),
                    Point::new(rect.x1 - s, rect.y0),
                    Point::new(rect.x1, rect.y1),
                    Point::new(rect.x0, rect.y1),
                ])
            }
            Self::Hexagon => {
                let s = hexagon_cut(rect);
                polygon(&[
                    Point::new(rect.x0 + s, rect.y0),
                    Point::new(rect.x1 - s, rect.y0),
                    Point::new(rect.x1, c.y),
                    Point::new(rect.x1 - s, rect.y1),
                    Point::new(rect.x0 + s, rect.y1),
                    Point::new(rect.x0, c.y),
                ])
            }
            Self::Document => {
                let a = h * WAVE;
                let y = rect.y1 - a;
                let mut path = BezPath::new();
                path.move_to(Point::new(rect.x0, rect.y0));
                path.line_to(Point::new(rect.x1, rect.y0));
                path.line_to(Point::new(rect.x1, y));
                // Up on the right half, down on the left; the dip reaches the bottom.
                path.quad_to(
                    Point::new((rect.x1 + c.x) / 2.0, y - 2.0 * a),
                    Point::new(c.x, y),
                );
                path.quad_to(
                    Point::new((c.x + rect.x0) / 2.0, y + 2.0 * a),
                    Point::new(rect.x0, y),
                );
                path.close_path();
                path
            }
            Self::PredefinedProcess | Self::UmlClass | Self::Pool | Self::Server => {
                rect.to_path(0.1)
            }
            Self::ManualInput => polygon(&[
                Point::new(rect.x0, rect.y0 + h * MANUAL_SLOPE),
                Point::new(rect.x1, rect.y0),
                Point::new(rect.x1, rect.y1),
                Point::new(rect.x0, rect.y1),
            ]),
            Self::Cylinder => {
                let e = cylinder_rim(rect);
                let mut path = BezPath::new();
                path.move_to(Point::new(rect.x0, rect.y0 + e));
                half_ellipse(&mut path, rect.x0, rect.x1, rect.y0 + e, e, true);
                path.line_to(Point::new(rect.x1, rect.y1 - e));
                half_ellipse(&mut path, rect.x1, rect.x0, rect.y1 - e, e, false);
                path.close_path();
                path
            }
            Self::Delay => {
                let r = (w / 2.0).min(h / 2.0);
                let k = KAPPA;
                let x = rect.x1 - r;
                let mut path = BezPath::new();
                path.move_to(Point::new(rect.x0, rect.y0));
                path.line_to(Point::new(x, rect.y0));
                let ry = h / 2.0;
                path.curve_to(
                    Point::new(x + k * r, rect.y0),
                    Point::new(rect.x1, c.y - k * ry),
                    Point::new(rect.x1, c.y),
                );
                path.curve_to(
                    Point::new(rect.x1, c.y + k * ry),
                    Point::new(x + k * r, rect.y1),
                    Point::new(x, rect.y1),
                );
                path.line_to(Point::new(rect.x0, rect.y1));
                path.close_path();
                path
            }
            Self::OffPage => polygon(&[
                Point::new(rect.x0, rect.y0),
                Point::new(rect.x1, rect.y0),
                Point::new(rect.x1, rect.y0 + h * OFF_PAGE_BODY),
                Point::new(c.x, rect.y1),
                Point::new(rect.x0, rect.y0 + h * OFF_PAGE_BODY),
            ]),
            Self::Note => {
                let f = note_fold(rect);
                polygon(&[
                    Point::new(rect.x0, rect.y0),
                    Point::new(rect.x1 - f, rect.y0),
                    Point::new(rect.x1, rect.y0 + f),
                    Point::new(rect.x1, rect.y1),
                    Point::new(rect.x0, rect.y1),
                ])
            }
            Self::Package => {
                let (tw, t) = package_tab(rect);
                polygon(&[
                    Point::new(rect.x0, rect.y0),
                    Point::new(rect.x0 + tw, rect.y0),
                    Point::new(rect.x0 + tw, rect.y0 + t),
                    Point::new(rect.x1, rect.y0 + t),
                    Point::new(rect.x1, rect.y1),
                    Point::new(rect.x0, rect.y1),
                ])
            }
            // The head; the body is in `details`.
            Self::Actor => {
                let (head, r) = actor_head(rect);
                kurbo::Circle::new(head, r).to_path(0.1)
            }
            Self::Monitor => {
                let screen = Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + h * SCREEN);
                let mut path = RoundedRect::from_rect(screen, w.min(h) * 0.04).to_path(0.1);
                let neck = w * 0.08;
                let foot = w * 0.25;
                path.move_to(Point::new(c.x - neck, screen.y1));
                path.line_to(Point::new(c.x + neck, screen.y1));
                path.line_to(Point::new(c.x + foot, rect.y1));
                path.line_to(Point::new(c.x - foot, rect.y1));
                path.close_path();
                path
            }
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
            // Like a rounded rectangle whose corners are half its height.
            Self::Terminator => rect.inset(-w.min(h) / 2.0 * (1.0 - FRAC_1_SQRT_2)),
            Self::Parallelogram | Self::Trapezoid => {
                let s = slant(rect);
                Rect::new(rect.x0 + s, rect.y0, rect.x1 - s, rect.y1)
            }
            Self::Hexagon => {
                let s = hexagon_cut(rect);
                Rect::new(rect.x0 + s, rect.y0, rect.x1 - s, rect.y1)
            }
            Self::Document => Rect::new(rect.x0, rect.y0, rect.x1, rect.y1 - 2.0 * h * WAVE),
            Self::PredefinedProcess => rect.inset((-w * SIDE_BAND, 0.0)),
            Self::ManualInput => Rect::new(rect.x0, rect.y0 + h * MANUAL_SLOPE, rect.x1, rect.y1),
            Self::Cylinder => {
                let e = cylinder_rim(rect);
                Rect::new(rect.x0, rect.y0 + 2.0 * e, rect.x1, rect.y1 - e)
            }
            // Out to 45° on the round end.
            Self::Delay => {
                let r = (w / 2.0).min(h / 2.0);
                let dy = h / 2.0 * (1.0 - FRAC_1_SQRT_2);
                let x = rect.x1 - r + r * FRAC_1_SQRT_2;
                Rect::new(rect.x0, rect.y0 + dy, x, rect.y1 - dy)
            }
            Self::OffPage => Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + h * OFF_PAGE_BODY),
            Self::Note => Rect::new(rect.x0, rect.y0 + note_fold(rect), rect.x1, rect.y1),
            Self::Package => Rect::new(rect.x0, rect.y0 + package_tab(rect).1, rect.x1, rect.y1),
            // The name compartment.
            Self::UmlClass => Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + h * CLASS_LINES[0]),
            Self::Pool => Rect::new(rect.x0 + pool_band(rect), rect.y0, rect.x1, rect.y1),
            Self::Server => Rect::new(rect.x0, rect.y0, rect.x1 - w * 0.32, rect.y1),
            // Under the arms, between the legs.
            Self::Actor => Rect::new(rect.x0, rect.y0 + h * 0.7, rect.x1, rect.y1),
            Self::Monitor => {
                let screen = Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + h * SCREEN);
                screen.inset(-w.min(h) * 0.06)
            }
        }
    }

    /// Whether `point` (in the same space as `rect`) is on the shape or within `pad` of
    /// its outline.
    pub fn hit(self, rect: Rect, point: Point, pad: f64) -> bool {
        if !rect.abs().inflate(pad, pad).contains(point) {
            return false;
        }
        if self == Self::Rectangle || self.is_figure() {
            return true;
        }
        let path = self.outline(rect);
        path.contains(point)
            || path
                .segments()
                .any(|seg| seg.nearest(point, 1e-3).distance_sq <= pad * pad)
    }
}

/// Kappa: control point distance for a quarter circle drawn with one cubic.
const KAPPA: f64 = 0.552_284_749_8;
/// Parallelogram and trapezoid slant, relative to the width.
const SLANT: f64 = 0.2;
/// Document wave height, relative to the height.
const WAVE: f64 = 0.08;
/// Predefined process side bands, relative to the width.
const SIDE_BAND: f64 = 0.1;
/// Manual input: how far down the top starts on the left, relative to the height.
const MANUAL_SLOPE: f64 = 0.3;
/// Off-page connector: the rectangular part, relative to the height.
const OFF_PAGE_BODY: f64 = 0.62;
/// UML class compartment lines, relative to the height.
const CLASS_LINES: [f64; 2] = [0.28, 0.62];
/// Monitor: the screen's share of the height.
const SCREEN: f64 = 0.75;

fn slant(rect: Rect) -> f64 {
    (rect.width() * SLANT).min(rect.height())
}

fn hexagon_cut(rect: Rect) -> f64 {
    (rect.width() * 0.25).min(rect.height() * 0.5)
}

/// Half the height of a cylinder's top and bottom ellipses.
fn cylinder_rim(rect: Rect) -> f64 {
    (rect.height() * 0.12).min(rect.width() * 0.3)
}

fn note_fold(rect: Rect) -> f64 {
    rect.width().min(rect.height()) * 0.2
}

/// Width and height of a package's tab.
fn package_tab(rect: Rect) -> (f64, f64) {
    (rect.width() * 0.4, rect.height() * 0.15)
}

fn pool_band(rect: Rect) -> f64 {
    (rect.width() * 0.08).min(48.0_f64.max(rect.width() * 0.04))
}

/// Center and radius of an actor's head.
fn actor_head(rect: Rect) -> (Point, f64) {
    let r = (rect.width() * 0.22).min(rect.height() * 0.13);
    (Point::new(rect.center().x, rect.y0 + r), r)
}

/// Appends half an ellipse from x `from` to x `to` around height `y` with vertical
/// radius `ry`: over the top (`up`) or under the bottom.
fn half_ellipse(path: &mut BezPath, from: f64, to: f64, y: f64, ry: f64, up: bool) {
    let cx = (from + to) / 2.0;
    let rx = (to - from) / 2.0;
    let dy = if up { -ry } else { ry };
    let k = KAPPA;
    path.curve_to(
        Point::new(from, y + k * dy),
        Point::new(cx - k * rx, y + dy),
        Point::new(cx, y + dy),
    );
    path.curve_to(
        Point::new(cx + k * rx, y + dy),
        Point::new(to, y + k * dy),
        Point::new(to, y),
    );
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
            let mut bbox = kind.outline(R).bounding_box();
            if let Some(details) = kind.details(R) {
                bbox = bbox.union(details.bounding_box());
            }
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
        for kind in ShapeKind::ALL.into_iter().filter(|k| !k.is_figure()) {
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
        for _ in 0..ShapeKind::BASIC.len() {
            kind = kind.next();
        }
        assert_eq!(kind, ShapeKind::Rectangle);
        assert_eq!(ShapeKind::Cylinder.next(), ShapeKind::Rectangle);
    }
}
