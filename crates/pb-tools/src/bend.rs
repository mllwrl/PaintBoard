//! Bending connectors by hand: handles on a selected connector's line, and the bends that
//! dragging them makes.
//!
//! An elbow line has a handle in the middle of each segment; dragging it slides the
//! segment across, and the line's corners become its bends. The first and last segments
//! keep a short stub at their end, so the line still leaves its object straight. A straight or
//! curved line has a handle on each bend, to move it, and one in the middle of each piece,
//! to add a bend there.

use pb_core::{Document, ObjectId, ObjectKind, Route};
use pb_geom::kurbo::{PathEl, Point, Vec2};

/// Length of the stub left at an end when its segment is slid away, in world units.
const STUB: f64 = 24.0;

/// What a bend handle does when dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BendGrip {
    /// Slides the elbow segment from corner `i` to `i + 1` across, or adds a bend between
    /// points `i` and `i + 1` of a straight or curved line.
    Segment(usize),
    /// Moves bend `i`.
    Point(usize),
}

/// The corners of a connector's drawn line.
fn corners(doc: &Document, id: ObjectId) -> Vec<Point> {
    let Some(path) = doc.connector_path(id) else {
        return Vec::new();
    };
    path.line
        .elements()
        .iter()
        .filter_map(|el| match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
            _ => None,
        })
        .collect()
}

/// The points a straight or curved line runs through: its ends and its bends.
fn through(doc: &Document, id: ObjectId) -> Option<Vec<Point>> {
    let ObjectKind::Connector(c) = &doc.get(id)?.data.kind else {
        return None;
    };
    let (s, e) = doc.connector_ends(id)?;
    let mut points = vec![s.point];
    points.extend_from_slice(&c.bends);
    points.push(e.point);
    Some(points)
}

fn route(doc: &Document, id: ObjectId) -> Option<Route> {
    match &doc.get(id)?.data.kind {
        ObjectKind::Connector(c) => Some(c.route),
        _ => None,
    }
}

/// The bend handles of connector `id`, with where they are (world).
pub fn handles(doc: &Document, id: ObjectId) -> Vec<(Point, BendGrip)> {
    match route(doc, id) {
        Some(Route::Elbow) => {
            let corners = corners(doc, id);
            let last = corners.len().saturating_sub(2);
            (0..corners.len().saturating_sub(1))
                // An end segment needs room for its stub.
                .filter(|&i| {
                    let short = (corners[i + 1] - corners[i]).hypot() < 2.0 * STUB;
                    !(short && (i == 0 || i == last))
                })
                .map(|i| {
                    let mid = corners[i].midpoint(corners[i + 1]);
                    (mid, BendGrip::Segment(i))
                })
                .collect()
        }
        Some(Route::Straight | Route::Curved) => {
            let Some(points) = through(doc, id) else {
                return Vec::new();
            };
            let bends = (1..points.len() - 1).map(|i| (points[i], BendGrip::Point(i - 1)));
            let middles = (0..points.len() - 1)
                .map(|i| (points[i].midpoint(points[i + 1]), BendGrip::Segment(i)));
            bends.chain(middles).collect()
        }
        None => Vec::new(),
    }
}

/// The bends of connector `id` after dragging `grip` by `delta`.
pub fn dragged(doc: &Document, id: ObjectId, grip: BendGrip, delta: Vec2) -> Option<Vec<Point>> {
    match (route(doc, id)?, grip) {
        (Route::Elbow, BendGrip::Segment(i)) => {
            let mut corners = corners(doc, id);
            if i + 1 >= corners.len() {
                return None;
            }
            let mut i = i;
            // The ends stay: an end segment leaves a stub there, and the slid part starts
            // after it.
            if i + 2 == corners.len() {
                let n = corners.len();
                let dir = (corners[n - 1] - corners[n - 2]).normalize();
                corners.insert(n - 1, corners[n - 1] - dir * STUB);
            }
            if i == 0 {
                let dir = (corners[1] - corners[0]).normalize();
                corners.insert(1, corners[0] + dir * STUB);
                i = 1;
            }
            let (a, b) = (corners[i], corners[i + 1]);
            // Across the segment only.
            let shift = if (a.y - b.y).abs() < 1e-9 {
                Vec2::new(0.0, delta.y)
            } else {
                Vec2::new(delta.x, 0.0)
            };
            corners[i] += shift;
            corners[i + 1] += shift;
            Some(corners[1..corners.len() - 1].to_vec())
        }
        (Route::Elbow, BendGrip::Point(_)) => None,
        (_, BendGrip::Point(i)) => {
            let points = through(doc, id)?;
            let mut bends = points[1..points.len() - 1].to_vec();
            *bends.get_mut(i)? += delta;
            Some(bends)
        }
        (_, BendGrip::Segment(i)) => {
            let points = through(doc, id)?;
            let mid = points.get(i)?.midpoint(*points.get(i + 1)?);
            let mut bends = points[1..points.len() - 1].to_vec();
            bends.insert(i, mid + delta);
            Some(bends)
        }
    }
}

