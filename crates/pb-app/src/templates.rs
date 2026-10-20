//! Ready-made boards to start from: frames, notes, tables and the like, laid out in code.
//! All our own designs.

use std::sync::Arc;

use pb_core::{
    Card, Cell, Content, Document, FrameData, HAlign, Kanban, MindMap, Node, ObjectData,
    ObjectKind, Rgba, Shape, ShapeKind, Span, Spans, Sticky, Style, Table, Text, TextMarks, VAlign,
};
use pb_geom::kurbo::{Rect, Size, Vec2};
use pb_text::TextEngine;

use crate::i18n::{Lang, tr_in};

/// A template: what it is called, what it is for, and how it is made.
pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub make: Make,
}

/// How a template is made.
pub enum Make {
    /// Objects built in code.
    Objects(fn(&mut Builder)),
    /// A Mermaid flowchart (see `pb_io::mermaid`), its labels in the given language.
    Mermaid(fn(Lang) -> String),
}

pub const TEMPLATES: [Template; 9] = [
    Template {
        name: "SWOT analysis",
        description: "Strengths, weaknesses, opportunities and threats in four squares",
        make: Make::Objects(swot),
    },
    Template {
        name: "Retrospective",
        description: "What went well, what to improve, and the actions to take",
        make: Make::Objects(retro),
    },
    Template {
        name: "Kanban",
        description: "A board with To do, In progress and Done columns",
        make: Make::Objects(kanban),
    },
    Template {
        name: "Roadmap",
        description: "Teams by quarter, in a table",
        make: Make::Objects(roadmap),
    },
    Template {
        name: "User story map",
        description: "Activities, the steps under them, and stories by release",
        make: Make::Objects(story_map),
    },
    Template {
        name: "Customer journey",
        description: "Stages of the journey against actions, thoughts, feelings and more",
        make: Make::Objects(journey),
    },
    Template {
        name: "Brainwriting 6-3-5",
        description: "Six people, three ideas each, five rounds of building on them",
        make: Make::Objects(brainwriting),
    },
    Template {
        name: "Mind map",
        description: "A central topic with four branches to grow",
        make: Make::Objects(mind_map),
    },
    Template {
        name: "Flowchart",
        description: "Start, steps, a decision and the end, joined by arrows",
        make: Make::Mermaid(flowchart),
    },
];

/// Note colors used by the templates.
const YELLOW: Rgba = Rgba::rgb(0xFF, 0xF1, 0x76);
const ORANGE: Rgba = Rgba::rgb(0xFF, 0xD0, 0x80);
const PINK: Rgba = Rgba::rgb(0xF8, 0xBB, 0xD9);
const BLUE: Rgba = Rgba::rgb(0xB3, 0xE0, 0xFF);
const GREEN: Rgba = Rgba::rgb(0xB9, 0xF0, 0xC8);
const PURPLE: Rgba = Rgba::rgb(0xD1, 0xC4, 0xE9);
const GRAY: Rgba = Rgba::rgb(0xF0, 0xF0, 0xF0);
const INK: Rgba = Rgba::rgb(0x1F, 0x29, 0x37);
/// Side of the templates' sticky notes, in world units.
const NOTE: f64 = 140.0;

/// Collects a template's objects, in world units around the origin.
pub struct Builder<'a> {
    doc: &'a Document,
    engine: &'a mut TextEngine,
    items: Vec<ObjectData>,
    /// The language of the texts put on the board.
    lang: Lang,
}

impl<'a> Builder<'a> {
    /// A builder putting texts on the board in the interface's language.
    pub fn new(doc: &'a Document, engine: &'a mut TextEngine) -> Self {
        Self {
            doc,
            engine,
            items: Vec::new(),
            lang: crate::i18n::lang(),
        }
    }

    /// Puts the texts on the board in `lang` instead.
    #[cfg(test)]
    pub fn in_lang(mut self, lang: Lang) -> Self {
        self.lang = lang;
        self
    }

