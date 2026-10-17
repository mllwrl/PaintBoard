//! Kanban boards: columns of cards with text, laid out across the object's width.
//!
//! Columns and cards have stable ids. Each card names its column and has a fractional
//! `order` within it, so moving a card changes that card only, and replicas moving or
//! typing in different cards merge.

use std::sync::Arc;

use kurbo::{Point, Rect};

use crate::{HAlign, Spans, Style, TextBox, VAlign};

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// Unique within its board (see [`crate::Document::new_line_id`]).
    pub id: Arc<str>,
    pub title: Arc<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// Unique within its board.
    pub id: Arc<str>,
    /// The id of the column it is in; cards of removed columns are kept, unseen.
    pub column: Arc<str>,
    /// Cards in a column go by `order`, then by id.
    pub order: f64,
    pub text: Arc<str>,
    pub spans: Spans,
}

/// A part of a board that a click can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KanbanPart {
    /// The title of column `i`.
    Title(usize),
    /// Card `i` of [`Kanban::cards`].
    Card(usize),
    /// The "add card" row under the cards of column `i`.
    Add(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Kanban {
    /// Left to right. Never empty.
    pub columns: Arc<[Column]>,
    /// In no particular order.
    pub cards: Arc<[Card]>,
}

/// Sizes at a column width of [`Kanban::COLUMN_WIDTH`]; they scale with it.
const PAD: f64 = 8.0;
const HEADER: f64 = 40.0;
const CARD_HEIGHT: f64 = 72.0;
const CARD_GAP: f64 = 8.0;
const ADD_HEIGHT: f64 = 32.0;
const CARD_FONT: f64 = 15.0;
const TITLE_FONT: f64 = 16.0;

impl Kanban {
    /// Width of a new board's columns, in logical pixels at the zoom it is placed at.
    pub const COLUMN_WIDTH: f64 = 240.0;
    /// Label of the row that adds cards.
    pub const ADD_LABEL: &str = "+ Add card";

    /// A board with empty columns titled `titles`, named by `ids`.
    pub fn new(titles: &[&str], mut ids: impl FnMut() -> String) -> Self {
        let columns: Arc<[Column]> = titles
            .iter()
            .map(|t| Column {
                id: ids().into(),
                title: (*t).into(),
            })
            .collect();
        Self {
            columns,
            cards: Arc::new([]),
        }
    }

    /// Default columns of a new board.
    pub fn starter(ids: impl FnMut() -> String) -> Self {
        Self::new(&["To do", "In progress", "Done"], ids)
    }

    /// The size of the layout's units in a board `width` wide.
    fn unit(&self, width: f64) -> f64 {
        width / self.columns.len().max(1) as f64 / Self::COLUMN_WIDTH
    }

    /// Indices of the cards in column `col`, top to bottom.
    pub fn cards_in(&self, col: usize) -> Vec<usize> {
        let Some(column) = self.columns.get(col) else {
            return Vec::new();
        };
        let mut cards: Vec<usize> = (0..self.cards.len())
            .filter(|&i| self.cards[i].column == column.id)
            .collect();
        cards.sort_by(|&a, &b| {
            let (a, b) = (&self.cards[a], &self.cards[b]);
            a.order.total_cmp(&b.order).then_with(|| a.id.cmp(&b.id))
        });
        cards
    }

    /// The column card `i` is in and its place there, if it is shown.
    pub fn place_of(&self, i: usize) -> Option<(usize, usize)> {
        let card = self.cards.get(i)?;
        let col = self.columns.iter().position(|c| c.id == card.column)?;
        let at = self.cards_in(col).iter().position(|&j| j == i)?;
        Some((col, at))
    }

    pub fn card_index(&self, id: &str) -> Option<usize> {
        self.cards.iter().position(|c| &*c.id == id)
    }

    /// The whole of column `col` in a board with `bounds`.
    pub fn column_rect(&self, bounds: Rect, col: usize) -> Option<Rect> {
        if col >= self.columns.len() {
            return None;
        }
        let w = bounds.width() / self.columns.len() as f64;
        let x = bounds.x0 + w * col as f64;
        let inset = PAD / 2.0 * self.unit(bounds.width());
        Some(Rect::new(x + inset, bounds.y0, x + w - inset, bounds.y1))
    }

    pub fn title_rect(&self, bounds: Rect, col: usize) -> Option<Rect> {
        let r = self.column_rect(bounds, col)?;
        let u = self.unit(bounds.width());
        Some(Rect::new(
            r.x0 + PAD * u,
            r.y0,
            r.x1 - PAD * u,
            r.y0 + HEADER * u,
        ))
    }

    /// Slot `at` of column `col`, where its card goes.
    fn slot(&self, bounds: Rect, col: usize, at: usize) -> Option<Rect> {
        let r = self.column_rect(bounds, col)?;
        let u = self.unit(bounds.width());
        let y = r.y0 + (HEADER + at as f64 * (CARD_HEIGHT + CARD_GAP)) * u;
        Some(Rect::new(
            r.x0 + PAD * u,
            y,
            r.x1 - PAD * u,
            y + CARD_HEIGHT * u,
        ))
    }

    pub fn card_rect(&self, bounds: Rect, i: usize) -> Option<Rect> {
        let (col, at) = self.place_of(i)?;
        self.slot(bounds, col, at)
    }

    /// The "add card" row of column `col`.
    pub fn add_rect(&self, bounds: Rect, col: usize) -> Option<Rect> {
        let n = self.cards_in(col).len();
        let slot = self.slot(bounds, col, n)?;
        let u = self.unit(bounds.width());
        Some(Rect::new(
            slot.x0,
            slot.y0,
            slot.x1,
            slot.y0 + ADD_HEIGHT * u,
        ))
    }

    /// The height that fits every column of a board `width` wide.
    pub fn fitted_height(&self, width: f64) -> f64 {
        let most = (0..self.columns.len())
            .map(|c| self.cards_in(c).len())
            .max()
            .unwrap_or(0);
        let u = self.unit(width);
        (HEADER + most as f64 * (CARD_HEIGHT + CARD_GAP) + ADD_HEIGHT + PAD) * u
    }

    /// `bounds` with its height fitted to the columns, keeping the top left.
    pub fn fitted(&self, bounds: Rect) -> Rect {
        let height = self.fitted_height(bounds.width());
        Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + height)
    }

    /// The part at `point` (in the board's unrotated coordinates), if any.
    pub fn part_at(&self, bounds: Rect, point: Point) -> Option<KanbanPart> {
        for col in 0..self.columns.len() {
            if self.title_rect(bounds, col)?.contains(point) {
                return Some(KanbanPart::Title(col));
            }
            for i in self.cards_in(col) {
                if self.card_rect(bounds, i)?.contains(point) {
                    return Some(KanbanPart::Card(i));
                }
            }
            if self.add_rect(bounds, col)?.contains(point) {
                return Some(KanbanPart::Add(col));
            }
        }
        None
    }

    /// Where a card whose middle is at `point` would be dropped: a column, and the place
    /// among its cards other than `moving`.
    pub fn drop_at(&self, bounds: Rect, point: Point, moving: Option<usize>) -> (usize, usize) {
        let n = self.columns.len();
        let w = bounds.width() / n as f64;
        let col = if w > 0.0 {
            (((point.x - bounds.x0) / w).floor().max(0.0) as usize).min(n - 1)
        } else {
            0
        };
        let others = self
            .cards_in(col)
            .into_iter()
            .filter(|&i| Some(i) != moving);
        let count = others.count();
        let u = self.unit(bounds.width());
        let pitch = (CARD_HEIGHT + CARD_GAP) * u;
        let first = bounds.y0 + (HEADER + CARD_HEIGHT / 2.0) * u;
        let at = if pitch > 0.0 {
            ((point.y - first) / pitch).round().max(0.0) as usize
        } else {
            0
        };
        (col, at.min(count))
    }

    /// The `order` for a card put at place `at` of column `col`, among the cards there
    /// other than `moving`.
    pub fn order_at(&self, col: usize, at: usize, moving: Option<usize>) -> f64 {
        let others: Vec<f64> = self
            .cards_in(col)
            .into_iter()
            .filter(|&i| Some(i) != moving)
            .map(|i| self.cards[i].order)
            .collect();
        let before = at.checked_sub(1).and_then(|i| others.get(i));
        match (before, others.get(at)) {
            (Some(a), Some(b)) => (a + b) / 2.0,
            (Some(a), None) => a + 1.0,
            (None, Some(b)) => b - 1.0,
            (None, None) => 0.0,
        }
    }

    /// A copy with card `card` put in place of the card with its id, or added.
    pub fn with_card(&self, card: Card) -> Self {
        let mut cards = self.cards.to_vec();
        match cards.iter_mut().find(|c| c.id == card.id) {
            Some(old) => *old = card,
            None => cards.push(card),
        }
        Self {
            cards: cards.into(),
            ..self.clone()
        }
    }

    /// Where the text of card `i` goes.
    pub fn card_text_box(&self, bounds: Rect, i: usize) -> Option<TextBox> {
        let rect = self.card_rect(bounds, i)?;
        let u = self.unit(bounds.width());
        Some(TextBox {
            area: rect.inset(-PAD * u),
            max_size: CARD_FONT * u,
            align: HAlign::Left,
            valign: VAlign::Top,
        })
    }

    /// Where the title of column `col` goes.
    pub fn title_text_box(&self, bounds: Rect, col: usize) -> Option<TextBox> {
        let rect = self.title_rect(bounds, col)?;
        let u = self.unit(bounds.width());
        Some(TextBox {
            area: rect,
            max_size: TITLE_FONT * u,
            align: HAlign::Left,
            valign: VAlign::Middle,
        })
    }

    /// Where the "add card" label of column `col` goes.
    pub fn add_text_box(&self, bounds: Rect, col: usize) -> Option<TextBox> {
        let rect = self.add_rect(bounds, col)?;
        let u = self.unit(bounds.width());
        Some(TextBox {
            area: rect.inset((-PAD * u, 0.0)),
            max_size: CARD_FONT * u,
            align: HAlign::Left,
            valign: VAlign::Middle,
        })
    }

    /// Inline styles of a column title: bold all through.
    pub fn title_spans(title: &str) -> Spans {
        if title.is_empty() {
            return Spans::default();
        }
        Arc::new([crate::Span {
            start: 0,
            end: title.len(),
            marks: crate::TextMarks {
                bold: true,
                ..Default::default()
            },
        }])
    }

    /// Corner radius of columns and cards.
    pub fn radius(&self, width: f64) -> f64 {
        6.0 * self.unit(width)
    }

    /// Default look of new boards: light gray columns, white cards with a gray outline.
    pub fn style() -> Style {
        Style {
            fill: crate::Rgba::rgb(0xEE, 0xF0, 0xF3),
            stroke: crate::Rgba::rgb(0xD0, 0xD4, 0xDA),
            stroke_width: 1.0,
            text_color: Some(crate::Rgba::rgb(0x1F, 0x29, 0x37)),
            ..Style::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Kanban {
        let mut n = 0;
        let k = Kanban::new(&["A", "B"], || {
            n += 1;
            format!("c{n}")
        });
        let card = |id: &str, col: &str, order: f64| Card {
            id: id.into(),
            column: col.into(),
            order,
            text: id.into(),
            spans: Spans::default(),
        };
        k.with_card(card("x", "c1", 1.0))
            .with_card(card("y", "c1", 0.0))
            .with_card(card("z", "c2", 0.0))
            .with_card(card("lost", "gone", 0.0))
    }

    #[test]
    fn cards_are_laid_out_by_column_and_order() {
        let k = board();
        // Two columns 240 wide: the layout is at its natural size.
        let bounds = Rect::new(0.0, 0.0, 480.0, 1000.0);
        let x = k.card_index("x").unwrap();
        let y = k.card_index("y").unwrap();
        assert_eq!(k.cards_in(0), vec![y, x]);
        assert_eq!(k.place_of(x), Some((0, 1)));
        assert_eq!(k.place_of(k.card_index("lost").unwrap()), None);
        let r = k.card_rect(bounds, x).unwrap();
        assert_eq!(r, Rect::new(12.0, 120.0, 228.0, 192.0));
        assert_eq!(k.part_at(bounds, r.center()), Some(KanbanPart::Card(x)));
        assert_eq!(
            k.part_at(bounds, Point::new(300.0, 20.0)),
            Some(KanbanPart::Title(1))
        );
        let add = k.add_rect(bounds, 1).unwrap();
        assert_eq!(k.part_at(bounds, add.center()), Some(KanbanPart::Add(1)));
        assert_eq!(k.fitted_height(480.0), 40.0 + 2.0 * 80.0 + 32.0 + 8.0);
        // Twice as wide, twice as big.
        assert_eq!(k.fitted_height(960.0), 2.0 * k.fitted_height(480.0));
    }

    #[test]
    fn dropped_cards_get_an_order_between_their_neighbors() {
        let k = board();
        let bounds = Rect::new(0.0, 0.0, 480.0, 1000.0);
        let x = k.card_index("x").unwrap();
        let y = k.card_index("y").unwrap();
        // Over the first slot of the first column, x goes before y.
        let first = k.card_rect(bounds, y).unwrap().center();
        assert_eq!(k.drop_at(bounds, first, Some(x)), (0, 0));
        assert_eq!(k.order_at(0, 0, Some(x)), -1.0);
        // Far below, it goes last.
        assert_eq!(k.drop_at(bounds, Point::new(100.0, 900.0), Some(x)), (0, 1));
        assert_eq!(k.order_at(0, 1, Some(x)), 1.0);
        // Between y and x in the other column.
        assert_eq!(k.drop_at(bounds, Point::new(300.0, 150.0), Some(y)), (1, 1));
        assert_eq!(k.order_at(1, 1, Some(y)), 1.0);
        assert_eq!(k.order_at(0, 1, None), 0.5);
    }
}
