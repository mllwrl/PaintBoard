use pb_core::{Document, FrameData, Object, ObjectData, ObjectId, ObjectKind};
use pb_geom::ink::polyline_distance;
use pb_geom::kurbo::{Point, Rect};

/// `point` grown by `tolerance` on every side, to look objects up in.
fn around(point: Point, tolerance: f64) -> Rect {
    Rect::from_center_size(point, (2.0 * tolerance, 2.0 * tolerance))
}

/// Topmost top-level object or group under `point`. `tolerance` (world units) makes thin
/// shapes easier to hit.
pub fn object_at(doc: &Document, point: Point, tolerance: f64) -> Option<ObjectId> {
    doc.objects_in(around(point, tolerance))
        .into_iter()
        .rev()
        .find(|o| object_hits(doc, o, point, tolerance))
        .map(|o| doc.top_level(o.id))
}

/// The frame whose title is under `point` at `zoom`, topmost first. Frames are hit only
/// there, so pressing inside one selects what is in it or starts a marquee.
pub fn frame_title_at(doc: &Document, point: Point, zoom: f64) -> Option<ObjectId> {
    // Titles sit above their frames: look below the point by a title's height.
    let band = FrameData::title_band(Rect::ZERO, zoom).height();
    doc.objects_in(Rect::new(point.x, point.y, point.x, point.y + band))
        .into_iter()
        .rev()
        .filter(|o| o.data.kind.is_frame())
        .find(|o| FrameData::title_band(o.data.bounds, zoom).contains(point))
        .map(|o| doc.top_level(o.id))
}

/// Topmost object (not group) under `point` that connectors can attach to.
pub fn attachable_at(doc: &Document, point: Point, tolerance: f64) -> Option<&Object> {
    doc.objects_in(around(point, tolerance))
        .into_iter()
        .rev()
        .filter(|o| pb_core::connector::can_attach(&o.data.kind))
        .find(|o| hits(&o.data, point, tolerance))
}

/// [`hits`], with connectors hit along their path and on their label.
fn object_hits(doc: &Document, object: &Object, point: Point, tolerance: f64) -> bool {
    match &object.data.kind {
        ObjectKind::Connector(_) => {
            let on_line = doc.connector_path(object.id).is_some_and(|path| {
                path.distance(point) <= tolerance + object.data.style.stroke_width / 2.0
            });
            let on_label = doc
                .connector_label(object.id)
                .is_some_and(|r| r.inflate(tolerance, tolerance).contains(point));
            on_line || on_label
        }
        _ => hits(&object.data, point, tolerance),
    }
}

/// Whether the object reaches into `area` (world): its frame for shapes, its line for ink.
fn touches(data: &ObjectData, area: Rect) -> bool {
    if !data.frame().overlaps(area) {
        return false;
    }
    match &data.kind {
        ObjectKind::Shape(_)
        | ObjectKind::Text(_)
        | ObjectKind::Sticky(_)
        | ObjectKind::Image(_)
        | ObjectKind::Table(_) => true,
        // Only when the area takes all of it, so a marquee inside a frame selects its
        // contents.
        ObjectKind::Frame(_) => area.contains_rect(data.bounds),
        // Needs the document's path; see `connector_touches`.
        ObjectKind::Connector(_) => false,
        ObjectKind::Ink(ink) => {
            let frame = data.frame();
            let points: Vec<_> = ink
                .points_in(data.bounds)
                .into_iter()
                .map(|(p, _)| frame.to_world(p))
                .collect();
            match &points[..] {
                [only] => area.contains(*only),
                _ => points.windows(2).any(|w| segment_touches(w[0], w[1], area)),
            }
        }
    }
}

/// Whether the segment `a`-`b` has a point inside `rect` (Liang–Barsky clipping).
fn segment_touches(a: Point, b: Point, rect: Rect) -> bool {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-d.x, a.x - rect.x0),
        (d.x, rect.x1 - a.x),
        (-d.y, a.y - rect.y0),
        (d.y, rect.y1 - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    t0 <= t1
}