    /// `en` in the builder's language.
    fn tr(&self, en: &'static str) -> &'static str {
        tr_in(self.lang, en)
    }

    /// The objects made, frames first, centered on `center`.
    pub fn finish(mut self, center: pb_geom::kurbo::Point) -> Vec<Content> {
        let all = self
            .items
            .iter()
            .map(|d| d.bounds)
            .reduce(|a, b| a.union(b))
            .unwrap_or_default();
        let delta: Vec2 = center - all.center();
        self.items.sort_by_key(|d| !d.kind.is_frame());
        self.items
            .into_iter()
            .map(|mut d| {
                d.translate(delta);
                d.into()
            })
            .collect()
    }

    fn push(&mut self, kind: ObjectKind, bounds: Rect, style: Style) {
        self.items.push(ObjectData {
            kind,
            bounds,
            rotation: 0.0,
            style,
            tags: Default::default(),
            link: None,
        });
    }

    fn frame(&mut self, x: f64, y: f64, w: f64, h: f64, title: &str) {
        self.push(
            ObjectKind::Frame(FrameData {
                title: title.into(),
            }),
            Rect::new(x, y, x + w, y + h),
            Style {
                fill: Rgba::rgb(0xFF, 0xFF, 0xFF),
                ..Style::default()
            },
        );
    }

    fn note(&mut self, x: f64, y: f64, text: &str, color: Rgba) {
        self.push(
            ObjectKind::Sticky(Sticky {
                text: text.into(),
                spans: Spans::default(),
            }),
            Rect::new(x, y, x + NOTE, y + NOTE),
            Style {
                fill: color,
                stroke_width: 0.0,
                ..Style::default()
            },
        );
    }

    /// An area of `fill` with `title` at its top.
    fn area(&mut self, x: f64, y: f64, w: f64, h: f64, title: &str, fill: Rgba) {
        self.push(
            ObjectKind::Shape(Shape {
                kind: ShapeKind::RoundedRectangle,
                text: title.into(),
                spans: bold(title),
            }),
            Rect::new(x, y, x + w, y + h),
            Style {
                fill,
                stroke_width: 0.0,
                text_color: Some(INK),
                font_size: Some(24.0),
                text_valign: VAlign::Top,
                ..Style::default()
            },
        );
    }

    /// A text object with its top left at (`x`, `y`).
    fn heading(&mut self, x: f64, y: f64, text: &str, size: f64) {
        // Measured bold, as it is drawn, so that it stays on one line.
        let spans = bold(text);
        let layout =
            self.engine
                .layout_aligned(text, &spans, size, None, pb_text::TextAlign::Start);
        self.push(
            ObjectKind::Text(Text {
                text: text.into(),
                spans,
                size,
            }),
            Rect::from_origin_size((x, y), (layout.width, layout.height)),
            Style {
                stroke: INK,
                ..Style::default()
            },
        );
    }

    /// A table with `rows` of cell texts; the first row and column are bold.
    fn table(&mut self, x: f64, y: f64, w: f64, h: f64, rows: &[&[&str]]) {
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
        let mut table = Table::new(rows.len(), cols, || self.doc.new_line_id());
        // A wider first column for the row names.
        let mut lines = table.cols.to_vec();
        lines[0].weight = 0.7;
        table.cols = lines.into();
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                if text.is_empty() {
                    continue;
                }
                let spans = if r == 0 || c == 0 {
                    bold(text)
                } else {
                    Spans::default()
                };
                table = table.with_cell(
                    r,
                    c,
                    Cell {
                        text: (*text).into(),
                        spans,
                    },
                );
            }
        }
        self.push(
            ObjectKind::Table(table),
            Rect::new(x, y, x + w, y + h),
            Style {
                text_align: HAlign::Left,
                ..Table::style()
            },
        );
    }
}

/// Spans making all of `text` bold.
fn bold(text: &str) -> Spans {
    if text.is_empty() {
        return Spans::default();
    }
    Arc::new([Span {
        start: 0,
        end: text.len(),
        marks: TextMarks {
            bold: true,
            ..Default::default()
        },
    }])
}

fn swot(b: &mut Builder) {
    let (w, h, gap) = (520.0, 360.0, 24.0);
    b.frame(
        0.0,
        0.0,
        2.0 * w + 3.0 * gap,
        2.0 * h + 3.0 * gap,
        b.tr("SWOT analysis"),
    );
    let squares = [
        (b.tr("Strengths"), GREEN, b.tr("What we do well")),
        (b.tr("Weaknesses"), PINK, b.tr("Where we fall short")),
        (b.tr("Opportunities"), BLUE, b.tr("Chances to grow")),
        (b.tr("Threats"), ORANGE, b.tr("What could hurt us")),
    ];
    for (i, (title, color, hint)) in squares.into_iter().enumerate() {
        let x = gap + (i % 2) as f64 * (w + gap);
        let y = gap + (i / 2) as f64 * (h + gap);
        b.area(x, y, w, h, title, lighter(color));
        b.note(x + 24.0, y + 80.0, hint, color);
    }
}

