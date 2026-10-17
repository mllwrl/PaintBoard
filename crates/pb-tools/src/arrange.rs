//! Align, distribute, lay out in a grid and cluster the selection.

use pb_core::{Command, Document, ObjectId, Rgba};
use pb_geom::kurbo::{Point, Rect, Vec2};

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

/// What clustered objects are grouped by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterBy {
    /// Fill color; objects without a fill go together.
    Color,
    /// The first of the board's tags each one has.
    Tag,
}

/// A heading to put above a cluster: its text, where its top left corner goes, and its
/// font size.
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterLabel {
    pub text: String,
    pub at: Point,
    pub size: f64,
}

/// Lays the selected objects out in a grid in reading order, from the selection's top left
/// corner. Needs two or more objects that are not locked.
pub fn arrange_grid(doc: &Document, selection: &Selection) -> Option<Command> {
    let units = reading_order(movable_bounds(doc, selection));
    if units.len() < 2 {
        return None;
    }
    let origin = top_left(&units);
    let gap = gap(&units);
    let (deltas, _) = grid(&units, origin, gap);
    moves(deltas.into_iter())
}

/// Groups the selected objects by `by` and lays each group out in its own grid, groups side
/// by side from the selection's top left corner, with room above each for a heading. The
/// headings are returned for the caller to add. Needs two or more objects that are not
/// locked.
pub fn cluster(
    doc: &Document,
    selection: &Selection,
    by: ClusterBy,
) -> Option<(Option<Command>, Vec<ClusterLabel>)> {
    let units = reading_order(movable_bounds(doc, selection));
    if units.len() < 2 {
        return None;
    }
    let mut groups: Vec<(String, Vec<(ObjectId, Rect)>)> = Vec::new();
    for unit in &units {
        let label = cluster_label(doc, unit.0, by);
        match groups.iter_mut().find(|(l, _)| *l == label) {
            Some((_, members)) => members.push(*unit),
            None => groups.push((label, vec![*unit])),
        }
    }
    let origin = top_left(&units);
    let gap = gap(&units);
    let avg_h = units.iter().map(|(_, r)| r.height()).sum::<f64>() / units.len() as f64;
    let band = avg_h * 0.4;
    let size = avg_h * 0.2;
    let mut x = origin.x;
    let mut deltas = Vec::new();
    let mut labels = Vec::new();
    for (label, members) in groups {
        labels.push(ClusterLabel {
            text: label,
            at: Point::new(x, origin.y),
            size,
        });
        let (moved, block) = grid(&members, Point::new(x, origin.y + band), gap);
        deltas.extend(moved);
        x += block.width() + 3.0 * gap;
    }
    Some((moves(deltas.into_iter()), labels))
}

/// What object or group `id` is clustered under.
fn cluster_label(doc: &Document, id: ObjectId, by: ClusterBy) -> String {
    let leaves = doc.leaves(id);
    match by {
        ClusterBy::Color => leaves
            .iter()
            .find(|o| o.data.kind.has_fill() && o.data.style.fill.to_bytes()[3] > 0)
            .map_or_else(|| "No color".to_owned(), |o| color_name(o.data.style.fill)),
        ClusterBy::Tag => doc
            .tags()
            .iter()
            .find(|t| leaves.iter().any(|o| o.data.tags.contains(&t.id)))
            .map_or_else(|| "No tag".to_owned(), |t| t.name.to_string()),
    }
}

/// A name for `color` by its hue: "Yellow", "Blue" and so on, or "White", "Gray" and
/// "Black" for colors with little hue.
pub fn color_name(color: Rgba) -> String {
    let [r, g, b, _] = color.to_bytes();
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let light = (max + min) / 2.0;
    let chroma = max - min;
    let saturation = if chroma == 0.0 {
        0.0
    } else {
        chroma / (1.0 - (2.0 * light - 1.0).abs())
    };
    let name = if light < 0.2 {
        "Black"
    } else if saturation < 0.15 || chroma < 0.06 {
        if light > 0.92 { "White" } else { "Gray" }
    } else {
        let hue = if max == r {
            60.0 * ((g - b) / chroma).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / chroma + 2.0)
        } else {
            60.0 * ((r - g) / chroma + 4.0)
        };
        match hue {
            h if !(15.0..345.0).contains(&h) => "Red",
            h if h < 45.0 => "Orange",
            h if h < 70.0 => "Yellow",
            h if h < 170.0 => "Green",
            h if h < 255.0 => "Blue",
            h if h < 290.0 => "Purple",
            _ => "Pink",
        }
    };
    name.to_owned()
}

/// `units` in reading order: rows top to bottom (by their middles, a row being about an
/// object high), left to right within a row.
fn reading_order(mut units: Vec<(ObjectId, Rect)>) -> Vec<(ObjectId, Rect)> {
    if units.is_empty() {
        return units;
    }
    let avg_h = units.iter().map(|(_, r)| r.height()).sum::<f64>() / units.len() as f64;
    let top = units
        .iter()
        .map(|(_, r)| r.y0)
        .fold(f64::INFINITY, f64::min);
    let row = |r: &Rect| ((r.center().y - top) / avg_h.max(1e-9)).floor() as i64;
    units.sort_by(|a, b| row(&a.1).cmp(&row(&b.1)).then(a.1.x0.total_cmp(&b.1.x0)));
    units
}

