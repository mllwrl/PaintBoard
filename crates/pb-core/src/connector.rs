//! Connectors: lines between two points or objects that follow the objects they attach to.
//!
//! A connector stores only its ends; its path is worked out from the objects' current
//! positions (see [`layout`]), so moving an object moves the connectors attached to it.

use std::sync::Arc;

use kurbo::{BezPath, Point, Rect, Shape, Size, Vec2};
use pb_geom::connector::{self as geom, ray_exit, side_normal};
pub use pb_geom::connector::{Cap, ConnectorPath, EndPoint, Route};
use pb_geom::rotate;

use crate::{ObjectData, ObjectId, ObjectKind};

/// Where on an object a connector end attaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// Wherever the outline faces the other end; it moves as the objects move.
    Auto,
    /// A fixed point, as fractions of the object's width and height, projected onto its
    /// outline (the side midpoints are Miro's connection points).
    Fixed(Point),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum End {
    Free(Point),
    Attached {
        object: ObjectId,
        anchor: Anchor,
        /// Last known position, used while the object is missing (deleted, not yet synced).
        at: Point,
    },
}

impl End {
    pub fn object(&self) -> Option<ObjectId> {
        match self {
            Self::Attached { object, .. } => Some(*object),
            Self::Free(_) => None,
        }
    }

    /// A free end's point, or an attached end's last known position.
    pub fn point(&self) -> Point {
        match *self {
            Self::Free(p) | Self::Attached { at: p, .. } => p,
        }
    }

