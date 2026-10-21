//! Tables: a grid of cells with text, filling the object's bounds.
//!
//! Rows and columns have stable ids, and each cell's text is stored under the ids of its
//! row and column, so concurrent edits of different cells merge, and adding a row on one
//! replica does not move the cells another replica is typing in. Merged cells are named
//! by the ids of their corner lines too.

use std::collections::BTreeMap;
use std::sync::Arc;

use kurbo::{Point, Rect};

use crate::{HAlign, Spans, Style, TextBox, VAlign};

/// A row or a column.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Unique within its table (see [`crate::Document::new_line_id`]).
    pub id: Arc<str>,
    /// Its share of the table's height (rows) or width (columns), relative to the others.
    pub weight: f64,
}

impl Line {
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self {
            id: id.into(),
            weight: 1.0,
        }
    }
}

/// A cell's text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub text: Arc<str>,
    pub spans: Spans,
}

/// A block of cells, by index: rows `row0..=row1` and columns `col0..=col1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRange {
    pub row0: usize,
    pub col0: usize,
    pub row1: usize,
    pub col1: usize,
}

impl CellRange {
    /// The block spanned by two corner cells, in any order.
    pub fn new((r0, c0): (usize, usize), (r1, c1): (usize, usize)) -> Self {
        Self {
            row0: r0.min(r1),
            col0: c0.min(c1),
            row1: r0.max(r1),
            col1: c0.max(c1),
        }
    }

    pub fn cell((row, col): (usize, usize)) -> Self {
        Self::new((row, col), (row, col))
    }

    pub fn rows(&self) -> std::ops::RangeInclusive<usize> {
        self.row0..=self.row1
    }

    pub fn cols(&self) -> std::ops::RangeInclusive<usize> {
        self.col0..=self.col1
    }

    /// Its rows (`rows`) or columns.
    pub fn lines(&self, rows: bool) -> std::ops::RangeInclusive<usize> {
        if rows { self.rows() } else { self.cols() }
    }

    pub fn contains(&self, (row, col): (usize, usize)) -> bool {
        self.rows().contains(&row) && self.cols().contains(&col)
    }

    pub fn is_single(&self) -> bool {
        self.row0 == self.row1 && self.col0 == self.col1
    }

    pub fn intersects(&self, other: &CellRange) -> bool {
        self.row0 <= other.row1
            && other.row0 <= self.row1
            && self.col0 <= other.col1
            && other.col0 <= self.col1
    }

    /// The smallest block covering both.
    pub fn union(&self, other: &CellRange) -> CellRange {
        CellRange {
            row0: self.row0.min(other.row0),
            col0: self.col0.min(other.col0),
            row1: self.row1.max(other.row1),
            col1: self.col1.max(other.col1),
        }
    }
}

/// Cells joined into one, named by the ids of its first and last row and column, so lines
/// added inside it widen it. Shows the text of its top left cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Merge {
    pub row0: Arc<str>,
    pub col0: Arc<str>,
    pub row1: Arc<str>,
    pub col1: Arc<str>,
}

/// Cells by the ids of their row and column.
pub type Cells = BTreeMap<(Arc<str>, Arc<str>), Cell>;

/// A table's structure and contents; its size is the object's bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// Top to bottom. Never empty.
    pub rows: Arc<[Line]>,
    /// Left to right. Never empty.
    pub cols: Arc<[Line]>,
    /// Non-empty cells by (row id, column id); cells of removed lines are kept, unseen.
    pub cells: Arc<Cells>,
    /// Merged cells. Ones naming missing lines, or overlapping an earlier one, are ignored.
    pub merges: Arc<[Merge]>,
}

impl Table {
    /// Side of a new table's cells, in logical pixels at the zoom it is placed at.
    pub const CELL_SIZE: (f64, f64) = (120.0, 44.0);
    /// Font size of cell text unless the style sets one, in world units.
    pub const FONT_SIZE: f64 = 16.0;
    /// Space between a cell's edge and its text, in world units.
    const PADDING: f64 = 6.0;

