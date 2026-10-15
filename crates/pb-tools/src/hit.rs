use pb_core::{Object, ObjectId, ObjectKind};
use pb_geom::kurbo::{Point, Rect};

/// Topmost object under `point`. `tolerance` (world units) makes thin shapes easier to hit.
///
/// TODO: query an R-tree instead of scanning every object.
pub fn object_at(objects: &[Object], point: Point, tolerance: f64) -> Option<ObjectId> {
    objects
        .iter()
        .rev()
        .find(|o| match o.data.kind {
            ObjectKind::Rectangle => {
                let pad = tolerance + o.data.style.stroke_width / 2.0;
                o.data.frame().contains(point, pad)
            }
        })
        .map(|o| o.id)
}

/// Objects touched by `area`, in z-order.
pub fn objects_in(objects: &[Object], area: Rect) -> impl Iterator<Item = ObjectId> + '_ {
    objects
        .iter()
        .filter(move |o| o.data.frame().overlaps(area))
        .map(|o| o.id)
}

#[cfg(test)]
mod tests {
    use pb_core::{Command, Document, ObjectData, Style};

    use super::*;

    fn doc_with(rects: &[Rect]) -> (Document, Vec<ObjectId>) {
        let mut doc = Document::new();
        let ids = rects
            .iter()
            .map(|&bounds| {
                doc.apply(Command::Create(vec![ObjectData {
                    kind: ObjectKind::Rectangle,
                    bounds,
                    rotation: 0.0,
                    style: Style::default(),
                }]))
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
        assert_eq!(
            object_at(doc.objects(), Point::new(75.0, 75.0), 0.0),
            Some(ids[1])
        );
        assert_eq!(
            object_at(doc.objects(), Point::new(10.0, 10.0), 0.0),
            Some(ids[0])
        );
        assert_eq!(object_at(doc.objects(), Point::new(500.0, 0.0), 0.0), None);
    }

    #[test]
    fn tolerance_extends_hit_area() {
        let (doc, ids) = doc_with(&[Rect::new(0.0, 0.0, 10.0, 10.0)]);
        // Default stroke is 2 wide, so the painted edge already reaches 11.
        assert_eq!(object_at(doc.objects(), Point::new(13.0, 5.0), 0.0), None);
        assert_eq!(
            object_at(doc.objects(), Point::new(13.0, 5.0), 3.0),
            Some(ids[0])
        );
    }

    #[test]
    fn marquee_selects_touched_objects() {
        let (doc, ids) = doc_with(&[
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(100.0, 100.0, 110.0, 110.0),
        ]);
        let hit: Vec<_> = objects_in(doc.objects(), Rect::new(5.0, 5.0, 50.0, 50.0)).collect();
        assert_eq!(hit, [ids[0]]);
    }
}
