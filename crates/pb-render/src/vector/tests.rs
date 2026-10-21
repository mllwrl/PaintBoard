use std::collections::HashMap;

use pb_core::{
    Cap, Command, Connector, Document, End, Image, Ink, Label, ObjectData, ObjectKind, Rgba, Route,
    Shape, ShapeKind, Sticky, Style, Text,
};
use vello::kurbo::{Point, Rect, Size};

use super::*;
use crate::{GridStyle, Overlay, Theme, View, draw_vector};

/// Where the test drawing's area starts, in world units.
const ORIGIN: Point = Point::new(-20.0, -20.0);
const SIZE: Size = Size::new(520.0, 420.0);

fn object(kind: ObjectKind, bounds: Rect, style: Style) -> ObjectData {
    ObjectData {
        kind,
        bounds,
        rotation: 0.0,
        style,
        tags: Default::default(),
        link: None,
    }
}

/// A shape with text, a sticky note, a text, a connector, an ink stroke, an image, a
/// half see-through ellipse, and a hidden shape.
fn board() -> Document {
    let mut doc = Document::new();
    let png = pb_io::image::encode_png(&pb_io::image::Pixels {
        width: 2,
        height: 2,
        rgba: [0xE0, 0x10, 0x10, 0xFF].repeat(4),
    })
    .unwrap();
    let asset = doc.add_asset(png);
    let shape = ObjectKind::Shape(Shape {
        kind: ShapeKind::Rectangle,
        text: "Hello".into(),
        spans: Default::default(),
    });
    let blue = Style {
        fill: Rgba::rgb(0xB3, 0xE0, 0xFF),
        stroke: Rgba::rgb(0x1F, 0x29, 0x37),
        stroke_width: 2.0,
        ..Style::default()
    };
    let sticky = ObjectKind::Sticky(Sticky {
        text: "Note".into(),
        spans: Default::default(),
    });
    let yellow = Style {
        fill: Rgba::rgb(0xFF, 0xF1, 0x76),
        ..Style::default()
    };
    let text = ObjectKind::Text(Text {
        text: "Plain words that wrap onto lines".into(),
        spans: Default::default(),
        size: 16.0,
    });
    let connector = ObjectKind::Connector(Connector {
        bends: Default::default(),
        start: End::Free(Point::new(210.0, 150.0)),
        end: End::Free(Point::new(400.0, 300.0)),
        route: Route::Straight,
        start_cap: Cap::None,
        end_cap: Cap::Arrow,
        label: Label::default(),
    });
    let points = [(110.0, 330.0), (150.0, 360.0), (190.0, 330.0)].map(|(x, y)| {
        let p = Point::new(x, y);
        (p, 1.0)
    });
    let (ink, ink_bounds) = Ink::from_world(&points).unwrap();
    let red = Style {
        stroke: Rgba::rgb(0xD0, 0x20, 0x20),
        stroke_width: 4.0,
        ..Style::default()
    };
    let faded = Style {
        fill: Rgba::rgb(0x00, 0x00, 0xFF),
        stroke_width: 0.0,
        opacity: 0.5,
        ..Style::default()
    };
    let items = vec![
        object(shape, Rect::new(0.0, 0.0, 200.0, 100.0), blue),
        object(sticky, Rect::new(250.0, 0.0, 450.0, 200.0), yellow),
        object(text, Rect::new(0.0, 120.0, 150.0, 180.0), Style::default()),
        object(connector, Rect::ZERO, Style::default()),
        object(ObjectKind::Ink(ink), ink_bounds, red),
        object(
            ObjectKind::Image(Image::new(asset)),
            Rect::new(0.0, 200.0, 100.0, 300.0),
            Style::default(),
        ),
        object(
            ShapeKind::Ellipse.into(),
            Rect::new(400.0, 320.0, 480.0, 380.0),
            faded,
        ),
        object(
            ShapeKind::Rectangle.into(),
            Rect::new(200.0, 330.0, 260.0, 380.0),
            Style {
                fill: Rgba::rgb(0x00, 0xA0, 0x00),
                ..Style::default()
            },
        ),
    ];
    let created = doc
        .apply(Command::Create(items.into_iter().map(Into::into).collect()))
        .unwrap()
        .created;
    let hidden = *created.last().unwrap();
    doc.apply(Command::SetHidden {
        ids: vec![hidden],
        hidden: true,
    })
    .unwrap();
    doc
}