    /// An empty table with lines named by `ids` (taken in order: rows first).
    pub fn new(rows: usize, cols: usize, mut ids: impl FnMut() -> String) -> Self {
        let rows: Arc<[Line]> = (0..rows.max(1)).map(|_| Line::new(ids())).collect();
        let cols: Arc<[Line]> = (0..cols.max(1)).map(|_| Line::new(ids())).collect();
        Self {
            rows,
            cols,
            cells: Arc::default(),
            merges: Arc::new([]),
        }
    }

    /// The merged cells that count, by index.
    pub fn merged(&self) -> Vec<CellRange> {
        let index = |lines: &[Line], id: &str| lines.iter().position(|l| &*l.id == id);
        let mut out: Vec<CellRange> = Vec::new();
        for m in self.merges.iter() {
            let (Some(r0), Some(c0), Some(r1), Some(c1)) = (
                index(&self.rows, &m.row0),
                index(&self.cols, &m.col0),
                index(&self.rows, &m.row1),
                index(&self.cols, &m.col1),
            ) else {
                continue;
            };
            let range = CellRange::new((r0, c0), (r1, c1));
            if !range.is_single() && out.iter().all(|o| !o.intersects(&range)) {
                out.push(range);
            }
        }
        out
    }

    /// The cell (`row`, `col`) is part of: a merged block, or just itself.
    pub fn area(&self, row: usize, col: usize) -> CellRange {
        self.merged()
            .into_iter()
            .find(|m| m.contains((row, col)))
            .unwrap_or(CellRange::cell((row, col)))
    }

    /// `range` grown until no merged block sticks out of it.
    pub fn expand(&self, mut range: CellRange) -> CellRange {
        let merged = self.merged();
        loop {
            let grown = merged
                .iter()
                .filter(|m| m.intersects(&range))
                .fold(range, |r, m| r.union(m));
            if grown == range {
                return range;
            }
            range = grown;
        }
    }

    /// The merges after joining the cells of `range` into one: merges inside it are
    /// replaced. `range` should be [expanded](Self::expand).
    pub fn with_merged(&self, range: CellRange) -> Arc<[Merge]> {
        let mut merges = self.merges_outside(range);
        if !range.is_single()
            && let (Some(r0), Some(c0), Some(r1), Some(c1)) = (
                self.rows.get(range.row0),
                self.cols.get(range.col0),
                self.rows.get(range.row1),
                self.cols.get(range.col1),
            )
        {
            merges.push(Merge {
                row0: r0.id.clone(),
                col0: c0.id.clone(),
                row1: r1.id.clone(),
                col1: c1.id.clone(),
            });
        }
        merges.into()
    }

    /// The merges after splitting every merged block touching `range`.
    pub fn with_split(&self, range: CellRange) -> Arc<[Merge]> {
        self.merges_outside(range).into()
    }

    /// The merges that count and stay clear of `range`.
    fn merges_outside(&self, range: CellRange) -> Vec<Merge> {
        self.merged()
            .into_iter()
            .filter(|m| !m.intersects(&range))
            .map(|m| self.merge_of(m))
            .collect()
    }

    fn merge_of(&self, m: CellRange) -> Merge {
        Merge {
            row0: self.rows[m.row0].id.clone(),
            col0: self.cols[m.col0].id.clone(),
            row1: self.rows[m.row1].id.clone(),
            col1: self.cols[m.col1].id.clone(),
        }
    }

    /// The merges after removing rows (`rows`) or columns `gone`: blocks lose those lines,
    /// and go when one cell or none is left.
    pub fn merges_without(
        &self,
        rows: bool,
        gone: std::ops::RangeInclusive<usize>,
    ) -> Arc<[Merge]> {
        let mut out = Vec::new();
        for m in self.merged() {
            let kept: Vec<usize> = m.lines(rows).filter(|i| !gone.contains(i)).collect();
            let (Some(&first), Some(&last)) = (kept.first(), kept.last()) else {
                continue;
            };
            let shrunk = if rows {
                CellRange {
                    row0: first,
                    row1: last,
                    ..m
                }
            } else {
                CellRange {
                    col0: first,
                    col1: last,
                    ..m
                }
            };
            if !shrunk.is_single() {
                out.push(self.merge_of(shrunk));
            }
        }
        out.into()
    }

