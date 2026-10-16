//! Quick create, like Miro's: "+" dots around a selected shape or sticky note that add a
//! copy of it on that side, connected to it.

use pb_core::{
    Anchor, Cap, Connector, Document, End, Label, ObjectData, ObjectId, ObjectKind, Route, Shape,
    Sticky,
};
use pb_geom::kurbo::{Point, Rect, Vec2};

/// Distance of the dots from the object's edges, in logical pixels: beyond the rotation
/// knob, so the two do not get in each other's way.
pub const DOT_OFFSET: f64 = 44.0;
/// Radius of a dot, and how near a press has to be, in logical pixels.
pub const DOT_RADIUS: f64 = 8.0;
/// Space between an object and its quick copy, in logical pixels.
const GAP: f64 = 80.0;

/// A side of an object, in its own (unrotated) frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    pub const ALL: [Self; 4] = [Self::North, Self::East, Self::South, Self::West];

    /// Outward direction in the object's frame.
    fn direction(self) -> Vec2 {
        match self {
            Self::North => Vec2::new(0.0, -1.0),
            Self::East => Vec2::new(1.0, 0.0),
            Self::South => Vec2::new(0.0, 1.0),
            Self::West => Vec2::new(-1.0, 0.0),
        }
    }

    fn opposite(self) -> Self {
        match self {
            Self::North => Self::South,
            Self::East => Self::West,
            Self::South => Self::North,
            Self::West => Self::East,
        }
    }

    /// The middle of this side, as fractions of the object's width and height.
    fn anchor(self) -> Point {
        let d = self.direction();
        Point::new(0.5 + d.x / 2.0, 0.5 + d.y / 2.0)
    }
}

/// Whether quick create works on objects of `kind`.
fn can_copy(kind: &ObjectKind) -> bool {
    matches!(kind, ObjectKind::Shape(_) | ObjectKind::Sticky(_))
}

/// The "+" dots around object `id` at `zoom`, in world space; none for other kinds.
pub fn dots(doc: &Document, id: ObjectId, zoom: f64) -> Vec<(Side, Point)> {
    let Some(object) = doc.get(id).filter(|o| can_copy(&o.data.kind)) else {
        return Vec::new();
    };
    let frame = object.data.frame();
    let rect = object.data.bounds;
    Side::ALL
        .into_iter()
        .map(|side| {
            let edge = edge_middle(rect, side);
            let local = edge + side.direction() * (DOT_OFFSET / zoom);
            (side, frame.to_world(local))
        })
        .collect()
}

fn edge_middle(rect: Rect, side: Side) -> Point {
    let a = side.anchor();
    Point::new(rect.x0 + a.x * rect.width(), rect.y0 + a.y * rect.height())
}

/// The dot of object `id` under `point` at `zoom`.
pub fn dot_at(doc: &Document, id: ObjectId, point: Point, zoom: f64) -> Option<Side> {
    let radius = DOT_RADIUS / zoom;
    dots(doc, id, zoom)
        .into_iter()
        .find(|(_, p)| (*p - point).hypot() <= radius)
        .map(|(side, _)| side)
}

/// A copy of object `id` beside it on `side`, empty of text, at `zoom`.
pub fn copy_beside(doc: &Document, id: ObjectId, side: Side, zoom: f64) -> Option<ObjectData> {
    let object = doc.get(id).filter(|o| can_copy(&o.data.kind))?;
    let mut data = object.data.clone();
    data.kind = match &data.kind {
        ObjectKind::Shape(shape) => ObjectKind::Shape(Shape::from(shape.kind)),
        _ => ObjectKind::Sticky(Sticky {
            spans: Default::default(),
            text: "".into(),
        }),
    };
    let size = data.bounds.size();
    let step = match side {
        Side::North | Side::South => size.height,
        Side::East | Side::West => size.width,
    } + GAP / zoom;
    // Along the object's own axes, so rotated objects line up too.
    let frame = data.frame();
    let center = frame.center();
    let local = data.bounds.center() + side.direction() * step;
    let moved = frame.to_world(local) - center;
    data.bounds = data.bounds + moved;
    Some(data)
}