fn retro(b: &mut Builder) {
    let (w, h, gap) = (420.0, 640.0, 24.0);
    b.frame(
        0.0,
        0.0,
        3.0 * w + 4.0 * gap,
        h + 2.0 * gap,
        b.tr("Retrospective"),
    );
    let columns = [
        (
            b.tr("What went well"),
            GREEN,
            [b.tr("A win worth keeping"), b.tr("Someone to thank")],
        ),
        (
            b.tr("What to improve"),
            PINK,
            [b.tr("Something that slowed us"), b.tr("A surprise")],
        ),
        (
            b.tr("Actions"),
            BLUE,
            [b.tr("One thing to try"), b.tr("Who does it, by when")],
        ),
    ];
    for (i, (title, color, notes)) in columns.into_iter().enumerate() {
        let x = gap + i as f64 * (w + gap);
        b.area(x, gap, w, h, title, lighter(color));
        for (j, text) in notes.into_iter().enumerate() {
            b.note(x + 24.0 + j as f64 * (NOTE + 20.0), gap + 90.0, text, color);
        }
    }
}

fn kanban(b: &mut Builder) {
    let titles = [b.tr("To do"), b.tr("In progress"), b.tr("Done")];
    let mut board = Kanban::new(&titles, || b.doc.new_line_id());
    let cards = [
        (0, b.tr("Plan the next release")),
        (0, b.tr("Write the docs")),
        (1, b.tr("Fix the login bug")),
        (2, b.tr("Set up the project")),
    ];
    for (n, (col, text)) in cards.into_iter().enumerate() {
        let card = Card {
            id: b.doc.new_line_id().into(),
            column: board.columns[col].id.clone(),
            order: n as f64,
            text: text.into(),
            spans: Spans::default(),
        };
        board = board.with_card(card);
    }
    let bounds = board.fitted(Rect::new(
        48.0,
        48.0,
        48.0 + 3.0 * Kanban::COLUMN_WIDTH * 1.3,
        49.0,
    ));
    let frame_h = bounds.height() + 96.0;
    b.frame(0.0, 0.0, bounds.width() + 96.0, frame_h, b.tr("Kanban"));
    b.push(ObjectKind::Kanban(board), bounds, Kanban::style());
}

fn roadmap(b: &mut Builder) {
    b.frame(0.0, 0.0, 1400.0, 620.0, b.tr("Roadmap"));
    b.heading(40.0, 36.0, b.tr("Product roadmap"), 32.0);
    let rows: [[&str; 5]; 5] = [
        ["", b.tr("Q1"), b.tr("Q2"), b.tr("Q3"), b.tr("Q4")],
        [
            b.tr("Product"),
            b.tr("Research users"),
            b.tr("Beta"),
            b.tr("Launch"),
            b.tr("Grow"),
        ],
        [
            b.tr("Design"),
            b.tr("Prototype"),
            b.tr("Design system"),
            "",
            "",
        ],
        [
            b.tr("Engineering"),
            b.tr("Architecture"),
            b.tr("Core features"),
            b.tr("Performance"),
            b.tr("Scale"),
        ],
        [
            b.tr("Marketing"),
            "",
            b.tr("Landing page"),
            b.tr("Launch campaign"),
            "",
        ],
    ];
    let rows: Vec<&[&str]> = rows.iter().map(|r| &r[..]).collect();
    b.table(40.0, 110.0, 1320.0, 470.0, &rows);
}

fn story_map(b: &mut Builder) {
    let col = NOTE + 30.0;
    let activities = [
        b.tr("Discover"),
        b.tr("Sign up"),
        b.tr("Use"),
        b.tr("Share"),
    ];
    let width = 220.0 + activities.len() as f64 * 2.0 * col + 40.0;
    b.frame(0.0, 0.0, width, 980.0, b.tr("User story map"));
    b.heading(40.0, 40.0, b.tr("Activities"), 22.0);
    b.heading(40.0, 220.0, b.tr("Steps"), 22.0);
    let releases = [b.tr("Release 1"), b.tr("Release 2")];
    for (i, release) in releases.iter().enumerate() {
        let y = 420.0 + i as f64 * 270.0;
        b.area(20.0, y - 20.0, width - 40.0, 250.0, release, GRAY);
    }
    for (i, activity) in activities.iter().enumerate() {
        let x = 220.0 + i as f64 * 2.0 * col;
        b.note(x, 40.0, activity, BLUE);
        for s in 0..2 {
            b.note(x + s as f64 * col, 220.0, b.tr("A step"), YELLOW);
        }
        b.note(x, 450.0, b.tr("A story"), PINK);
        b.note(x, 720.0, b.tr("A later story"), PINK);
    }
}

