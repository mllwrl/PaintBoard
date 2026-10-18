//! The board's text as Markdown, and tables as CSV.

use pb_core::{Link, Object, ObjectKind, Table};

/// Markdown for `objects`, under the heading `title`: frames become second-level
/// headings with what lies in them, everything in reading order (top to bottom, then left
/// to right). Text, notes and shapes are list items, tables are Markdown tables, kanban
/// columns small headings with their cards, mind maps nested lists, and Markdown documents
/// are copied as they are. Pictures, drawings and lines are left out.
pub fn markdown(title: &str, objects: &[&Object]) -> String {
    let mut frames: Vec<&Object> = objects
        .iter()
        .copied()
        .filter(|o| o.data.kind.is_frame())
        .collect();
    sort_for_reading(&mut frames);
    // Each object goes with the smallest frame its middle is in.
    let frame_of = |object: &Object| {
        let center = object.data.frame().aabb().center();
        frames
            .iter()
            .enumerate()
            .filter(|(_, f)| f.data.bounds.contains(center))
            .min_by(|(_, a), (_, b)| a.data.bounds.area().total_cmp(&b.data.bounds.area()))
            .map(|(i, _)| i)
    };
    let mut loose = Vec::new();
    let mut framed = vec![Vec::new(); frames.len()];
    for object in objects.iter().copied().filter(|o| !o.data.kind.is_frame()) {
        match frame_of(object) {
            Some(i) => framed[i].push(object),
            None => loose.push(object),
        }
    }

    let mut out = Blocks::default();
    out.push(format!("# {}", one_line(title)));
    write_objects(&mut out, loose);
    for (frame, members) in frames.iter().zip(framed) {
        let title = frame.data.kind.text().unwrap_or_default().trim();
        let title = if title.is_empty() {
            "Untitled frame"
        } else {
            title
        };
        out.push(format!("## {}", one_line(title)));
        write_objects(&mut out, members);
    }
    out.finish()
}

/// Markdown made of blocks separated by blank lines; list items in a row stay together.
#[derive(Default)]
struct Blocks {
    text: String,
    in_list: bool,
}

impl Blocks {
    fn push(&mut self, block: String) {
        self.separate(false);
        self.text.push_str(&block);
    }

    fn item(&mut self, item: String) {
        self.separate(true);
        self.text.push_str(&item);
    }

    fn separate(&mut self, item: bool) {
        if !self.text.is_empty() {
            self.text
                .push_str(if item && self.in_list { "\n" } else { "\n\n" });
        }
        self.in_list = item;
    }

    fn finish(mut self) -> String {
        self.text.push('\n');
        self.text
    }
}

fn sort_for_reading(objects: &mut [&Object]) {
    objects.sort_by(|a, b| {
        let (a, b) = (a.data.frame().aabb(), b.data.frame().aabb());
        a.y0.total_cmp(&b.y0).then(a.x0.total_cmp(&b.x0))
    });
}

fn write_objects(out: &mut Blocks, mut objects: Vec<&Object>) {
    sort_for_reading(&mut objects);
    for object in objects {
        let link = match &object.data.link {
            Some(Link::Url(url)) => Some(url.to_string()),
            _ => None,
        };
        match &object.data.kind {
            ObjectKind::Doc(page) => out.push(page.text.trim_end().to_owned()),
            ObjectKind::Table(table) => out.push(markdown_table(table)),
            ObjectKind::Kanban(kanban) => {
                for (col, column) in kanban.columns.iter().enumerate() {
                    out.push(format!("### {}", one_line(&column.title)));
                    for i in kanban.cards_in(col) {
                        out.item(list_item(&kanban.cards[i].text, 0));
                    }
                }
            }
            ObjectKind::MindMap(map) => {
                if let Some(root) = map.root() {
                    let mut stack = vec![(root, 0)];
                    while let Some((i, depth)) = stack.pop() {
                        out.item(list_item(&map.nodes[i].text, depth));
                        let children = map.children(i);
                        stack.extend(children.into_iter().rev().map(|c| (c, depth + 1)));
                    }
                }
            }
            ObjectKind::Embed(embed) => {
                let title = if embed.title.trim().is_empty() {
                    &embed.url
                } else {
                    &embed.title
                };
                out.item(format!("- [{}]({})", one_line(title), embed.url));
            }
            kind => {
                let Some(text) = kind.text().filter(|t| !t.trim().is_empty()) else {
                    continue;
                };
                match link {
                    Some(url) => out.item(format!("- [{}]({url})", one_line(text))),
                    None => out.item(list_item(text, 0)),
                }
            }
        }
    }
}