/// Whether `point` (world) is on the object or within `tolerance` of what it paints.
pub(crate) fn hits(data: &ObjectData, point: Point, tolerance: f64) -> bool {
    let pad = tolerance + data.style.stroke_width / 2.0;
    let local = data.frame().to_local(point);
    match &data.kind {
        ObjectKind::Shape(shape) => shape.kind.hit(data.bounds, local, pad),
        ObjectKind::Text(_) | ObjectKind::Sticky(_) | ObjectKind::Table(_) => {
            data.bounds.inflate(pad, pad).contains(local)
        }
        // By its title only, see `frame_title_at`.
        ObjectKind::Frame(_) => false,
        // Hit along the path, which needs the document; see `connector_hit`.
        ObjectKind::Connector(_) => false,
        // Images ignore their outline width.
        ObjectKind::Image(_) => data.bounds.inflate(tolerance, tolerance).contains(local),
        // Ink is hit along its line, not anywhere in its bounds.
        ObjectKind::Ink(ink) => {
            if !data.bounds.inflate(pad, pad).contains(local) {
                return false;
            }
            let points: Vec<_> = ink
                .points_in(data.bounds)
                .into_iter()
                .map(|(p, _)| p)
                .collect();
            polyline_distance(&points, local) <= pad
        }
    }
}

/// Unlocked top-level objects and groups touched by `area`, in z-order.
pub fn objects_in(doc: &Document, area: Rect) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    for object in doc.objects_in(area) {
        let id = doc.top_level(object.id);
        let touched = match &object.data.kind {
            ObjectKind::Connector(_) => {
                doc.connector_path(object.id)
                    .is_some_and(|path| pb_geom::connector::touches(path, area))
                    || doc
                        .connector_label(object.id)
                        .is_some_and(|r| r.overlaps(area))
            }
            _ => touches(&object.data, area),
        };
        if touched && !ids.contains(&id) && !doc.is_locked(id) {
            ids.push(id);
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use pb_core::{Command, ObjectData, ShapeKind, Style};

    use super::*;

    fn doc_with(rects: &[Rect]) -> (Document, Vec<ObjectId>) {
        let mut doc = Document::new();
        let ids = rects
            .iter()
            .map(|&bounds| {
                let data = ObjectData {
                    kind: ShapeKind::Rectangle.into(),
                    bounds,
                    rotation: 0.0,
                    style: Style::default(),
                };
                doc.apply(Command::Create(vec![data.into()]))
                    .unwrap()
                    .created[0]
            })
            .collect();
        (doc, ids)
    }

    #[test]
    fn picks_topmost() {
        let (doc, ids) = doc_with(&[
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Rect::new(50.0, 50.0, 150.0, 150.0),
        ]);
        assert_eq!(object_at(&doc, Point::new(75.0, 75.0), 0.0), Some(ids[1]));
        assert_eq!(object_at(&doc, Point::new(10.0, 10.0), 0.0), Some(ids[0]));
        assert_eq!(object_at(&doc, Point::new(500.0, 0.0), 0.0), None);
    }

    #[test]
    fn tolerance_extends_hit_area() {
        let (doc, ids) = doc_with(&[Rect::new(0.0, 0.0, 10.0, 10.0)]);
        // Default stroke is 2 wide, so the painted edge already reaches 11.
        assert_eq!(object_at(&doc, Point::new(13.0, 5.0), 0.0), None);
        assert_eq!(object_at(&doc, Point::new(13.0, 5.0), 3.0), Some(ids[0]));
    }

    #[test]
    fn hits_follow_the_shape_outline() {
        let mut doc = Document::new();
        let data = ObjectData {
            kind: ShapeKind::Ellipse.into(),
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            rotation: 0.0,
            style: Style::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        assert_eq!(object_at(&doc, Point::new(50.0, 50.0), 0.0), Some(id));
        // The corner of the bounds is outside the circle.
        assert_eq!(object_at(&doc, Point::new(5.0, 5.0), 0.0), None);
    }

    #[test]
    fn marquee_selects_touched_objects() {
        let (doc, ids) = doc_with(&[
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(100.0, 100.0, 110.0, 110.0),
        ]);
        let hit = objects_in(&doc, Rect::new(5.0, 5.0, 50.0, 50.0));
        assert_eq!(hit, [ids[0]]);
    }

    #[test]
    fn hits_resolve_to_groups_and_skip_locked() {
        let (mut doc, ids) = doc_with(&[
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(20.0, 0.0, 30.0, 10.0),
            Rect::new(40.0, 0.0, 50.0, 10.0),
        ]);
        let g = doc
            .apply(Command::Group(vec![ids[0], ids[1]]))
            .unwrap()
            .created[0];
        doc.apply(Command::SetLocked {
            ids: vec![ids[2]],
            locked: true,
        })
        .unwrap();
        assert_eq!(object_at(&doc, Point::new(25.0, 5.0), 0.0), Some(g));
        // Locked objects can still be clicked, but not swept up by a marquee.
        assert_eq!(object_at(&doc, Point::new(45.0, 5.0), 0.0), Some(ids[2]));
        let hit = objects_in(&doc, Rect::new(-5.0, -5.0, 100.0, 100.0));
        assert_eq!(hit, [g]);
    }
}