fn top_left(units: &[(ObjectId, Rect)]) -> Point {
    let x = units
        .iter()
        .map(|(_, r)| r.x0)
        .fold(f64::INFINITY, f64::min);
    let y = units
        .iter()
        .map(|(_, r)| r.y0)
        .fold(f64::INFINITY, f64::min);
    Point::new(x, y)
}

/// Space between laid-out objects: a fifth of their average size.
fn gap(units: &[(ObjectId, Rect)]) -> f64 {
    let n = units.len().max(1) as f64;
    let avg = units
        .iter()
        .map(|(_, r)| r.width().min(r.height()))
        .sum::<f64>()
        / n;
    avg * 0.2
}

/// Moves putting `units` in a grid of about as many columns as rows from `origin`, each in
/// a cell as big as the largest; and the area of the grid.
fn grid(units: &[(ObjectId, Rect)], origin: Point, gap: f64) -> (Vec<(ObjectId, Vec2)>, Rect) {
    let n = units.len();
    let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
    let rows = n.div_ceil(cols);
    let w = units.iter().map(|(_, r)| r.width()).fold(0.0, f64::max);
    let h = units.iter().map(|(_, r)| r.height()).fold(0.0, f64::max);
    let deltas = units
        .iter()
        .enumerate()
        .map(|(i, &(id, r))| {
            let slot = Point::new(
                origin.x + (i % cols) as f64 * (w + gap),
                origin.y + (i / cols) as f64 * (h + gap),
            );
            (id, slot - r.origin())
        })
        .collect();
    let area = Rect::from_origin_size(
        origin,
        (cols as f64 * (w + gap) - gap, rows as f64 * (h + gap) - gap),
    );
    (deltas, area)
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
    fn grids_keep_reading_order() {
        // Five squares scattered: the grid is 3 wide, in reading order from the top left.
        let (mut doc, ids, sel) = setup(&[
            Rect::new(300.0, 0.0, 310.0, 10.0),
            Rect::new(0.0, 2.0, 10.0, 12.0),
            Rect::new(100.0, 100.0, 110.0, 110.0),
            Rect::new(50.0, 1.0, 60.0, 11.0),
            Rect::new(0.0, 200.0, 10.0, 210.0),
        ]);
        doc.apply(arrange_grid(&doc, &sel).unwrap()).unwrap();
        let at = |id| doc.get(id).unwrap().data.bounds.origin();
        // The gap is a fifth of 10.
        assert_eq!(at(ids[1]), Point::new(0.0, 0.0));
        assert_eq!(at(ids[3]), Point::new(12.0, 0.0));
        assert_eq!(at(ids[0]), Point::new(24.0, 0.0));
        assert_eq!(at(ids[2]), Point::new(0.0, 12.0));
        assert_eq!(at(ids[4]), Point::new(12.0, 12.0));
    }

    #[test]
    fn clusters_by_color_side_by_side() {
        let (mut doc, ids, sel) = setup(&[
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(20.0, 0.0, 30.0, 10.0),
            Rect::new(40.0, 0.0, 50.0, 10.0),
        ]);
        let yellow = pb_core::StyleChange {
            fill: Some(Rgba::rgb(0xFF, 0xF1, 0x76)),
            ..Default::default()
        };
        doc.apply(Command::SetStyle {
            ids: vec![ids[0], ids[2]],
            change: yellow,
        })
        .unwrap();
        let (moves, labels) = cluster(&doc, &sel, ClusterBy::Color).unwrap();
        let names: Vec<_> = labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(names, ["Yellow", "White"]);
        doc.apply(moves.unwrap()).unwrap();
        let at = |id| doc.get(id).unwrap().data.bounds.origin();
        // The yellow ones in a grid two wide under their heading, the white one beside.
        assert_eq!(at(ids[0]), Point::new(0.0, 4.0));
        assert_eq!(at(ids[2]), Point::new(12.0, 4.0));
        assert_eq!(at(ids[1]), Point::new(28.0, 4.0));
        assert_eq!(labels[1].at, Point::new(28.0, 0.0));
        // The sticky note colors get the names people would give them.
        let named = |r, g, b| color_name(Rgba::rgb(r, g, b));
        assert_eq!(named(0xB3, 0xE0, 0xFF), "Blue");
        assert_eq!(named(0xD1, 0xC4, 0xE9), "Purple");
        assert_eq!(named(0xF8, 0xBB, 0xD9), "Pink");
        assert_eq!(named(0xFF, 0xAB, 0xAB), "Red");
        assert_eq!(named(0xFF, 0xD0, 0x80), "Orange");
        assert_eq!(named(0xB9, 0xF0, 0xC8), "Green");
        assert_eq!(named(0xF0, 0xF0, 0xF0), "White");
        assert_eq!(named(0x80, 0x80, 0x80), "Gray");
        assert_eq!(named(0x1F, 0x29, 0x37), "Black");
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
