//! Tables: a grid of cells with text, filling the object's bounds.
//!
//! Rows and columns have stable ids, and each cell's text is stored under the ids of its
//! row and column, so concurrent edits of different cells merge, and adding a row on one
//! replica does not move the cells another replica is typing in.

use std::collections::BTreeMap;
use std::sync::Arc;

use kurbo::{Point, Rect};

use crate::{HAlign, Span, Spans, Style, TextBox, VAlign};

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
        }
    }

    pub fn cell(&self, row: usize, col: usize) -> Option<&Cell> {
        let key = (
            self.rows.get(row)?.id.clone(),
            self.cols.get(col)?.id.clone(),
        );
        self.cells.get(&key)
    }

    /// The text of cell (`row`, `col`), empty if it has none.
    pub fn text(&self, row: usize, col: usize) -> (&str, &[Span]) {
        self.cell(row, col)
            .map_or(("", &[]), |c| (&*c.text, &*c.spans))
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

    /// Cell (`row`, `col`) in a table with `bounds`.
    pub fn cell_rect(&self, bounds: Rect, row: usize, col: usize) -> Option<Rect> {
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
        let a = self.cell_rect(bounds, range.row0, range.col0)?;
        let b = self.cell_rect(bounds, range.row1, range.col1)?;
        Some(a.union(b))
    }

    /// The cell at `point` (in the table's unrotated coordinates), if inside.
    pub fn cell_at(&self, bounds: Rect, point: Point) -> Option<(usize, usize)> {
        if !bounds.contains(point) {
            return None;
        }
        let (ys, xs) = self.grid(bounds);
        let find = |edges: &[f64], v: f64| edges.windows(2).position(|w| v >= w[0] && v <= w[1]);
        Some((find(&ys, point.y)?, find(&xs, point.x)?))
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
    /// `back`.
    pub fn next_cell(&self, row: usize, col: usize, back: bool) -> (usize, usize) {
        let (rows, cols) = (self.rows.len(), self.cols.len());
        let i = row * cols + col;
        let n = rows * cols;
        let j = if back { (i + n - 1) % n } else { (i + 1) % n };
        (j / cols, j % cols)
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