    pub(crate) fn translate(&mut self, delta: Vec2) {
        match self {
            Self::Free(p) | Self::Attached { at: p, .. } => *p += delta,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Connector {
    pub start: End,
    pub end: End,
    pub route: Route,
    pub start_cap: Cap,
    pub end_cap: Cap,
    pub label: Label,
    /// Points the line runs through, set by hand (world); empty for the automatic path.
    /// They move with the connector, not with the objects it attaches to.
    pub bends: Arc<[Point]>,
}

/// Text on a connector's line, in the connector's color. The line breaks around it.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub text: Arc<str>,
    /// Where its middle sits, as a fraction of the line's length from the start.
    pub at: f64,
    /// Size of the laid-out text, in world units. The document cannot lay out text, so
    /// whoever edits the label measures it.
    pub size: Size,
}

impl Label {
    /// Font size in world units.
    pub const FONT_SIZE: f64 = 16.0;
    /// Gap between the text and the line around it, in world units.
    pub const PADDING: f64 = 4.0;

    /// An empty label at `at`.
    pub fn empty(at: f64) -> Self {
        Self {
            text: Arc::from(""),
            at,
            size: Size::ZERO,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The box of the text on `path`; `None` without text.
    pub fn rect(&self, path: &ConnectorPath) -> Option<Rect> {
        (!self.is_empty()).then(|| Rect::from_center_size(path.point_at(self.at), self.size))
    }
}

impl Default for Label {
    fn default() -> Self {
        Self::empty(0.5)
    }
}

/// A partial connector edit: only the set fields change.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConnectorChange {
    pub start: Option<End>,
    pub end: Option<End>,
    pub route: Option<Route>,
    pub start_cap: Option<Cap>,
    pub end_cap: Option<Cap>,
}

impl ConnectorChange {
    pub fn apply_to(&self, c: &Connector) -> Connector {
        Connector {
            start: self.start.unwrap_or(c.start),
            end: self.end.unwrap_or(c.end),
            route: self.route.unwrap_or(c.route),
            start_cap: self.start_cap.unwrap_or(c.start_cap),
            end_cap: self.end_cap.unwrap_or(c.end_cap),
            label: c.label.clone(),
            // Bends made for one kind of path make no sense on another.
            bends: match self.route {
                Some(route) if route != c.route => Arc::from([]),
                _ => c.bends.clone(),
            },
        }
    }
}

/// Whether connectors can attach to objects of this kind (everything but connectors).
pub fn can_attach(kind: &ObjectKind) -> bool {
    !matches!(kind, ObjectKind::Connector(_) | ObjectKind::Frame(_))
}

/// An object's outline in its unrotated coordinates.
fn outline(data: &ObjectData) -> BezPath {
    match &data.kind {
        ObjectKind::Shape(shape) => shape.kind.outline(data.bounds),
        _ => data.bounds.to_path(0.1),
    }
}

/// The point where the outline meets the ray from the object's center through `local`
/// (unrotated coordinates), with the outward normal of the nearest side, in world space.
fn on_outline(data: &ObjectData, local: Point) -> EndPoint {
    let frame = data.frame();
    let b = data.bounds;
    let point = ray_exit(&outline(data), b.center(), local).unwrap_or(local);
    EndPoint {
        point: frame.to_world(point),
        normal: Some(rotate(side_normal(b, local), frame.rotation)),
    }
}

/// A fraction of the object's bounds, as an unrotated point.
fn fraction_point(bounds: Rect, f: Point) -> Point {
    Point::new(
        bounds.x0 + f.x * bounds.width(),
        bounds.y0 + f.y * bounds.height(),
    )
}

/// The four connection points of an object: the anchors of its side midpoints and where
/// they are on the board.
pub fn connection_points(data: &ObjectData) -> [(Anchor, Point); 4] {
    [(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)].map(|(u, v)| {
        let f = Point::new(u, v);
        let local = fraction_point(data.bounds, f);
        (Anchor::Fixed(f), on_outline(data, local).point)
    })
}

/// Rough position of an end, to aim the other end at: a free point, a fixed anchor, or the
/// center of the object for an automatic one.
fn aim<'a>(end: &End, lookup: &impl Fn(ObjectId) -> Option<&'a ObjectData>) -> Point {
    match *end {
        End::Free(p) => p,
        End::Attached { object, anchor, at } => match lookup(object) {
            Some(data) => match anchor {
                Anchor::Auto => data.bounds.center(),
                Anchor::Fixed(f) => on_outline(data, fraction_point(data.bounds, f)).point,
            },
            None => at,
        },
    }
}

fn resolve_end<'a>(
    end: &End,
    other: Point,
    route: Route,
    lookup: &impl Fn(ObjectId) -> Option<&'a ObjectData>,
) -> EndPoint {
    let End::Attached { object, anchor, at } = *end else {
        return EndPoint::free(end.point());
    };
    let Some(data) = lookup(object) else {
        return EndPoint::free(at);
    };
    let b = data.bounds;
    let local = match anchor {
        Anchor::Fixed(f) => fraction_point(b, f),
        Anchor::Auto => {
            let toward = data.frame().to_local(other);
            match route {
                // Straight lines aim at each other's centers.
                Route::Straight => toward,
                // Elbows and curves leave from the middle of the side facing the other end.
                Route::Elbow | Route::Curved => {
                    let n = side_normal(b, toward);
                    b.center() + Vec2::new(n.x * b.width(), n.y * b.height()) / 2.0
                }
            }
        }
    };
    on_outline(data, local)
}

/// Both ends of a connector where they are now. `lookup` gives objects' current data (the
/// document's, or mid-gesture overrides).
pub fn resolve<'a>(
    c: &Connector,
    lookup: impl Fn(ObjectId) -> Option<&'a ObjectData>,
) -> (EndPoint, EndPoint) {
    let start = resolve_end(&c.start, aim(&c.end, &lookup), c.route, &lookup);
    let end = resolve_end(&c.end, aim(&c.start, &lookup), c.route, &lookup);
    (start, end)
}

/// The drawable path of a connector object, with its resolved ends. Elbow connectors go
/// around the obstacles (see [`obstacle`]) `obstacles_in` finds in an area; only elbow
/// connectors ask.
pub fn layout<'a>(
    c: &Connector,
    width: f64,
    lookup: impl Fn(ObjectId) -> Option<&'a ObjectData>,
    obstacles_in: impl FnOnce(Rect) -> Vec<Rect>,
) -> (EndPoint, EndPoint, ConnectorPath) {
    let (start, end) = resolve(c, lookup);
    let obstacles = if c.route == Route::Elbow && c.bends.is_empty() {
        obstacles_in(geom::obstacle_area(start.point, end.point))
    } else {
        Vec::new()
    };
    let caps = (c.start_cap, c.end_cap);
    let path = geom::layout(c.route, start, end, caps, width, &obstacles, &c.bends);
    (start, end, path)
}