    pub fn cell(&self, row: usize, col: usize) -> Option<&Cell> {
        let key = (
            self.rows.get(row)?.id.clone(),
            self.cols.get(col)?.id.clone(),
        );
        self.cells.get(&key)
    }

    /// The text of cell (`row`, `col`), empty if it has none.
    pub fn text(&self, row: usize, col: usize) -> (&str, &Spans) {
        static PLAIN: std::sync::LazyLock<Spans> = std::sync::LazyLock::new(Spans::default);
        self.cell(row, col)
            .map_or(("", &*PLAIN), |c| (&*c.text, &c.spans))
    }

    /// A copy with the text of cell (`row`, `col`) replaced.
    pub fn with_cell(&self, row: usize, col: usize, cell: Cell) -> Self {
        let mut table = self.clone();
        if let (Some(r), Some(c)) = (self.rows.get(row), self.cols.get(col)) {
            let mut cells = (*self.cells).clone();
            cells.insert((r.id.clone(), c.id.clone()), cell);
            table.cells = Arc::new(cells);
        }
        table
    }

    /// Edges of lines with `weights` spread over `from`..`to`: one more than there are lines.
    fn edges(lines: &[Line], from: f64, to: f64) -> Vec<f64> {
        let total: f64 = lines.iter().map(|l| l.weight.max(0.0)).sum();
        let mut edges = vec![from];
        let mut at = 0.0;
        for line in lines {
            at += line.weight.max(0.0);
            let t = if total > 0.0 { at / total } else { 0.0 };
            edges.push(from + (to - from) * t);
        }
        edges
    }

    /// Edges of the rows (top to bottom) and columns (left to right) in `bounds`.
    pub fn grid(&self, bounds: Rect) -> (Vec<f64>, Vec<f64>) {
        (
            Self::edges(&self.rows, bounds.y0, bounds.y1),
            Self::edges(&self.cols, bounds.x0, bounds.x1),
        )
    }

    /// The lines between cells in a table with `bounds`, broken where they would cross
    /// merged blocks.
    pub fn inner_lines(&self, bounds: Rect) -> Vec<(Point, Point)> {
        let (ys, xs) = self.grid(bounds);
        let merged = self.merged();
        // Whether the line between rows (`rows`) or columns `i - 1` and `i` runs inside a
        // merged block in the column or row `across`.
        let inside = |rows: bool, i: usize, across: usize| {
            merged.iter().any(|m| {
                let (along, other) = (m.lines(rows), m.lines(!rows));
                along.contains(&(i - 1)) && along.contains(&i) && other.contains(&across)
            })
        };
        let mut out = Vec::new();
        for (rows, edges, across) in [(true, &ys, &xs), (false, &xs, &ys)] {
            let point = |at: f64, v: f64| {
                if rows {
                    Point::new(v, at)
                } else {
                    Point::new(at, v)
                }
            };
            let cells = across.len() - 1;
            for (i, &at) in edges.iter().enumerate().take(edges.len() - 1).skip(1) {
                let mut from = None;
                // Past the last cell everything is closed.
                for (j, &edge) in across.iter().enumerate() {
                    let open = j < cells && !inside(rows, i, j);
                    match (open, from) {
                        (true, None) => from = Some(edge),
                        (false, Some(start)) => {
                            out.push((point(at, start), point(at, edge)));
                            from = None;
                        }
                        _ => {}
                    }
                }
            }
        }
        out
    }

