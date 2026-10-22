//! Snapshot tests: sample boards are drawn on the GPU without a window and compared with
//! pictures kept in `tests/snapshots`. A missing picture is written instead (check it in);
//! PB_UPDATE_SNAPSHOTS=1 rewrites them after an intended change of looks. Without a usable
//! GPU the test says so and passes.

use std::collections::HashMap;
use std::path::PathBuf;

use pb_core::{
    Anchor, Cap, Cell, Command, Connector, Document, End, FrameData, Label, ObjectData, ObjectKind,
    Rgba, Route, Shape, ShapeKind, Span, Sticky, Style, Table, Text, TextMarks,
};
use pb_geom::Camera;
use pb_geom::kurbo::{Point, Rect, Size};
use pb_render::vello::Scene;
use pb_render::{
    GridStyle, ImageCache, Offscreen, Overlay, TextRenderer, Theme, View, build_scene,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;

/// A pixel counts as changed when a channel differs by more than this...
const CHANNEL_TOLERANCE: u8 = 32;
/// ...and a picture as changed when more than this share of its pixels did (edges are
/// smoothed slightly differently by different GPUs and drivers).
const PIXEL_TOLERANCE: f64 = 0.005;

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

fn shape(kind: ShapeKind, text: &str, bounds: Rect, style: Style) -> ObjectData {
    object(
        ObjectKind::Shape(Shape {
            kind,
            text: text.into(),
            spans: Default::default(),
        }),
        bounds,
        style,
    )
}

fn filled(r: u8, g: u8, b: u8) -> Style {
    Style {
        fill: Rgba::rgb(r, g, b),
        ..Style::default()
    }
}

fn board(items: Vec<ObjectData>) -> Document {
    let mut doc = Document::new();
    doc.apply(Command::Create(items.into_iter().map(Into::into).collect()))
        .unwrap();
    doc
}

fn shapes() -> Document {
    let mut turned = shape(
        ShapeKind::Rectangle,
        "",
        Rect::new(330.0, 40.0, 430.0, 100.0),
        filled(0xFF, 0xD0, 0x80),
    );
    turned.rotation = 0.4;
    board(vec![
        shape(
            ShapeKind::Rectangle,
            "Box",
            Rect::new(20.0, 20.0, 160.0, 100.0),
            filled(0xC8, 0xE6, 0xFF),
        ),
        shape(
            ShapeKind::Ellipse,
            "",
            Rect::new(190.0, 20.0, 300.0, 120.0),
            Style::shape(),
        ),
        shape(
            ShapeKind::Star,
            "",
            Rect::new(20.0, 140.0, 140.0, 260.0),
            filled(0xFF, 0xE0, 0x66),
        ),
        shape(
            ShapeKind::Diamond,
            "Decision",
            Rect::new(170.0, 150.0, 330.0, 270.0),
            Style::shape(),
        ),
        turned,
    ])
}

fn notes_and_text() -> Document {
    let bold = TextMarks {
        bold: true,
        ..TextMarks::default()
    };
    let spans = pb_core::Spans::from_marks(vec![Span {
        start: 0,
        end: 4,
        marks: bold,
    }]);
    board(vec![
        object(
            ObjectKind::Frame(FrameData {
                title: "Ideas".into(),
            }),
            Rect::new(10.0, 30.0, 470.0, 310.0),
            Style::default(),
        ),
        object(
            ObjectKind::Sticky(Sticky {
                text: "Ship the first version".into(),
                spans: Default::default(),
            }),
            Rect::new(30.0, 50.0, 190.0, 210.0),
            filled(0xFF, 0xF1, 0x76),
        ),
        object(
            ObjectKind::Text(Text {
                text: "Bold and plain, кириллица".into(),
                spans,
                size: 18.0,
            }),
            Rect::new(220.0, 70.0, 460.0, 100.0),
            Style::default(),
        ),
    ])
}

fn connectors() -> Document {
    let mut doc = board(vec![
        shape(
            ShapeKind::Rectangle,
            "Start",
            Rect::new(20.0, 20.0, 140.0, 80.0),
            Style::shape(),
        ),
        shape(
            ShapeKind::Rectangle,
            "End",
            Rect::new(300.0, 220.0, 440.0, 290.0),
            Style::shape(),
        ),
    ]);
    let ids: Vec<_> = doc.objects().iter().map(|o| o.id).collect();
    let end = |object| End::Attached {
        object,
        anchor: Anchor::Auto,
        at: Point::ZERO,
    };
    let line = |route, label: &str| {
        object(
            ObjectKind::Connector(Connector {
                start: end(ids[0]),
                end: end(ids[1]),
                route,
                start_cap: Cap::None,
                end_cap: Cap::Arrow,
                label: Label {
                    text: label.into(),
                    at: 0.5,
                    size: Size::new(30.0, 20.0),
                },
                bends: Default::default(),
            }),
            Rect::ZERO,
            Style::default(),
        )
    };
    doc.apply(Command::Create(vec![
        line(Route::Elbow, "yes").into(),
        line(Route::Straight, "").into(),
    ]))
    .unwrap();
    doc
}

fn table() -> Document {
    let mut doc = Document::new();
    let mut table = Table::new(3, 2, || doc.new_line_id());
    for (r, row) in [["Name", "Role"], ["Ann", "Design"], ["Bob", "Code"]]
        .iter()
        .enumerate()
    {
        for (c, text) in row.iter().enumerate() {
            let cell = Cell {
                text: (*text).into(),
                spans: Default::default(),
            };
            table = table.with_cell(r, c, cell);
        }
    }
    doc.apply(Command::Create(vec![
        object(
            ObjectKind::Table(table),
            Rect::new(40.0, 40.0, 400.0, 200.0),
            Table::style(),
        )
        .into(),
    ]))
    .unwrap();
    doc
}

/// Draws `doc` as the app shows it at 100%, without grid or selection.
fn draw(gpu: &mut Offscreen, doc: &Document, theme: &Theme) -> Vec<u8> {
    let camera = Camera::new(Point::ZERO, 1.0);
    let (overlay, filter, votes) = (
        Overlay::default(),
        pb_core::Filter::default(),
        HashMap::new(),
    );
    let view = View {
        camera: &camera,
        viewport: Size::new(f64::from(WIDTH), f64::from(HEIGHT)),
        scale: 1.0,
        grid: GridStyle::None,
        theme,
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
    let mut scene = Scene::new();
    build_scene(
        &mut scene,
        &view,
        &mut TextRenderer::new(),
        &mut ImageCache::default(),
    );
    gpu.render_image(&scene, WIDTH, HEIGHT, theme.background)
        .expect("drawing into an image works on a GPU that was found")
}

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

/// Compares `rgba` with the kept picture `name`, writing it when missing or asked to.
fn check(name: &str, rgba: Vec<u8>) -> Result<(), String> {
    let pixels = pb_io::image::Pixels {
        width: WIDTH,
        height: HEIGHT,
        rgba,
    };
    let path = snapshot_dir().join(format!("{name}.png"));
    let update = std::env::var_os("PB_UPDATE_SNAPSHOTS").is_some();
    if update || !path.exists() {
        std::fs::create_dir_all(snapshot_dir()).unwrap();
        std::fs::write(&path, pb_io::image::encode_png(&pixels).unwrap()).unwrap();
        eprintln!("wrote {}", path.display());
        return Ok(());
    }
    let kept = pb_io::image::decode(&std::fs::read(&path).unwrap(), 4096).unwrap();
    if (kept.width, kept.height) != (WIDTH, HEIGHT) {
        return Err(format!(
            "{name}: kept picture is {}x{}",
            kept.width, kept.height
        ));
    }
    let changed = kept
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(pixels.rgba.as_chunks::<4>().0)
        .filter(|(a, b)| {
            a.iter()
                .zip(b.iter())
                .any(|(x, y)| x.abs_diff(*y) > CHANNEL_TOLERANCE)
        })
        .count();
    let share = changed as f64 / f64::from(WIDTH * HEIGHT);
    if share > PIXEL_TOLERANCE {
        // Next to the kept one, for a look (ignored by git).
        let actual = snapshot_dir().join(format!("{name}.actual.png"));
        std::fs::write(&actual, pb_io::image::encode_png(&pixels).unwrap()).unwrap();
        return Err(format!(
            "{name}: {:.2}% of pixels changed, see {}",
            share * 100.0,
            actual.display()
        ));
    }
    Ok(())
}

/// The GPU to draw with, or `None` to skip. CI runners have no real GPU: Linux has none at
/// all, and the software adapters on the Windows and macOS runners crash or draw a little
/// differently, so on CI these tests run only with `PB_GPU_TESTS=1`.
fn gpu() -> Option<Offscreen> {
    if std::env::var_os("CI").is_some() && std::env::var_os("PB_GPU_TESTS").is_none() {
        eprintln!("on CI: snapshot tests skipped (PB_GPU_TESTS=1 runs them)");
        return None;
    }
    let gpu = pollster::block_on(Offscreen::new());
    if gpu.is_none() {
        eprintln!("no GPU found: snapshot tests skipped");
    }
    gpu
}

#[test]
fn sample_boards_look_as_kept() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let (light, dark) = (Theme::light(), Theme::dark());
    let cases: [(&str, Document, &Theme); 5] = [
        ("shapes", shapes(), &light),
        ("notes_and_text", notes_and_text(), &light),
        ("connectors", connectors(), &light),
        ("table", table(), &light),
        ("dark", notes_and_text(), &dark),
    ];
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(name, doc, theme)| check(name, draw(&mut gpu, doc, theme)).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// vello fits all images drawn in a frame into one atlas of at most 8192×8192 pixels; nine
/// big photos at full size would not fit, and some would silently be missing.
#[test]
fn many_big_photos_all_show() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let mut doc = Document::new();
    let colors: Vec<[u8; 3]> = (0..9u8)
        .map(|i| [40 + i * 20, 200 - i * 20, if i % 2 == 0 { 30 } else { 220 }])
        .collect();
    let (w, h) = (4000u32, 3000u32);
    let mut items = Vec::new();
    for (i, c) in colors.iter().enumerate() {
        let rgba = [c[0], c[1], c[2], 255].repeat((w * h) as usize);
        let png = pb_io::image::encode_png(&pb_io::image::Pixels {
            width: w,
            height: h,
            rgba,
        })
        .unwrap();
        let asset = doc.add_asset(png);
        let (col, row) = ((i % 3) as f64, (i / 3) as f64);
        let bounds = Rect::new(
            col * 160.0,
            row * 106.0,
            col * 160.0 + 150.0,
            row * 106.0 + 100.0,
        );
        items.push(object(
            ObjectKind::Image(pb_core::Image::new(asset)),
            bounds,
            Style::default(),
        ));
    }
    doc.apply(Command::Create(items.into_iter().map(Into::into).collect()))
        .unwrap();
    let rgba = draw(&mut gpu, &doc, &Theme::light());
    for (i, c) in colors.iter().enumerate() {
        let (x, y) = ((i % 3) * 160 + 75, (i / 3) * 106 + 50);
        let at = (y * WIDTH as usize + x) * 4;
        assert_eq!(&rgba[at..at + 3], c, "image {i} is missing");
    }
}

/// vello drops the contents of its image atlas when a frame has no images (or gradients)
/// in it, yet thinks the images it uploaded before are still there: a picture that left
/// the screen came back blank.
#[test]
fn a_picture_shows_again_after_leaving_the_screen() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let mut doc = Document::new();
    let (w, h) = (300u32, 200u32);
    let rgba = [200u8, 40, 40, 255].repeat((w * h) as usize);
    let png = pb_io::image::encode_png(&pb_io::image::Pixels {
        width: w,
        height: h,
        rgba,
    })
    .unwrap();
    let asset = doc.add_asset(png);
    let picture = object(
        ObjectKind::Image(pb_core::Image::new(asset)),
        Rect::new(-150.0, -100.0, 150.0, 100.0),
        Style::default(),
    );
    doc.apply(Command::Create(vec![picture.into()])).unwrap();
    let theme = Theme::light();
    let (overlay, filter, votes) = (
        Overlay::default(),
        pb_core::Filter::default(),
        HashMap::new(),
    );
    let mut images = ImageCache::default();
    let mut text = TextRenderer::new();
    // The camera's origin is the view's top left corner.
    let mut frame = |origin: Point| {
        let camera = Camera::new(origin, 1.0);
        let view = View {
            camera: &camera,
            viewport: Size::new(f64::from(WIDTH), f64::from(HEIGHT)),
            scale: 1.0,
            grid: GridStyle::None,
            theme: &theme,
            document: &doc,
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
        let mut scene = Scene::new();
        build_scene(&mut scene, &view, &mut text, &mut images);
        gpu.render_image(&scene, WIDTH, HEIGHT, theme.background)
            .unwrap()
    };
    let middle = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
    let home = Point::new(-f64::from(WIDTH) / 2.0, -f64::from(HEIGHT) / 2.0);
    let shown = frame(home);
    assert_eq!(&shown[middle..middle + 3], &[200, 40, 40]);
    // Away: nothing with an image on screen.
    frame(Point::new(1e5, 1e5));
    frame(Point::new(1e5, 1e5));
    let back = frame(home);
    assert_eq!(
        &back[middle..middle + 3],
        &[200, 40, 40],
        "the picture is blank"
    );
}