#[cfg(test)]
mod tests {
    use pb_core::{Cap, Command, Connector, End, Label, ObjectData, Style};
    use pb_geom::kurbo::Rect;

    use super::*;

    fn line(doc: &mut Document, route: Route) -> ObjectId {
        let data = ObjectData {
            kind: ObjectKind::Connector(Connector {
                start: End::Free(Point::new(0.0, 0.0)),
                end: End::Free(Point::new(200.0, 100.0)),
                route,
                start_cap: Cap::None,
                end_cap: Cap::None,
                label: Label::default(),
                bends: Default::default(),
            }),
            bounds: Rect::ZERO,
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        doc.apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0]
    }

    #[test]
    fn elbow_segments_slide_across() {
        let mut doc = Document::new();
        let id = line(&mut doc, Route::Elbow);
        // Free ends: across, down, across; a handle on each segment.
        let handles = handles(&doc, id);
        assert_eq!(handles.len(), 3, "{handles:?}");
        let (at, grip) = handles[1];
        assert_eq!(at, Point::new(100.0, 50.0));
        // Dragging a vertical segment moves it sideways only.
        let bends = dragged(&doc, id, grip, Vec2::new(40.0, 30.0)).unwrap();
        assert_eq!(
            bends,
            vec![Point::new(140.0, 0.0), Point::new(140.0, 100.0)]
        );
        doc.apply(Command::SetBends {
            id,
            bends: bends.into(),
        })
        .unwrap();
        // The line now runs through them, still axis-aligned.
        let drawn = corners(&doc, id);
        assert_eq!(drawn[1], Point::new(140.0, 0.0));
        for w in drawn.windows(2) {
            assert!(w[0].x == w[1].x || w[0].y == w[1].y, "{drawn:?}");
        }
        // Moving the connector moves its bends; another route drops them.
        doc.apply(Command::Move {
            ids: vec![id],
            delta: Vec2::new(10.0, 0.0),
        })
        .unwrap();
        assert_eq!(corners(&doc, id)[1], Point::new(150.0, 0.0));
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(corners(&loaded, id)[1], Point::new(150.0, 0.0));
        doc.apply(Command::SetConnector {
            ids: vec![id],
            change: pb_core::ConnectorChange {
                route: Some(Route::Straight),
                ..Default::default()
            },
        })
        .unwrap();
        assert_eq!(corners(&doc, id).len(), 2);
    }

    #[test]
    fn end_segments_slide_and_keep_a_stub() {
        let mut doc = Document::new();
        let id = line(&mut doc, Route::Elbow);
        // The first segment, (0, 0) to (100, 0), slides down 30: a stub stays at the start.
        let bends = dragged(&doc, id, BendGrip::Segment(0), Vec2::new(0.0, 30.0)).unwrap();
        doc.apply(Command::SetBends {
            id,
            bends: bends.into(),
        })
        .unwrap();
        let drawn = corners(&doc, id);
        assert_eq!(drawn[0], Point::new(0.0, 0.0));
        assert_eq!(drawn[1], Point::new(STUB, 0.0));
        assert_eq!(drawn[2], Point::new(STUB, 30.0));
        assert_eq!(drawn.last(), Some(&Point::new(200.0, 100.0)));
        for w in drawn.windows(2) {
            assert!(w[0].x == w[1].x || w[0].y == w[1].y, "{drawn:?}");
        }
    }

    #[test]
    fn straight_lines_get_and_move_bends() {
        let mut doc = Document::new();
        let id = line(&mut doc, Route::Straight);
        let handles = handles(&doc, id);
        assert_eq!(
            handles,
            vec![(Point::new(100.0, 50.0), BendGrip::Segment(0))]
        );
        let bends = dragged(&doc, id, BendGrip::Segment(0), Vec2::new(0.0, -50.0)).unwrap();
        assert_eq!(bends, vec![Point::new(100.0, 0.0)]);
        doc.apply(Command::SetBends {
            id,
            bends: bends.into(),
        })
        .unwrap();
        assert_eq!(corners(&doc, id).len(), 3);
        // Now there is a bend handle and two middles.
        let handles = super::handles(&doc, id);
        assert_eq!(handles.len(), 3);
        let moved = dragged(&doc, id, BendGrip::Point(0), Vec2::new(5.0, 5.0)).unwrap();
        assert_eq!(moved, vec![Point::new(105.0, 5.0)]);
    }
}