fn record(doc: &Document) -> Drawing {
    let camera = pb_geom::Camera::new(ORIGIN, 1.0);
    let theme = Theme::light();
    let (overlay, filter, votes) = (
        Overlay::default(),
        pb_core::Filter::default(),
        HashMap::new(),
    );
    let view = View {
        camera: &camera,
        viewport: SIZE,
        scale: 1.0,
        grid: GridStyle::None,
        theme: &theme,
        document: doc,
        assets: doc.assets(),
        overrides: &[],
        hidden: &[],
        previews: &[],
        overlay: &overlay,
        filter: &filter,
        me: "me",
        votes: &votes,
        plain: true,
        only: None,
        frame_titles: true,
    };
    draw_vector(
        &view,
        &mut crate::TextRenderer::new(),
        &mut crate::ImageCache::default(),
    )
}

/// Checks that every tag is closed in order and attribute quotes pair up; the names of
/// the elements, in order.
fn well_formed(xml: &str) -> Vec<String> {
    let mut open: Vec<String> = Vec::new();
    let mut names = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>').expect("tag not closed") + start;
        let tag = &rest[start + 1..end];
        rest = &rest[end + 1..];
        assert_eq!(tag.matches('"').count() % 2, 0, "quotes in <{tag}>");
        if tag.starts_with('?') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            assert_eq!(
                open.pop().as_deref(),
                Some(name),
                "</{name}> closes nothing"
            );
            continue;
        }
        let name: String = tag
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '/')
            .collect();
        names.push(name.clone());
        if !tag.ends_with('/') {
            open.push(name);
        }
    }
    assert!(open.is_empty(), "left open: {open:?}");
    assert!(rest.trim().is_empty());
    names
}

/// The color of the pixel showing world point `(x, y)` in a picture of the drawing area
/// `scale` pixels per unit.
fn pixel(pixels: &pb_io::image::Pixels, scale: f64, (x, y): (f64, f64)) -> [u8; 4] {
    let px = ((x - ORIGIN.x) * scale) as usize;
    let py = ((y - ORIGIN.y) * scale) as usize;
    let at = (py * pixels.width as usize + px) * 4;
    pixels.rgba[at..at + 4].try_into().unwrap()
}

fn near(color: [u8; 4], expected: [u8; 3]) -> bool {
    (0..3).all(|i| color[i].abs_diff(expected[i]) <= 24)
}

#[test]
fn board_becomes_a_well_formed_svg() {
    let doc = board();
    let svg = svg(&record(&doc), SIZE, Some(Color::WHITE));
    let names = well_formed(&svg);
    assert_eq!(names[0], "svg");
    assert!(svg.contains("width=\"520\" height=\"420\" viewBox=\"0 0 520 420\""));
    // The shape: its fill and outline, moved into the drawing's area.
    assert!(svg.contains("fill=\"#b3e0ff\""));
    assert!(svg.contains("stroke=\"#1f2937\" stroke-width=\"2\""));
    assert!(svg.contains("transform=\"translate(20 20)\""));
    // The note and its shadow.
    assert!(svg.contains("fill=\"#fff176\""));
    assert!(svg.contains("<feGaussianBlur"));
    // Text as glyphs: outlines defined once, then placed.
    assert!(names.iter().filter(|n| *n == "use").count() >= 20);
    assert!(svg.contains("<path id=\"g0\""));
    // The ink stroke and the connector with its arrowhead.
    assert!(svg.contains("stroke=\"#d02020\" stroke-width=\"4\" stroke-linecap=\"round\""));
    assert!(svg.contains("M210 150"));
    // The picture, embedded as it was stored.
    assert!(svg.contains("xlink:href=\"data:image/png;base64,iVBORw0KGgo"));
    // The see-through ellipse as a group; the hidden shape is left out.
    assert!(svg.contains("opacity=\"0.5\""));
    assert!(!svg.contains("#00a000"));
}

#[test]
fn svg_draws_like_the_board() {
    let doc = board();
    let svg = svg(&record(&doc), SIZE, Some(Color::WHITE));
    let drawn = pb_io::svg::draw(svg.as_bytes(), None).unwrap();
    let pixels = pb_io::image::decode(&drawn.png, 8192).unwrap();
    let scale = f64::from(drawn.pixels.0) / SIZE.width;
    assert!(near(
        pixel(&pixels, scale, (20.0, 80.0)),
        [0xB3, 0xE0, 0xFF]
    ));
    assert!(near(
        pixel(&pixels, scale, (440.0, 190.0)),
        [0xFF, 0xF1, 0x76]
    ));
    assert!(near(
        pixel(&pixels, scale, (50.0, 250.0)),
        [0xE0, 0x10, 0x10]
    ));
    assert!(near(
        pixel(&pixels, scale, (440.0, 350.0)),
        [0x80, 0x80, 0xFF]
    ));
    assert!(near(
        pixel(&pixels, scale, (230.0, 355.0)),
        [0xFF, 0xFF, 0xFF]
    ));
    assert!(near(
        pixel(&pixels, scale, (490.0, 10.0)),
        [0xFF, 0xFF, 0xFF]
    ));
}

