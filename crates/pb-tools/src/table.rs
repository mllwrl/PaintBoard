//! Resizing a table's rows and columns by dragging the lines between them.

use std::sync::Arc;

use pb_core::{Line, ObjectData, Table};
use pb_geom::kurbo::Point;

/// Rows and columns never get smaller than this, in world units.
const MIN_SIZE: f64 = 12.0;

/// A line between two rows (`rows`) or two columns of a table: the one after line
/// `index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divider {
    pub rows: bool,
    pub index: usize,
}

/// The inner grid line of the table `data` within `radius` of `world`, if any.
pub fn divider_at(data: &ObjectData, table: &Table, world: Point, radius: f64) -> Option<Divider> {
    let local = data.frame().to_local(world);
    let b = data.bounds;
    if !b.inflate(radius, radius).contains(local) {
        return None;
    }
    let (ys, xs) = table.grid(b);
    let near = |edges: &[f64], v: f64| {
        edges[1..edges.len() - 1]
            .iter()
            .position(|&e| (e - v).abs() <= radius)
    };
    near(&xs, local.x)
        .map(|index| Divider { rows: false, index })
        .or_else(|| near(&ys, local.y).map(|index| Divider { rows: true, index }))
}

/// The table's rows or columns after moving `divider` by `delta` world units along the
/// table (down or right); the two lines beside it share the change, the rest stay.
pub fn resized(data: &ObjectData, table: &Table, divider: Divider, delta: f64) -> Arc<[Line]> {
    let (lines, size) = if divider.rows {
        (&table.rows, data.bounds.height())
    } else {
        (&table.cols, data.bounds.width())
    };
    let i = divider.index;
    let total: f64 = lines.iter().map(|l| l.weight).sum();
    if size <= 0.0 || total <= 0.0 || i + 1 >= lines.len() {
        return lines.clone();
    }
    // World size of each line, then back to weights.
    let unit = size / total;
    let (a, b) = (lines[i].weight * unit, lines[i + 1].weight * unit);
    let min = MIN_SIZE.min((a + b) / 2.0);
    let d = delta.clamp(min - a, b - min);
    let mut out = lines.to_vec();
    out[i].weight = (a + d) / unit;
    out[i + 1].weight = (b - d) / unit;
    out.into()
}

#[cfg(test)]
mod tests {
    use pb_core::{ObjectKind, Style};
    use pb_geom::kurbo::Rect;

    use super::*;

    fn table() -> (ObjectData, Table) {
        let mut n = 0;
        let table = Table::new(2, 3, || {
            n += 1;
            n.to_string()
        });
        let data = ObjectData {
            kind: ObjectKind::Table(table.clone()),
            bounds: Rect::new(0.0, 0.0, 300.0, 100.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        (data, table)
    }

    #[test]
    fn dividers_are_found_and_moved_between_neighbors() {
        let (data, table) = table();
        let at = |x, y| divider_at(&data, &table, Point::new(x, y), 3.0);
        assert_eq!(
            at(101.0, 20.0),
            Some(Divider {
                rows: false,
                index: 0
            })
        );
        assert_eq!(
            at(150.0, 49.0),
            Some(Divider {
                rows: true,
                index: 0
            })
        );
        // The outer edges resize the whole table, not lines.
        assert_eq!(at(0.0, 20.0), None);
        assert_eq!(at(150.0, 20.0), None);

        let cols = resized(
            &data,
            &table,
            Divider {
                rows: false,
                index: 0,
            },
            50.0,
        );
        let t = Table {
            cols,
            ..table.clone()
        };
        assert_eq!(t.cell_rect(data.bounds, 0, 0).unwrap().width(), 150.0);
        assert_eq!(t.cell_rect(data.bounds, 0, 1).unwrap().width(), 50.0);
        assert_eq!(t.cell_rect(data.bounds, 0, 2).unwrap().width(), 100.0);
        // It stops before a line gets too small.
        let cols = resized(
            &data,
            &table,
            Divider {
                rows: false,
                index: 0,
            },
            500.0,
        );
        let t = Table { cols, ..table };
        let width = t.cell_rect(data.bounds, 0, 1).unwrap().width();
        assert!((width - MIN_SIZE).abs() < 1e-9, "{width}");
    }
}