/// A list item `depth` levels in; lines after the first are indented to stay in it.
fn list_item(text: &str, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut lines = text.trim().lines();
    let mut item = format!("{indent}- {}", lines.next().unwrap_or_default());
    for line in lines {
        item.push_str(&format!("\n{indent}  {line}"));
    }
    item
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A Markdown table; the first row is its header.
fn markdown_table(table: &Table) -> String {
    let cell = |row: usize, col: usize| {
        let text = table.text(row, col).0.trim();
        text.replace('|', "\\|").replace('\n', "<br>")
    };
    let line = |row: usize| {
        let cells: Vec<String> = (0..table.cols.len()).map(|col| cell(row, col)).collect();
        format!("| {} |", cells.join(" | "))
    };
    let mut lines = vec![line(0)];
    lines.push(format!("|{}", " --- |".repeat(table.cols.len())));
    lines.extend((1..table.rows.len()).map(line));
    lines.join("\n")
}

/// The table as CSV that spreadsheets open: comma-separated, quoted where needed, CRLF
/// line ends, with a byte order mark so Excel reads it as UTF-8.
pub fn table_csv(table: &Table) -> String {
    let field = |text: &str| {
        if text.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.to_owned()
        }
    };
    let mut out = String::from('\u{feff}');
    for row in 0..table.rows.len() {
        let cells: Vec<String> = (0..table.cols.len())
            .map(|col| field(table.text(row, col).0))
            .collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use pb_core::{Cell, Command, Document, FrameData, ObjectData, Spans, Sticky, Style};
    use pb_geom::kurbo::Rect;

    use super::*;

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
        let kind = ObjectKind::Sticky(Sticky {
            text: text.into(),
            spans: Spans::default(),
        });
        data(kind, Rect::new(x, y, x + 100.0, y + 100.0))
    }

    fn table(doc: &Document, cells: &[&[&str]]) -> Table {
        let mut table = Table::new(cells.len(), cells[0].len(), || doc.new_line_id());
        for (r, row) in cells.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                let cell = Cell {
                    text: (*text).into(),
                    spans: Spans::default(),
                };
                table = table.with_cell(r, c, cell);
            }
        }
        table
    }

    #[test]
    fn frames_become_sections_in_reading_order() {
        let mut doc = Document::new();
        let frame = |title: &str, x: f64| {
            let kind = ObjectKind::Frame(FrameData {
                title: title.into(),
            });
            data(kind, Rect::new(x, 0.0, x + 500.0, 500.0))
        };
        let items = vec![
            frame("Ideas", 0.0).into(),
            frame("", 1000.0).into(),
            sticky("Second\nline two", 200.0, 200.0).into(),
            sticky("First", 50.0, 50.0).into(),
            sticky("Elsewhere", 0.0, 2000.0).into(),
            sticky("In the other", 1100.0, 100.0).into(),
        ];
        doc.apply(Command::Create(items)).unwrap();
        let objects: Vec<&Object> = doc.objects().iter().collect();
        assert_eq!(
            markdown("My  board", &objects),
            "# My board\n\n- Elsewhere\n\n## Ideas\n\n- First\n- Second\n  line two\n\n\
             ## Untitled frame\n\n- In the other\n"
        );
    }

    #[test]
    fn tables_become_markdown_and_csv() {
        let doc = Document::new();
        let table = table(
            &doc,
            &[
                &["Name", "Note"],
                &["Ann", "a|b\nc"],
                &["Bob", "say \"hi\", ok"],
            ],
        );
        assert_eq!(
            markdown_table(&table),
            "| Name | Note |\n| --- | --- |\n| Ann | a\\|b<br>c |\n| Bob | say \"hi\", ok |"
        );
        assert_eq!(
            table_csv(&table),
            "\u{feff}Name,Note\r\nAnn,\"a|b\nc\"\r\nBob,\"say \"\"hi\"\", ok\"\r\n"
        );
        // What CSV import reads back is what was written.
        let rows = crate::csv::parse(&table_csv(&table));
        assert_eq!(rows[2], vec!["Bob".to_owned(), "say \"hi\", ok".to_owned()]);
    }
}
