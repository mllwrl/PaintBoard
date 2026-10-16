use pb_core::{Document, ObjectId, ObjectKind};
use pb_geom::kurbo::{Point, Rect};

/// Topmost top-level object or group under `point`. `tolerance` (world units) makes thin
/// shapes easier to hit.
///
/// TODO: query an R-tree instead of scanning every object.
pub fn object_at(doc: &Document, point: Point, tolerance: f64) -> Option<ObjectId> {
    doc.objects()
        .iter()
        .rev()
        .find(|o| match o.data.kind {
            ObjectKind::Shape(shape) => {
                let pad = tolerance + o.data.style.stroke_width / 2.0;
                let local = o.data.frame().to_local(point);
                shape.hit(o.data.bounds, local, pad)
            }
        })
        .map(|o| doc.top_level(o.id))
}

/// Unlocked top-level objects and groups touched by `area`, in z-order.
pub fn objects_in(doc: &Document, area: Rect) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    for object in doc.objects() {
        let id = doc.top_level(object.id);
        if object.data.frame().overlaps(area) && !ids.contains(&id) && !doc.is_locked(id) {
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