/// An arrow from `from`'s `side` to the facing side of `to`.
pub fn link(
    doc: &Document,
    from: ObjectId,
    to: ObjectId,
    side: Side,
    route: Route,
) -> Option<ObjectData> {
    let a = doc.get(from)?;
    let b = doc.get(to)?;
    let end = |o: &pb_core::Object, side: Side| End::Attached {
        object: o.id,
        anchor: Anchor::Fixed(side.anchor()),
        at: o.data.frame().to_world(edge_middle(o.data.bounds, side)),
    };
    let (start, finish) = (end(a, side), end(b, side.opposite()));
    Some(ObjectData {
        kind: ObjectKind::Connector(Connector {
            start,
            end: finish,
            route,
            start_cap: Cap::None,
            end_cap: Cap::Arrow,
            label: Label::default(),
        }),
        bounds: Rect::from_points(start.point(), finish.point()),
        rotation: 0.0,
        style: pb_core::Style::default(),
    })
}

#[cfg(test)]
mod tests {
    use pb_core::{Command, ShapeKind};

    use super::*;

    fn doc_with_square() -> (Document, ObjectId) {
        let mut doc = Document::new();
        let data = ObjectData {
            kind: ObjectKind::Shape(Shape {
                spans: Default::default(),
                kind: ShapeKind::Diamond,
                text: "Start".into(),
            }),
            bounds: Rect::new(0.0, 0.0, 100.0, 60.0),
            rotation: 0.0,
            style: pb_core::Style::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        (doc, id)
    }

    #[test]
    fn dots_sit_around_and_copies_go_beside() {
        let (mut doc, id) = doc_with_square();
        let dots = dots(&doc, id, 2.0);
        assert_eq!(dots.len(), 4);
        // East: right of the edge middle by the offset at 2x.
        assert_eq!(dots[1], (Side::East, Point::new(122.0, 30.0)));
        assert_eq!(
            dot_at(&doc, id, Point::new(123.0, 31.0), 2.0),
            Some(Side::East)
        );
        assert_eq!(dot_at(&doc, id, Point::new(50.0, 30.0), 2.0), None);

        let copy = copy_beside(&doc, id, Side::South, 1.0).unwrap();
        assert_eq!(copy.bounds, Rect::new(0.0, 140.0, 100.0, 200.0));
        assert_eq!(copy.kind.text(), Some(""));
        assert!(matches!(&copy.kind, ObjectKind::Shape(s) if s.kind == ShapeKind::Diamond));
        let new = doc
            .apply(Command::Create(vec![copy.into()]))
            .unwrap()
            .created[0];
        let line = link(&doc, id, new, Side::South, Route::Straight).unwrap();
        doc.apply(Command::Create(vec![line.into()])).unwrap();
        let connector = doc.objects().last().unwrap().id;
        let (s, e) = doc.connector_ends(connector).unwrap();
        assert!(
            (s.point - Point::new(50.0, 60.0)).hypot() < 1e-6,
            "{:?}",
            s.point
        );
        assert!(
            (e.point - Point::new(50.0, 140.0)).hypot() < 1e-6,
            "{:?}",
            e.point
        );
    }

    #[test]
    fn only_shapes_and_notes_get_dots() {
        let mut doc = Document::new();
        let data = ObjectData {
            kind: ObjectKind::Frame(pb_core::FrameData { title: "F".into() }),
            bounds: Rect::new(0.0, 0.0, 100.0, 60.0),
            rotation: 0.0,
            style: pb_core::Style::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        assert!(dots(&doc, id, 1.0).is_empty());
        assert!(copy_beside(&doc, id, Side::East, 1.0).is_none());
    }
}