fn journey(b: &mut Builder) {
    b.frame(0.0, 0.0, 1500.0, 760.0, b.tr("Customer journey"));
    b.heading(40.0, 36.0, b.tr("Customer journey map"), 32.0);
    let stages = [
        "",
        b.tr("Awareness"),
        b.tr("Consideration"),
        b.tr("Purchase"),
        b.tr("Retention"),
        b.tr("Advocacy"),
    ];
    let lanes = [
        b.tr("Actions"),
        b.tr("Thoughts"),
        b.tr("Feelings"),
        b.tr("Pain points"),
        b.tr("Opportunities"),
    ];
    let lanes: Vec<[&str; 6]> = lanes.iter().map(|l| [l, "", "", "", "", ""]).collect();
    let mut rows: Vec<&[&str]> = vec![&stages];
    rows.extend(lanes.iter().map(|r| &r[..]));
    b.table(40.0, 110.0, 1420.0, 610.0, &rows);
}

fn brainwriting(b: &mut Builder) {
    let (people, ideas) = (6, 3);
    let col = ideas as f64 * (NOTE + 12.0) + 40.0;
    let row = NOTE + 50.0;
    let rounds = 5;
    b.frame(
        0.0,
        0.0,
        180.0 + people as f64 * col,
        120.0 + rounds as f64 * row,
        b.tr("Brainwriting 6-3-5"),
    );
    let colors = [YELLOW, ORANGE, PINK, BLUE, GREEN, PURPLE];
    let person = b.tr("Person {}");
    for (p, &color) in colors.iter().enumerate().take(people) {
        let x = 180.0 + p as f64 * col;
        b.heading(x, 40.0, &person.replace("{}", &(p + 1).to_string()), 20.0);
        for r in 0..rounds {
            for i in 0..ideas {
                let y = 100.0 + r as f64 * row;
                b.note(x + i as f64 * (NOTE + 12.0), y, "", color);
            }
        }
    }
    let round = b.tr("Round {}");
    for r in 0..rounds {
        b.heading(
            30.0,
            150.0 + r as f64 * row,
            &round.replace("{}", &(r + 1).to_string()),
            20.0,
        );
    }
}

fn mind_map(b: &mut Builder) {
    let measure = |engine: &mut TextEngine, text: &str, size: f64| {
        let l = engine.layout(text, size, Some(MindMap::WRAP_WIDTH));
        Size::new(l.width, l.height)
    };
    let probe = MindMap::new("probe", "", Size::ZERO);
    let root_size = probe.font_size(0);
    let root_id: Arc<str> = b.doc.new_line_id().into();
    let main = b.tr("Main idea");
    let mut map = MindMap::new(root_id.clone(), main, measure(b.engine, main, root_size));
    let branches = [
        (b.tr("Goals"), false),
        (b.tr("Ideas"), false),
        (b.tr("Questions"), true),
        (b.tr("Next steps"), true),
    ];
    for (n, (text, left)) in branches.into_iter().enumerate() {
        let id: Arc<str> = b.doc.new_line_id().into();
        let child_size = 16.0;
        map = map.with_node(Node {
            id: id.clone(),
            parent: Some(root_id.clone()),
            order: n as f64,
            text: text.into(),
            spans: Spans::default(),
            text_size: measure(b.engine, text, child_size),
            collapsed: false,
            left,
            color: None,
        });
        for k in 0..2 {
            let sub = format!("{text} {}", k + 1);
            map = map.with_node(Node {
                id: b.doc.new_line_id().into(),
                parent: Some(id.clone()),
                order: k as f64,
                text: sub.as_str().into(),
                spans: Spans::default(),
                text_size: measure(b.engine, &sub, child_size),
                collapsed: false,
                left,
                color: None,
            });
        }
    }
    let size = map.layout().bounds.size();
    b.push(
        ObjectKind::MindMap(map),
        Rect::from_center_size((0.0, 0.0), size),
        MindMap::style(),
    );
}

