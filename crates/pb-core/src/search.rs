//! Finding text on the board.

use pb_geom::Frame;

use crate::{Document, ObjectData, ObjectId, ObjectKind};

/// A place on the board where the text searched for appears.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// The object it is in (for grouped objects, the object itself, not the group).
    pub id: ObjectId,
    /// Where it is (world): a table cell, a kanban card or column title, a connector's
    /// label, or the whole object.
    pub area: Frame,
}

/// Every place `query` appears, ignoring case, in reading order: top to bottom, then left
/// to right. Searches text, sticky notes, shapes, frame titles, connector labels, table
/// cells, kanban cards and column titles, and the names of objects' tags. Empty for a
/// blank query.
pub fn search(doc: &Document, query: &str) -> Vec<Hit> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let found = |text: &str| text.to_lowercase().contains(&query);
    let tag_ids: Vec<_> = doc
        .tags()
        .iter()
        .filter(|t| found(&t.name))
        .map(|t| t.id.clone())
        .collect();
    let mut hits = Vec::new();
    for object in doc.objects() {
        let data = &object.data;
        let before = hits.len();
        let mut hit = |area: Frame| {
            hits.push(Hit {
                id: object.id,
                area,
            });
        };
        let part = |rect| part_frame(data, rect);
        match &data.kind {
            ObjectKind::Connector(c) => {
                if found(&c.label.text)
                    && let Some(rect) = doc.connector_label(object.id)
                {
                    hit(Frame::axis_aligned(rect));
                }
            }
            ObjectKind::Table(table) => {
                let merged = table.merged();
                for row in 0..table.rows.len() {
                    for col in 0..table.cols.len() {
                        let covered = merged
                            .iter()
                            .any(|m| m.contains((row, col)) && (m.row0, m.col0) != (row, col));
                        if !covered
                            && found(table.text(row, col).0)
                            && let Some(rect) = table.cell_rect(data.bounds, row, col)
                        {
                            hit(part(rect));
                        }
                    }
                }
            }
            ObjectKind::Kanban(kanban) => {
                for col in 0..kanban.columns.len() {
                    if found(&kanban.columns[col].title)
                        && let Some(rect) = kanban.title_rect(data.bounds, col)
                    {
                        hit(part(rect));
                    }
                    for i in kanban.cards_in(col) {
                        if found(&kanban.cards[i].text)
                            && let Some(rect) = kanban.card_rect(data.bounds, i)
                        {
                            hit(part(rect));
                        }
                    }
                }
            }
            kind => {
                if kind.text().is_some_and(found) {
                    hit(data.frame());
                }
            }
        }
        let tagged = data.tags.iter().any(|t| tag_ids.contains(t));
        if tagged && hits.len() == before {
            hits.push(Hit {
                id: object.id,
                area: data.frame(),
            });
        }
    }
    hits.sort_by(|a, b| {
        let (a, b) = (a.area.aabb(), b.area.aabb());
        a.y0.total_cmp(&b.y0).then(a.x0.total_cmp(&b.x0))
    });
    hits
}

/// The part `rect` of object `data` (in its unrotated coordinates), in the world.
fn part_frame(data: &ObjectData, rect: kurbo::Rect) -> Frame {
    Frame::axis_aligned(rect).rotated_about(data.bounds.center(), data.rotation)
}

#[cfg(test)]
mod tests {
    use kurbo::Rect;

    use super::*;
    use crate::{Cell, Command, Kanban, Rgba, Spans, Sticky, Style, Table, Tag};

    fn data(kind: ObjectKind, bounds: Rect) -> ObjectData {
        ObjectData {
            kind,
            bounds,
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        }
    }

    fn sticky(text: &str, x: f64, y: f64) -> ObjectData {
        data(
            ObjectKind::Sticky(Sticky {
                text: text.into(),
                spans: Spans::default(),
            }),
            Rect::new(x, y, x + 100.0, y + 100.0),
        )
    }

    #[test]
    fn finds_text_in_reading_order_ignoring_case() {
        let mut doc = Document::new();
        let items = vec![
            sticky("Купить молоко", 300.0, 0.0).into(),
            sticky("молоко и хлеб", 0.0, 0.0).into(),
            sticky("Позвонить", 0.0, 200.0).into(),
            sticky("МОЛОКО", 0.0, 400.0).into(),
        ];
        doc.apply(Command::Create(items)).unwrap();
        let hits = search(&doc, "Молоко");
        let xs: Vec<_> = hits.iter().map(|h| h.area.rect.origin()).collect();
        assert_eq!(
            xs,
            vec![
                kurbo::Point::new(0.0, 0.0),
                kurbo::Point::new(300.0, 0.0),
                kurbo::Point::new(0.0, 400.0)
            ]
        );
        assert!(search(&doc, "   ").is_empty());
        assert!(search(&doc, "кефир").is_empty());
    }

    #[test]
    fn finds_the_cell_card_or_tag() {
        let mut doc = Document::new();
        let mut table = Table::new(2, 2, || doc.new_line_id());
        table = table.with_cell(
            1,
            1,
            Cell {
                text: "Total".into(),
                spans: Spans::default(),
            },
        );
        let kanban = Kanban::starter(|| doc.new_line_id());
        let card = crate::Card {
            id: doc.new_line_id().into(),
            column: kanban.columns[1].id.clone(),
            order: 0.0,
            text: "Write totals".into(),
            spans: Spans::default(),
        };
        let kanban = kanban.with_card(card);
        let tag = Tag {
            id: doc.new_line_id().into(),
            name: "totally".into(),
            color: Rgba::rgb(0, 0, 0),
        };
        doc.apply(Command::DefineTag(tag.clone())).unwrap();
        let mut tagged = sticky("nothing here", 0.0, 900.0);
        tagged.tags = vec![tag.id.clone()].into();
        let k_bounds = Rect::new(0.0, 300.0, 720.0, 600.0);
        let items = vec![
            data(
                ObjectKind::Table(table.clone()),
                Rect::new(0.0, 0.0, 200.0, 100.0),
            )
            .into(),
            data(ObjectKind::Kanban(kanban.clone()), k_bounds).into(),
            tagged.into(),
        ];
        doc.apply(Command::Create(items)).unwrap();
        let hits = search(&doc, "tota");
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].area.rect, Rect::new(100.0, 50.0, 200.0, 100.0));
        assert_eq!(hits[1].area.rect, kanban.card_rect(k_bounds, 0).unwrap());
        assert_eq!(hits[2].area.rect, Rect::new(0.0, 900.0, 100.0, 1000.0));
    }
}