    /// Cell (`row`, `col`) in a table with `bounds`; all of it for a merged block.
    pub fn cell_rect(&self, bounds: Rect, row: usize, col: usize) -> Option<Rect> {
        self.range_rect(bounds, self.area(row, col))
    }

    /// The single grid cell (`row`, `col`), ignoring merges.
    fn grid_rect(&self, bounds: Rect, row: usize, col: usize) -> Option<Rect> {
        let (ys, xs) = self.grid(bounds);
        Some(Rect::new(
            *xs.get(col)?,
            *ys.get(row)?,
            *xs.get(col + 1)?,
            *ys.get(row + 1)?,
        ))
    }

    /// The area of `range` in a table with `bounds`.
    pub fn range_rect(&self, bounds: Rect, range: CellRange) -> Option<Rect> {
        let a = self.grid_rect(bounds, range.row0, range.col0)?;
        let b = self.grid_rect(bounds, range.row1, range.col1)?;
        Some(a.union(b))
    }

    /// The cell at `point` (in the table's unrotated coordinates), if inside; the top left
    /// one of a merged block.
    pub fn cell_at(&self, bounds: Rect, point: Point) -> Option<(usize, usize)> {
        if !bounds.contains(point) {
            return None;
        }
        let (ys, xs) = self.grid(bounds);
        let find = |edges: &[f64], v: f64| edges.windows(2).position(|w| v >= w[0] && v <= w[1]);
        let range = self.area(find(&ys, point.y)?, find(&xs, point.x)?);
        Some((range.row0, range.col0))
    }

    /// Where the text of cell (`row`, `col`) goes, sized and aligned as `style` says.
    pub fn text_box(&self, bounds: Rect, row: usize, col: usize, style: &Style) -> Option<TextBox> {
        let rect = self.cell_rect(bounds, row, col)?;
        let pad = Self::PADDING
            .min(rect.width() / 4.0)
            .min(rect.height() / 4.0);
        Some(TextBox {
            area: rect.inset(-pad),
            max_size: style.font_size.unwrap_or(Self::FONT_SIZE),
            align: style.text_align,
            valign: style.text_valign,
        })
    }

    /// The cell after (`row`, `col`), reading left to right and wrapping; before it with
    /// `back`. Merged blocks count once, as their top left cell.
    pub fn next_cell(&self, row: usize, col: usize, back: bool) -> (usize, usize) {
        let (rows, cols) = (self.rows.len(), self.cols.len());
        let n = rows * cols;
        let merged = self.merged();
        let mut i = row * cols + col;
        for _ in 0..n {
            i = if back { (i + n - 1) % n } else { (i + 1) % n };
            let cell = (i / cols, i % cols);
            let hidden = merged
                .iter()
                .any(|m| m.contains(cell) && (m.row0, m.col0) != cell);
            if !hidden {
                return cell;
            }
        }
        (row, col)
    }