/// The flowchart template's Mermaid source, its labels in `lang`.
fn flowchart(lang: Lang) -> String {
    let tr = |en| tr_in(lang, en);
    format!(
        "flowchart TD\n    A([{}]) --> B[{}]\n    B --> C{{{}}}\n    C -->|{}| D[{}]\n    C -->|{}| E[{}]\n    D --> F([{}])\n    E --> F\n",
        tr("Begin"),
        tr("First step"),
        tr("Decision?"),
        tr("yes"),
        tr("Do this"),
        tr("no"),
        tr("Do that"),
        tr("End"),
    )
}

/// `color` mixed halfway with white, for areas behind notes of that color.
fn lighter(color: Rgba) -> Rgba {
    let [r, g, b, _] = color.to_bytes();
    let mix = |c: u8| ((u16::from(c) + 255) / 2) as u8;
    Rgba::rgb(mix(r), mix(g), mix(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_fits_in_its_frame() {
        fits_in_its_frame(Lang::En);
    }

    /// Russian texts are longer; they still fit where they go.
    #[test]
    fn every_template_fits_in_russian_too() {
        fits_in_its_frame(Lang::Ru);
    }

    #[test]
    fn russian_templates_put_russian_text_on_the_board() {
        let ru = flowchart(Lang::Ru);
        assert!(ru.contains("Начало") && pb_io::mermaid::parse(&ru).is_ok());
        let doc = Document::new();
        let mut engine = TextEngine::new();
        let mut b = Builder::new(&doc, &mut engine).in_lang(Lang::Ru);
        swot(&mut b);
        let items = b.finish(pb_geom::kurbo::Point::ZERO);
        let shapes: Vec<ObjectData> = items.iter().flat_map(Content::shapes).collect();
        assert!(matches!(&shapes[0].kind, ObjectKind::Frame(f) if &*f.title == "SWOT-анализ"));
    }

    fn fits_in_its_frame(lang: Lang) {
        let doc = Document::new();
        let mut engine = TextEngine::new();
        for template in &TEMPLATES {
            let Make::Objects(make) = template.make else {
                assert!(
                    matches!(template.make, Make::Mermaid(src) if pb_io::mermaid::parse(&src(lang)).is_ok())
                );
                continue;
            };
            let mut b = Builder::new(&doc, &mut engine).in_lang(lang);
            make(&mut b);
            let items = b.finish(pb_geom::kurbo::Point::new(500.0, 500.0));
            assert!(!items.is_empty(), "{}", template.name);
            let shapes: Vec<ObjectData> = items.iter().flat_map(Content::shapes).collect();
            let frames: Vec<_> = shapes.iter().filter(|d| d.kind.is_frame()).collect();
            // Frames come first, and everything else sits inside the frame.
            if let Some(frame) = frames.first() {
                assert!(shapes[0].kind.is_frame(), "{}", template.name);
                for d in &shapes[1..] {
                    assert!(
                        frame.bounds.inflate(1.0, 1.0).contains_rect(d.bounds),
                        "{}: {:?} outside {:?}",
                        template.name,
                        d.bounds,
                        frame.bounds
                    );
                }
                // The title fits over the frame.
                let ObjectKind::Frame(data) = &frame.kind else {
                    unreachable!()
                };
                let title = engine.layout(&data.title, 16.0, None);
                assert!(title.width < frame.bounds.width(), "{}", data.title);
            }
            // Texts fit on one line where they go: area titles, table cells.
            for d in &shapes {
                match &d.kind {
                    ObjectKind::Shape(shape) => {
                        let size = d.style.font_size.unwrap_or(16.0);
                        let line = engine.layout(&shape.text, size, None);
                        assert!(line.width < d.bounds.width() - 40.0, "{}", shape.text);
                    }
                    ObjectKind::Table(table) => {
                        for r in 0..table.rows.len() {
                            for c in 0..table.cols.len() {
                                let Some(cell) = table.cell(r, c) else {
                                    continue;
                                };
                                let rect = table.cell_rect(d.bounds, r, c).unwrap();
                                let line = engine.layout(&cell.text, Table::FONT_SIZE, None);
                                assert!(line.width < rect.width() - 12.0, "{}", cell.text);
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Centered where asked.
            let all = shapes
                .iter()
                .map(|d| d.bounds)
                .reduce(|a, b| a.union(b))
                .unwrap();
            assert!((all.center() - pb_geom::kurbo::Point::new(500.0, 500.0)).hypot() < 1e-6);
        }
    }
}
