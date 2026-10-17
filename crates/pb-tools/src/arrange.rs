//! Align and distribute the selection.

use pb_core::{Command, Document, ObjectId};
use pb_geom::kurbo::{Rect, Vec2};

use crate::{Selection, unit_frame};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    /// Centers on a vertical line.
    CenterX,
    Right,
    Top,
    /// Centers on a horizontal line.
    CenterY,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Lines the selected objects up with the edge or center of the selection bounds.
/// Needs two or more objects that are not locked.
pub fn align(doc: &Document, selection: &Selection, to: Align) -> Option<Command> {
    let units = movable_bounds(doc, selection);
    if units.len() < 2 {
        return None;
    }
    let all = units.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b))?;
    moves(units.iter().map(|&(id, r)| {
        let delta = match to {
            Align::Left => Vec2::new(all.x0 - r.x0, 0.0),
            Align::CenterX => Vec2::new(all.center().x - r.center().x, 0.0),
            Align::Right => Vec2::new(all.x1 - r.x1, 0.0),
            Align::Top => Vec2::new(0.0, all.y0 - r.y0),
            Align::CenterY => Vec2::new(0.0, all.center().y - r.center().y),
            Align::Bottom => Vec2::new(0.0, all.y1 - r.y1),
        };
        (id, delta)
    }))
}

/// Spaces the selected objects evenly along `axis`, keeping the outermost two in place.
/// Needs three or more objects that are not locked.
pub fn distribute(doc: &Document, selection: &Selection, axis: Axis) -> Option<Command> {
    // Work on x; the vertical case swaps coordinates in and out.
    let flip = |r: Rect| match axis {
        Axis::Horizontal => r,
        Axis::Vertical => Rect::new(r.y0, r.x0, r.y1, r.x1),
    };
    let mut units: Vec<_> = movable_bounds(doc, selection)
        .into_iter()
        .map(|(id, r)| (id, flip(r)))
        .collect();
    if units.len() < 3 {
        return None;
    }
    units.sort_by(|(_, a), (_, b)| a.center().x.total_cmp(&b.center().x));
    let (first, last) = (units[0].1, units[units.len() - 1].1);
    let inner = &units[1..units.len() - 1];
    let widths: f64 = inner.iter().map(|(_, r)| r.width()).sum();
    let gap = (last.x0 - first.x1 - widths) / (units.len() - 1) as f64;
    let mut x = first.x1 + gap;
    moves(inner.iter().map(|&(id, r)| {
        let dx = x - r.x0;
        x += r.width() + gap;
        let delta = match axis {
            Axis::Horizontal => Vec2::new(dx, 0.0),
            Axis::Vertical => Vec2::new(0.0, dx),
        };
        (id, delta)
    }))
}

/// Axis-aligned bounds of each selected object or group that is not locked.
fn movable_bounds(doc: &Document, selection: &Selection) -> Vec<(ObjectId, Rect)> {
    selection
        .ids()
        .iter()
        .filter(|&&id| !doc.is_locked(id))
        .filter_map(|&id| Some((id, unit_frame(doc, id, |o| o.data.frame())?.aabb())))
        .collect()
}

/// A move of the objects that actually change place, if any do.
fn moves(deltas: impl Iterator<Item = (ObjectId, Vec2)>) -> Option<Command> {
    const EPSILON: f64 = 1e-9;
    let moves: Vec<_> = deltas.filter(|(_, d)| d.hypot() > EPSILON).collect();
    (!moves.is_empty()).then_some(Command::MoveEach(moves))
}

#[cfg(test)]
mod tests {
    use pb_core::{ObjectData, ShapeKind, Style};

    use super::*;

    fn setup(rects: &[Rect]) -> (Document, Vec<ObjectId>, Selection) {
        let mut doc = Document::new();
        let data = rects
            .iter()
            .map(|&bounds| {
                ObjectData {
                    kind: ShapeKind::Rectangle.into(),
                    bounds,
                    rotation: 0.0,
                    style: Style::default(),
                    tags: Default::default(),
                    link: None,
                }
                .into()
            })
            .collect();
        let ids = doc.apply(Command::Create(data)).unwrap().created;
        let mut selection = Selection::default();
        selection.set(ids.iter().copied());
        (doc, ids, selection)
    }

    fn deltas(cmd: Option<Command>) -> Vec<(ObjectId, Vec2)> {
        match cmd {
            Some(Command::MoveEach(moves)) => moves,
            other => panic!("expected MoveEach, got {other:?}"),
        }
    }

    #[test]
    fn aligns_to_selection_bounds() {
        let (doc, ids, sel) = setup(&[
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(50.0, 40.0, 100.0, 60.0),
        ]);
        assert_eq!(
            deltas(align(&doc, &sel, Align::Right)),
            [(ids[0], Vec2::new(90.0, 0.0))]
        );
        assert_eq!(
            deltas(align(&doc, &sel, Align::Top)),
            [(ids[1], Vec2::new(0.0, -40.0))]
        );
        // Bounds center (50, 30).
        assert_eq!(
            deltas(align(&doc, &sel, Align::CenterY)),
            [
                (ids[0], Vec2::new(0.0, 25.0)),
                (ids[1], Vec2::new(0.0, -20.0))
            ]
        );
    }

    #[test]
    fn nothing_to_align() {
        let (mut doc, ids, sel) = setup(&[
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(0.0, 40.0, 10.0, 60.0),
        ]);
        assert!(align(&doc, &sel, Align::Left).is_none());
        doc.apply(Command::SetLocked {
            ids: vec![ids[1]],
            locked: true,
        })
        .unwrap();
        assert!(align(&doc, &sel, Align::Right).is_none());
    }

    #[test]
    fn distributes_with_equal_gaps() {
        // Gaps 10 and 50 become 30 and 30; the outer two stay.
        let (doc, ids, sel) = setup(&[
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(100.0, 0.0, 110.0, 10.0),
            Rect::new(20.0, 50.0, 50.0, 60.0),
        ]);
        assert_eq!(
            deltas(distribute(&doc, &sel, Axis::Horizontal)),
            [(ids[2], Vec2::new(20.0, 0.0))]
        );
        assert!(distribute(&doc, &sel, Axis::Vertical).is_some());
    }

    #[test]
    fn distribute_vertically_moves_along_y() {
        let (doc, ids, sel) = setup(&[
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(0.0, 15.0, 10.0, 25.0),
            Rect::new(0.0, 90.0, 10.0, 100.0),
        ]);
        assert_eq!(
            deltas(distribute(&doc, &sel, Axis::Vertical)),
            [(ids[1], Vec2::new(0.0, 30.0))]
        );
    }

    #[test]
    fn groups_move_as_one() {
        let (mut doc, ids, _) = setup(&[
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(20.0, 0.0, 30.0, 10.0),
            Rect::new(0.0, 50.0, 100.0, 60.0),
        ]);
        let g = doc
            .apply(Command::Group(vec![ids[0], ids[1]]))
            .unwrap()
            .created[0];
        let mut sel = Selection::default();
        sel.set([g, ids[2]]);
        // The group (0..30) centers on the bounds (0..100) as a whole.
        assert_eq!(
            deltas(align(&doc, &sel, Align::CenterX)),
            [(g, Vec2::new(35.0, 0.0))]
        );
    }
}