    /// Default text style for new tables: left-aligned, centered vertically.
    pub fn style() -> Style {
        Style {
            fill: crate::Rgba::rgb(0xFF, 0xFF, 0xFF),
            stroke: crate::Rgba::rgb(0xB8, 0xBC, 0xC4),
            stroke_width: 1.0,
            text_color: Some(crate::Rgba::rgb(0x1F, 0x29, 0x37)),
            text_align: HAlign::Left,
            text_valign: VAlign::Middle,
            ..Style::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: usize, cols: usize) -> Table {
        let mut n = 0;
        Table::new(rows, cols, || {
            n += 1;
            n.to_string()
        })
    }

    #[test]
    fn cells_split_the_bounds_by_weight() {
        let mut t = table(2, 3);
        let bounds = Rect::new(0.0, 0.0, 300.0, 100.0);
        assert_eq!(
            t.cell_rect(bounds, 1, 2),
            Some(Rect::new(200.0, 50.0, 300.0, 100.0))
        );
        let mut cols = t.cols.to_vec();
        cols[0].weight = 2.0;
        t.cols = cols.into();
        assert_eq!(
            t.cell_rect(bounds, 0, 0),
            Some(Rect::new(0.0, 0.0, 150.0, 50.0))
        );
        assert_eq!(t.cell_at(bounds, Point::new(160.0, 60.0)), Some((1, 1)));
        assert_eq!(t.cell_at(bounds, Point::new(400.0, 60.0)), None);
        assert_eq!(t.cell_rect(bounds, 2, 0), None);
    }

    #[test]
    fn merged_cells_act_as_one() {
        let t = table(3, 3);
        let block = CellRange::new((0, 0), (1, 1));
        let t = Table {
            merges: t.with_merged(block),
            ..t
        };
        let bounds = Rect::new(0.0, 0.0, 300.0, 300.0);
        assert_eq!(t.merged(), vec![block]);
        assert_eq!(t.cell_at(bounds, Point::new(150.0, 150.0)), Some((0, 0)));
        assert_eq!(
            t.cell_rect(bounds, 0, 0),
            Some(Rect::new(0.0, 0.0, 200.0, 200.0))
        );
        assert_eq!(t.next_cell(0, 0, false), (0, 2));
        assert_eq!(t.next_cell(0, 2, false), (1, 2));
        assert_eq!(t.next_cell(1, 2, true), (0, 2));
        assert_eq!(t.next_cell(0, 2, true), (0, 0));
        // Lines stop at the block.
        let lines = t.inner_lines(bounds);
        assert!(lines.contains(&(Point::new(200.0, 100.0), Point::new(300.0, 100.0))));
        assert!(lines.contains(&(Point::new(0.0, 200.0), Point::new(300.0, 200.0))));
        assert!(lines.contains(&(Point::new(100.0, 200.0), Point::new(100.0, 300.0))));
        assert!(lines.contains(&(Point::new(200.0, 0.0), Point::new(200.0, 300.0))));
        assert_eq!(lines.len(), 4);
        // Picking part of it picks all of it.
        assert_eq!(
            t.expand(CellRange::new((1, 1), (2, 1))),
            CellRange::new((0, 0), (2, 1))
        );
        // A row added inside widens it; removing its corner row shrinks it.
        let mut rows = t.rows.to_vec();
        rows.insert(1, Line::new("new"));
        let t = Table {
            rows: rows.into(),
            ..t
        };
        assert_eq!(t.merged(), vec![CellRange::new((0, 0), (2, 1))]);
        let merges = t.merges_without(true, 0..=0);
        let rows: Arc<[Line]> = t.rows[1..].into();
        let t = Table { rows, merges, ..t };
        assert_eq!(t.merged(), vec![CellRange::new((0, 0), (1, 1))]);
        // Two cells left still merge, one does not.
        assert_eq!(t.merges_without(false, 1..=1).len(), 1);
        let rows: Arc<[Line]> = t.rows[..1].into();
        let one_row = Table {
            rows,
            merges: t.merges_without(true, 1..=2),
            ..t.clone()
        };
        assert_eq!(one_row.merged(), vec![CellRange::new((0, 0), (0, 1))]);
        assert!(one_row.merges_without(false, 1..=1).is_empty());
        assert!(t.with_split(CellRange::cell((1, 1))).is_empty());
    }

    #[test]
    fn cells_are_keyed_by_line_ids() {
        let t = table(2, 2);
        let cell = Cell {
            text: "x".into(),
            spans: Spans::default(),
        };
        let t = t.with_cell(1, 0, cell);
        assert_eq!(t.text(1, 0).0, "x");
        // A row added above keeps the cell with its row.
        let mut rows = t.rows.to_vec();
        rows.insert(0, Line::new("new"));
        let t = Table {
            rows: rows.into(),
            ..t
        };
        assert_eq!(t.text(2, 0).0, "x");
        assert_eq!(t.text(1, 0).0, "");
        assert_eq!(t.next_cell(2, 1, false), (0, 0));
        assert_eq!(t.next_cell(0, 0, true), (2, 1));
    }
}