/// What elbow connectors go around for an object: its world bounds, for shapes, sticky
/// notes, text and images. Ink, connectors and frames are crossed.
pub fn obstacle(data: &ObjectData) -> Option<Rect> {
    match data.kind {
        ObjectKind::Shape(_)
        | ObjectKind::Sticky(_)
        | ObjectKind::Text(_)
        | ObjectKind::Image(_)
        | ObjectKind::Table(_) => Some(data.frame().aabb()),
        ObjectKind::Ink(_) | ObjectKind::Connector(_) | ObjectKind::Frame(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use pb_geom::ShapeKind;

    use super::*;
    use crate::Style;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> ObjectData {
        ObjectData {
            kind: ShapeKind::Rectangle.into(),
            bounds: Rect::new(x, y, x + w, y + h),
            rotation: 0.0,
            style: Style::default(),
        }
    }

    fn ids() -> (ObjectId, ObjectId) {
        let mut doc = crate::Document::new();
        let made = doc
            .apply(crate::Command::Create(vec![
                rect(0.0, 0.0, 1.0, 1.0).into(),
                rect(0.0, 0.0, 1.0, 1.0).into(),
            ]))
            .unwrap()
            .created;
        (made[0], made[1])
    }

    fn attached(object: ObjectId, anchor: Anchor) -> End {
        End::Attached {
            object,
            anchor,
            at: Point::ZERO,
        }
    }

    #[test]
    fn straight_auto_ends_meet_the_outlines_facing_each_other() {
        let (a, b) = ids();
        let objects = HashMap::from([
            (a, rect(0.0, 0.0, 100.0, 100.0)),
            (b, rect(300.0, 0.0, 100.0, 100.0)),
        ]);
        let c = Connector {
            bends: Default::default(),
            start: attached(a, Anchor::Auto),
            end: attached(b, Anchor::Auto),
            route: Route::Straight,
            start_cap: Cap::None,
            end_cap: Cap::Arrow,
            label: Label::default(),
        };
        let (s, e) = resolve(&c, |id| objects.get(&id));
        assert!((s.point - Point::new(100.0, 50.0)).hypot() < 1e-6);
        assert!((e.point - Point::new(300.0, 50.0)).hypot() < 1e-6);
    }

    #[test]
    fn elbow_auto_ends_use_side_midpoints() {
        let (a, b) = ids();
        let objects = HashMap::from([
            (a, rect(0.0, 0.0, 100.0, 100.0)),
            (b, rect(300.0, 400.0, 100.0, 100.0)),
        ]);
        let c = Connector {
            bends: Default::default(),
            start: attached(a, Anchor::Auto),
            end: attached(b, Anchor::Auto),
            route: Route::Elbow,
            start_cap: Cap::None,
            end_cap: Cap::None,
            label: Label::default(),
        };
        let (s, e) = resolve(&c, |id| objects.get(&id));
        // B is mostly below A: A's bottom side and B's top side.
        assert!((s.point - Point::new(50.0, 100.0)).hypot() < 1e-6);
        assert_eq!(s.normal, Some(Vec2::new(0.0, 1.0)));
        assert!((e.point - Point::new(350.0, 400.0)).hypot() < 1e-6);
    }

    #[test]
    fn fixed_anchors_follow_rotation_and_missing_objects_fall_back() {
        let (a, b) = ids();
        let mut turned = rect(0.0, 0.0, 100.0, 50.0);
        turned.rotation = std::f64::consts::FRAC_PI_2;
        let objects = HashMap::from([(a, turned)]);
        let c = Connector {
            bends: Default::default(),
            start: attached(a, Anchor::Fixed(Point::new(1.0, 0.5))),
            end: End::Attached {
                object: b,
                anchor: Anchor::Auto,
                at: Point::new(500.0, 500.0),
            },
            route: Route::Straight,
            start_cap: Cap::None,
            end_cap: Cap::None,
            label: Label::default(),
        };
        let (s, e) = resolve(&c, |id| objects.get(&id));
        // The right side's middle, turned a quarter clockwise about the center (50, 25).
        assert!((s.point - Point::new(50.0, 75.0)).hypot() < 1e-6, "{s:?}");
        let n = s.normal.unwrap();
        assert!((n - Vec2::new(0.0, 1.0)).hypot() < 1e-9);
        assert_eq!(e.point, Point::new(500.0, 500.0));
    }

    #[test]
    fn connection_points_sit_on_the_outline() {
        let mut triangle = rect(0.0, 0.0, 100.0, 60.0);
        triangle.kind = ShapeKind::Triangle.into();
        let points = connection_points(&triangle);
        // The triangle's apex is the top midpoint; its left side's middle is inside the
        // bounds, on the slanted edge.
        assert!((points[0].1 - Point::new(50.0, 0.0)).hypot() < 1e-6);
        assert!(points[3].1.x > 0.0 && points[3].1.x < 50.0);
    }
}