#[test]
fn pdf_draws_like_the_board() {
    let doc = board();
    let page = |drawing| Page {
        drawing,
        size: SIZE,
    };
    let bytes = pdf(&[page(record(&doc)), page(record(&doc))]);
    assert_eq!(pb_io::pdf::page_count(&bytes).unwrap(), 2);
    let pages = pb_io::pdf::render_pages(&bytes, 1).unwrap();
    let pixels = pb_io::image::decode(&pages[0].png, 8192).unwrap();
    let scale = f64::from(pages[0].width) / SIZE.width;
    assert!((scale - 1.5).abs() < 0.01, "{scale}");
    assert!(near(
        pixel(&pixels, scale, (20.0, 80.0)),
        [0xB3, 0xE0, 0xFF]
    ));
    assert!(near(
        pixel(&pixels, scale, (440.0, 190.0)),
        [0xFF, 0xF1, 0x76]
    ));
    assert!(near(
        pixel(&pixels, scale, (50.0, 250.0)),
        [0xE0, 0x10, 0x10]
    ));
    assert!(near(
        pixel(&pixels, scale, (440.0, 350.0)),
        [0x80, 0x80, 0xFF]
    ));
    assert!(near(
        pixel(&pixels, scale, (230.0, 355.0)),
        [0xFF, 0xFF, 0xFF]
    ));
}

#[test]
fn links_in_text_stay_clickable_in_files() {
    let mut doc = Document::new();
    let spans = pb_core::Spans::new(
        vec![],
        vec![pb_core::LinkSpan {
            start: 0,
            end: 11,
            marks: pb_core::Link::from_input("example.com/a?b=(c)"),
        }],
        vec![pb_core::Para {
            index: 1,
            style: pb_core::ParaStyle::list(pb_core::ListKind::Bullet, 0),
        }],
    );
    let text = ObjectKind::Text(Text {
        text: "example.com\nan item".into(),
        spans,
        size: 16.0,
    });
    let data = object(text, Rect::new(0.0, 0.0, 200.0, 60.0), Style::default());
    doc.apply(Command::Create(vec![data.into()])).unwrap();
    let drawing = record(&doc);
    let svg = svg(&drawing, SIZE, None);
    well_formed(&svg);
    assert!(
        svg.contains("<a href=\"https://example.com/a?b=(c)\""),
        "{svg}"
    );
    let bytes = pdf(&[Page {
        drawing,
        size: SIZE,
    }]);
    assert_eq!(pb_io::pdf::page_count(&bytes).unwrap(), 1);
    let file = String::from_utf8_lossy(&bytes);
    assert!(file.contains("/Subtype /Link"));
    assert!(file.contains("/URI (https://example.com/a?b=\\(c\\))"));
    assert!(file.contains("/Annots ["));
}

/// Writes the test board as SVG, PDF and pictures of both to `PB_VECTOR_OUT`, to look at.
#[test]
#[ignore = "writes files, run by hand"]
fn write_test_board() {
    let dir = std::path::PathBuf::from(std::env::var("PB_VECTOR_OUT").unwrap());
    let doc = board();
    let svg = svg(&record(&doc), SIZE, Some(Color::WHITE));
    std::fs::write(dir.join("board.svg"), &svg).unwrap();
    std::fs::write(
        dir.join("board-svg.png"),
        pb_io::svg::draw(svg.as_bytes(), None).unwrap().png,
    )
    .unwrap();
    let pdf = pdf(&[Page {
        drawing: record(&doc),
        size: SIZE,
    }]);
    std::fs::write(dir.join("board.pdf"), &pdf).unwrap();
    let pages = pb_io::pdf::render_pages(&pdf, 1).unwrap();
    std::fs::write(dir.join("board-pdf.png"), &pages[0].png).unwrap();
}

#[test]
fn base64_pads() {
    assert_eq!(base64(b"Man"), "TWFu");
    assert_eq!(base64(b"Ma"), "TWE=");
    assert_eq!(base64(b"M"), "TQ==");
    assert_eq!(base64(b""), "");
}

#[test]
fn numbers_are_short() {
    assert_eq!(number(1.0, 3), "1");
    assert_eq!(number(1.25, 3), "1.25");
    assert_eq!(number(-0.0001, 3), "0");
    assert_eq!(number(f64::NAN, 3), "0");
}
