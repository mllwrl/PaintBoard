use pb_core::connector::ConnectorPath;
use pb_core::{
    Assets, FrameData, HAlign, Image, Label, ObjectData, ObjectKind, Rgba, Span, Spans, Style,
    TextBox,
};
use std::collections::HashMap;
use std::sync::Arc;

use pb_geom::ink::{smooth_path, variable_outline};
use pb_text::{TextAlign, TextEngine, TextLayout};
use vello::Scene;
use vello::kurbo::{Affine, BezPath, Cap, Join, Line, Rect, RoundedRect, Shape, Stroke};
use vello::peniko::{Color, Fill, Mix};

/// Ink never gets thinner than this fraction of its width, however light the pressure.
const MIN_PRESSURE: f64 = 0.15;

/// Sticky note shadow: offset down and blur, relative to the note's smaller side.
const SHADOW_OFFSET: f64 = 0.015;
const SHADOW_BLUR: f64 = 0.025;
const SHADOW_COLOR: Color = Color::from_rgba8(0, 0, 0, 0x30);
/// Drawn in place of an image whose file is missing or cannot be read.
const PLACEHOLDER_FILL: Color = Color::from_rgb8(0xE4, 0xE4, 0xE0);
const PLACEHOLDER_LINE: Color = Color::from_rgb8(0xA0, 0xA0, 0x9A);
/// A frame's hairline edge and its title.
const FRAME_EDGE: Color = Color::from_rgb8(0xD0, 0xD0, 0xCB);
const FRAME_TITLE: Color = Color::from_rgb8(0x5F, 0x63, 0x68);

/// Kanban cards and the label of the row that adds them.
const CARD_FILL: Color = Color::from_rgb8(0xFF, 0xFF, 0xFF);
const ADD_LABEL: Color = Color::from_rgb8(0x6B, 0x72, 0x80);

/// Fitted sizes kept by [`TextRenderer`]; the cache is dropped when it grows past this.
const FIT_CACHE_LIMIT: usize = 4096;

/// Font size of text with `spans` fitted into a sticky note or shape: as large as fits
/// in its box.
pub fn fitted_font_size(
    engine: &mut TextEngine,
    text: &str,
    spans: &[Span],
    text_box: TextBox,
) -> f64 {
    let area = text_box.area;
    engine.fit_size(text, spans, area.width(), area.height(), text_box.max_size)
}

/// How lines aligned with `align` are laid out.
pub fn text_align(align: HAlign) -> TextAlign {
    match align {
        HAlign::Left => TextAlign::Start,
        HAlign::Center => TextAlign::Center,
        HAlign::Right => TextAlign::End,
    }
}

/// Lays out and draws board text with the app font.
///
/// TODO: cache layouts per object; laying out every text each frame will not scale.
pub struct TextRenderer {
    engine: TextEngine,
    /// Font sizes of fitted text by text, styles and box, as fitting takes several layouts.
    fitted: HashMap<(Arc<str>, Spans, [u64; 3]), f64>,
}

impl TextRenderer {
    pub fn new() -> Self {
        Self {
            engine: TextEngine::new(),
            fitted: HashMap::new(),
        }
    }

    fn fitted_font_size(&mut self, text: &Arc<str>, spans: &Spans, text_box: TextBox) -> f64 {
        let area = text_box.area;
        let key = (
            text.clone(),
            spans.clone(),
            [area.width(), area.height(), text_box.max_size].map(f64::to_bits),
        );
        if let Some(&size) = self.fitted.get(&key) {
            return size;
        }
        if self.fitted.len() >= FIT_CACHE_LIMIT {
            self.fitted.clear();
        }
        let size = fitted_font_size(&mut self.engine, text, spans, text_box);
        self.fitted.insert(key, size);
        size
    }

    /// Draws `text` with `spans` fitted into `text_box` and aligned in it.
    fn draw_fitted(
        &mut self,
        scene: &mut Scene,
        transform: Affine,
        (text, spans): (&Arc<str>, &Spans),
        text_box: TextBox,
        brush: Color,
    ) {
        if text.is_empty() {
            return;
        }
        let size = self.fitted_font_size(text, spans, text_box);
        let area = text_box.area;
        let align = text_align(text_box.align);
        let layout = self
            .engine
            .layout_aligned(text, spans, size, Some(area.width()), align);
        let top = text_box.top(layout.height);
        draw_text(scene, transform, (area.x0, top), &layout, brush);
    }

    /// For measuring text before creating objects.
    pub fn engine(&mut self) -> &mut TextEngine {
        &mut self.engine
    }
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Draws a laid out text, its top-left corner at `origin` in `transform`. Runs without a
/// color of their own take `brush`.
fn draw_text(
    scene: &mut Scene,
    transform: Affine,
    origin: (f64, f64),
    layout: &TextLayout,
    brush: Color,
) {
    let transform = transform * Affine::translate(origin);
    let paint = |b: pb_text::Brush| b.map_or(brush, color);
    for run in &layout.runs {
        let glyphs = run.glyphs.iter().map(|g| vello::Glyph {
            id: g.id,
            x: g.x,
            y: g.y,
        });
        scene
            .draw_glyphs(&run.font)
            .font_size(run.size)
            .transform(transform)
            .brush(paint(run.brush))
            .draw(Fill::NonZero, glyphs);
    }
    for d in &layout.decorations {
        let half = f64::from(d.thickness) / 2.0;
        let bar = Rect::new(
            f64::from(d.x0),
            f64::from(d.y) - half,
            f64::from(d.x1),
            f64::from(d.y) + half,
        );
        scene.fill(Fill::NonZero, transform, paint(d.brush), None, &bar);
    }
}

/// What drawing objects needs besides the scene.
pub(crate) struct Resources<'a> {
    pub text: &'a mut TextRenderer,
    pub images: &'a mut crate::ImageCache,
    pub assets: &'a Assets,
    /// Logical pixels per world unit, for what keeps its size on screen.
    pub zoom: f64,
}

/// Draws one object. `transform` maps world to physical pixels.
pub(crate) fn draw_object(
    scene: &mut Scene,
    transform: Affine,
    data: &ObjectData,
    resources: Resources<'_>,
) {
    let Resources {
        text,
        images,
        assets,
        zoom,
    } = resources;
    let transform = transform * data.frame().transform();
    let style = &data.style;
    if style.opacity <= 0.0 {
        return;
    }
    // Opacity applies to the object as a whole, so its own overlaps do not show.
    let layered = style.opacity < 1.0;
    if layered {
        let pad = paint_padding(data);
        let mut clip = data.bounds.inflate(pad, pad);
        if data.kind.is_frame() {
            clip = clip.union(FrameData::title_band(data.bounds, zoom));
        }
        scene.push_layer(
            Fill::NonZero,
            Mix::Normal,
            style.opacity as f32,
            transform,
            &clip,
        );
    }
    match &data.kind {
        ObjectKind::Shape(shape) => {
            let path = shape.kind.outline(data.bounds);
            scene.fill(Fill::NonZero, transform, color(style.fill), None, &path);
            if style.stroke_width > 0.0 {
                scene.stroke(
                    &Stroke::new(style.stroke_width).with_join(Join::Round),
                    transform,
                    color(style.stroke),
                    None,
                    &path,
                );
            }
            if let Some(text_box) = data.text_box() {
                let brush = color(style.text_color());
                text.draw_fitted(
                    scene,
                    transform,
                    (&shape.text, &shape.spans),
                    text_box,
                    brush,
                );
            }
        }
        ObjectKind::Text(content) => {
            let b = data.bounds;
            // A hair of slack, so text measured to fit exactly does not wrap.
            let layout = text.engine.layout_aligned(
                &content.text,
                &content.spans,
                content.size,
                Some(b.width() + 0.5),
                TextAlign::Start,
            );
            let top = b.y0 + (b.height() - layout.height) / 2.0;
            draw_text(scene, transform, (b.x0, top), &layout, color(style.stroke));
        }
        ObjectKind::Sticky(sticky) => {
            let b = data.bounds;
            let side = b.width().min(b.height());
            let shadow = b + vello::kurbo::Vec2::new(0.0, side * SHADOW_OFFSET);
            scene.draw_blurred_rounded_rect(
                transform,
                shadow,
                SHADOW_COLOR,
                0.0,
                side * SHADOW_BLUR,
            );
            scene.fill(Fill::NonZero, transform, color(style.fill), None, &b);
            if let Some(text_box) = data.text_box() {
                text.draw_fitted(
                    scene,
                    transform,
                    (&sticky.text, &sticky.spans),
                    text_box,
                    color(style.stroke),
                );
            }
        }
        // Drawn by `draw_connector`, which needs the objects they attach to.
        ObjectKind::Connector(_) => {}
        ObjectKind::Table(table) => {
            let b = data.bounds;
            scene.fill(Fill::NonZero, transform, color(style.fill), None, &b);
            let brush = color(style.text_color());
            let merged = table.merged();
            // Inside a merged block only its top left cell shows.
            let hidden = |row, col| {
                merged
                    .iter()
                    .any(|m| m.contains((row, col)) && (m.row0, m.col0) != (row, col))
            };
            for row in 0..table.rows.len() {
                for col in 0..table.cols.len() {
                    let Some(cell) = table.cell(row, col) else {
                        continue;
                    };
                    if hidden(row, col) {
                        continue;
                    }
                    if let Some(text_box) = table.text_box(b, row, col, style) {
                        let content = (&cell.text, &cell.spans);
                        text.draw_fitted(scene, transform, content, text_box, brush);
                    }
                }
            }
            if style.stroke_width > 0.0 {
                let mut grid = BezPath::new();
                for (from, to) in table.inner_lines(b) {
                    grid.move_to(from);
                    grid.line_to(to);
                }
                grid.extend(b.path_elements(0.1));
                let stroke = Stroke::new(style.stroke_width);
                scene.stroke(&stroke, transform, color(style.stroke), None, &grid);
            }
        }
        ObjectKind::Kanban(kanban) => {
            let b = data.bounds;
            let radius = kanban.radius(b.width());
            let brush = color(style.text_color());
            let outline = Stroke::new(style.stroke_width);
            for col in 0..kanban.columns.len() {
                let Some(rect) = kanban.column_rect(b, col) else {
                    continue;
                };
                let column = RoundedRect::from_rect(rect, radius);
                scene.fill(Fill::NonZero, transform, color(style.fill), None, &column);
                let title = &kanban.columns[col].title;
                if let Some(text_box) = kanban.title_text_box(b, col) {
                    let spans = pb_core::Kanban::title_spans(title);
                    text.draw_fitted(scene, transform, (title, &spans), text_box, brush);
                }
                for i in kanban.cards_in(col) {
                    let Some(rect) = kanban.card_rect(b, i) else {
                        continue;
                    };
                    let card = RoundedRect::from_rect(rect, radius);
                    scene.fill(Fill::NonZero, transform, CARD_FILL, None, &card);
                    if style.stroke_width > 0.0 {
                        scene.stroke(&outline, transform, color(style.stroke), None, &card);
                    }
                    let c = &kanban.cards[i];
                    if let Some(text_box) = kanban.card_text_box(b, i) {
                        text.draw_fitted(scene, transform, (&c.text, &c.spans), text_box, brush);
                    }
                }
                if let Some(text_box) = kanban.add_text_box(b, col) {
                    let label: Arc<str> = pb_core::Kanban::ADD_LABEL.into();
                    let spans = Spans::default();
                    text.draw_fitted(scene, transform, (&label, &spans), text_box, ADD_LABEL);
                }
            }
        }
        ObjectKind::MindMap(map) => draw_mindmap(scene, transform, data, map, text),
        ObjectKind::Frame(frame) => {
            let b = data.bounds;
            scene.fill(Fill::NonZero, transform, color(style.fill), None, &b);
            scene.stroke(&Stroke::new(1.0 / zoom), transform, FRAME_EDGE, None, &b);
            if !frame.title.is_empty() {
                let size = FrameData::TITLE_SIZE / zoom;
                let layout = text.engine.layout(&frame.title, size, None);
                let band = FrameData::title_band(b, zoom);
                let top = b.y0 - FrameData::TITLE_GAP / zoom - layout.height;
                // A long title is cut at the frame's width.
                scene.push_clip_layer(Fill::NonZero, transform, &band);
                draw_text(scene, transform, (b.x0, top), &layout, FRAME_TITLE);
                scene.pop_layer();
            }
        }
        ObjectKind::Image(image) => match images.get(assets, image.asset) {
            Some(brush) => draw_image(scene, transform, data.bounds, image, brush),
            None => draw_placeholder(scene, transform, data.bounds),
        },
        ObjectKind::Ink(ink) => {
            let points = ink.points_in(data.bounds);
            let ink_color = color(style.stroke);
            if ink.has_pressure() {
                let radii: Vec<_> = points
                    .iter()
                    .map(|&(p, pressure)| {
                        (p, style.stroke_width * pressure.max(MIN_PRESSURE) / 2.0)
                    })
                    .collect();
                let outline = variable_outline(&radii);
                scene.fill(Fill::NonZero, transform, ink_color, None, &outline);
            } else {
                let line: Vec<_> = points.iter().map(|(p, _)| *p).collect();
                let stroke = Stroke::new(style.stroke_width)
                    .with_caps(Cap::Round)
                    .with_join(Join::Round);
                scene.stroke(&stroke, transform, ink_color, None, &smooth_path(&line));
            }
        }
    }
    if layered {
        scene.pop_layer();
    }
}

/// A connector's line and arrowheads in the outline color, and its label's text in a gap
/// in the line; `label` is the label with the box of its text.
pub(crate) fn draw_connector(
    scene: &mut Scene,
    transform: Affine,
    style: &Style,
    path: &ConnectorPath,
    label: Option<(&Label, Rect)>,
    text: &mut TextRenderer,
) {
    if style.opacity <= 0.0 {
        return;
    }
    let pad = style.stroke_width;
    let area = path.bounds().inflate(pad, pad);
    let area = label.map_or(area, |(_, r)| area.union(r)).inflate(1.0, 1.0);
    // Opacity applies to the whole connector, so the line under an arrowhead does not show.
    let layered = style.opacity < 1.0;
    if layered {
        scene.push_layer(
            Fill::NonZero,
            Mix::Normal,
            style.opacity as f32,
            transform,
            &area,
        );
    }
    // The line breaks around the label.
    if let Some((_, r)) = label {
        let gap = Label::PADDING;
        clip_out(scene, transform, area, &[r.inflate(gap, gap)]);
    }
    let brush = color(style.stroke);
    let stroke = Stroke::new(style.stroke_width.max(0.5))
        .with_caps(Cap::Round)
        .with_join(Join::Round);
    scene.stroke(&stroke, transform, brush, None, &path.line);
    for head in &path.heads {
        scene.fill(Fill::NonZero, transform, brush, None, head);
    }
    if let Some((label, r)) = label {
        scene.pop_layer();
        let size = Label::FONT_SIZE;
        let layout = text
            .engine
            .layout_aligned(&label.text, &[], size, None, TextAlign::Center);
        let c = r.center();
        let origin = (c.x - layout.width / 2.0, c.y - layout.height / 2.0);
        draw_text(scene, transform, origin, &layout, brush);
    }
    if layered {
        scene.pop_layer();
    }
}

/// Pushes a clip layer of `area` without `holes` (all in `transform`'s space).
pub(crate) fn clip_out(scene: &mut Scene, transform: Affine, area: Rect, holes: &[Rect]) {
    let mut clip = BezPath::new();
    clip.extend(area.path_elements(0.1));
    // Wound the other way, so they are holes whatever the fill rule; vello's clips do not
    // seem to honor even-odd.
    for hole in holes {
        clip.move_to((hole.x0, hole.y0));
        clip.line_to((hole.x0, hole.y1));
        clip.line_to((hole.x1, hole.y1));
        clip.line_to((hole.x1, hole.y0));
        clip.close_path();
    }
    scene.push_clip_layer(Fill::NonZero, transform, &clip);
}

/// The cropped part of `brush` stretched over `bounds`.
fn draw_image(
    scene: &mut Scene,
    transform: Affine,
    bounds: Rect,
    image: &Image,
    brush: &vello::peniko::ImageBrush,
) {
    let (w, h) = (f64::from(brush.image.width), f64::from(brush.image.height));
    let crop = image.crop;
    if crop.width() <= 0.0 || crop.height() <= 0.0 {
        return;
    }
    // Image pixels -> crop rectangle at the origin -> bounds.
    let brush_transform = Affine::translate(bounds.origin().to_vec2())
        * Affine::scale_non_uniform(
            bounds.width() / (crop.width() * w),
            bounds.height() / (crop.height() * h),
        )
        * Affine::translate((-crop.x0 * w, -crop.y0 * h));
    scene.fill(
        Fill::NonZero,
        transform,
        brush,
        Some(brush_transform),
        &bounds,
    );
}

/// A gray box with a cross, for an image that cannot be shown.
fn draw_placeholder(scene: &mut Scene, transform: Affine, b: Rect) {
    scene.fill(Fill::NonZero, transform, PLACEHOLDER_FILL, None, &b);
    let width = b.width().min(b.height()) * 0.01;
    let stroke = Stroke::new(width.max(0.5));
    scene.stroke(&stroke, transform, PLACEHOLDER_LINE, None, &b);
    for line in [
        Line::new((b.x0, b.y0), (b.x1, b.y1)),
        Line::new((b.x1, b.y0), (b.x0, b.y1)),
    ] {
        scene.stroke(&stroke, transform, PLACEHOLDER_LINE, None, &line);
    }
}

/// World-space axis-aligned area an object paints, including its stroke.
pub fn painted_bounds(data: &ObjectData) -> Rect {
    let pad = paint_padding(data);
    data.frame().aabb().inflate(pad, pad)
}

/// How far an object paints outside its bounds: half its outline, or a sticky's shadow.
/// Draws a mind map over `data`'s bounds; `transform` maps its unrotated coordinates to
/// physical pixels.
fn draw_mindmap(
    scene: &mut Scene,
    transform: Affine,
    data: &ObjectData,
    map: &pb_core::MindMap,
    text: &mut TextRenderer,
) {
    let style = &data.style;
    let b = data.bounds;
    let layout = map.layout();
    let (scale, _) = map.transform(&layout, b);
    let world = |r: Rect| map.to_world(&layout, b, r);
    let branch =
        |p: &pb_core::mindmap::Placed| color(pb_core::MindMap::branch_color(p.branch, style));
    for &(from, to) in &layout.edges {
        let (parent, child) = (&layout.placed[from], &layout.placed[to]);
        let (pr, cr) = (world(parent.rect), world(child.rect));
        let (start, end) = if child.left {
            ((pr.x0, pr.center().y), (cr.x1, cr.center().y))
        } else {
            ((pr.x1, pr.center().y), (cr.x0, cr.center().y))
        };
        let mid = (start.0 + end.0) / 2.0;
        let mut path = BezPath::new();
        path.move_to(start);
        path.curve_to((mid, start.1), (mid, end.1), end);
        let width = if parent.depth == 0 { 3.0 } else { 2.0 } * scale;
        let stroke = Stroke::new(width).with_caps(Cap::Round);
        scene.stroke(&stroke, transform, branch(child), None, &path);
    }
    let ink = color(style.text_color());
    let [r, g, bl, _] = style.fill.to_bytes();
    let dark_root = 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(bl) < 140.0;
    for placed in &layout.placed {
        let rect = world(placed.rect);
        let node = &map.nodes[placed.index];
        let radius = (rect.height() / 2.0).min(10.0 * scale);
        let shape = RoundedRect::from_rect(rect, radius);
        let brush = if placed.depth == 0 {
            scene.fill(Fill::NonZero, transform, color(style.fill), None, &shape);
            if dark_root {
                Color::from_rgb8(0xFF, 0xFF, 0xFF)
            } else {
                ink
            }
        } else {
            scene.fill(Fill::NonZero, transform, CARD_FILL, None, &shape);
            let width = if placed.depth == 1 { 2.0 } else { 1.0 } * scale;
            scene.stroke(&Stroke::new(width), transform, branch(placed), None, &shape);
            ink
        };
        let text_box = map.placed_text_box(&layout, b, placed);
        text.draw_fitted(scene, transform, (&node.text, &node.spans), text_box, brush);
        let Some(at) = map.toggle_at(placed) else {
            continue;
        };
        let center = world(Rect::from_center_size(at, (0.0, 0.0))).center();
        let r = pb_core::MindMap::toggle_radius() * scale;
        let dot = vello::kurbo::Circle::new(center, r);
        scene.fill(Fill::NonZero, transform, CARD_FILL, None, &dot);
        scene.stroke(
            &Stroke::new(1.5 * scale),
            transform,
            branch(placed),
            None,
            &dot,
        );
        if placed.hidden > 0 {
            let label: Arc<str> = placed.hidden.to_string().into();
            let size = r * 1.1;
            let layout = text.engine.layout(&label, size, None);
            let origin = (
                center.x - layout.width / 2.0,
                center.y - layout.height / 2.0,
            );
            draw_text(scene, transform, origin, &layout, branch(placed));
        } else {
            let arm = r * 0.5;
            let line = Line::new((center.x - arm, center.y), (center.x + arm, center.y));
            scene.stroke(
                &Stroke::new(1.5 * scale),
                transform,
                branch(placed),
                None,
                &line,
            );
        }
    }
}

/// Tag pills: font size relative to a sticky note's side, or in world units under other
/// objects.
const STICKY_TAG_FONT: f64 = 0.07;
const TAG_FONT: f64 = 12.0;

/// Draws the names of `tags` as colored pills: along the bottom inside a sticky note,
/// under other objects. `transform` maps world to physical pixels.
pub(crate) fn draw_tags(
    scene: &mut Scene,
    transform: Affine,
    data: &ObjectData,
    tags: &[&pb_core::Tag],
    text: &mut TextRenderer,
) {
    let b = data.bounds;
    let (size, inside) = match data.kind {
        ObjectKind::Sticky(_) => (b.width().min(b.height()) * STICKY_TAG_FONT, true),
        _ => (TAG_FONT, false),
    };
    let height = size * 1.6;
    let pad = size * 0.6;
    let gap = size * 0.4;
    let y = if inside {
        b.y1 - pad - height
    } else {
        b.y1 + gap * 1.5
    };
    let mut x = b.x0 + if inside { pad } else { 0.0 };
    let right = if inside { b.x1 - pad } else { f64::INFINITY };
    let transform = transform * data.frame().transform();
    let alpha = data.style.opacity.clamp(0.0, 1.0) as f32;
    for tag in tags {
        let layout = text.engine.layout(&tag.name, size, None);
        let width = layout.width + 2.0 * pad;
        if x + width > right && x > b.x0 + pad {
            break;
        }
        let pill = RoundedRect::new(x, y, x + width, y + height, height / 2.0);
        let fill = color(tag.color).multiply_alpha(alpha);
        scene.fill(Fill::NonZero, transform, fill, None, &pill);
        let [r, g, bl, _] = tag.color.to_bytes();
        let light = 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(bl) > 160.0;
        let ink = if light {
            Color::from_rgb8(0x1F, 0x29, 0x37)
        } else {
            Color::from_rgb8(0xFF, 0xFF, 0xFF)
        };
        let top = y + (height - layout.height) / 2.0;
        draw_text(
            scene,
            transform,
            (x + pad, top),
            &layout,
            ink.multiply_alpha(alpha),
        );
        x += width + gap;
    }
}

fn paint_padding(data: &ObjectData) -> f64 {
    match data.kind {
        ObjectKind::Sticky(_) => {
            let side = data.bounds.width().min(data.bounds.height());
            side * (SHADOW_OFFSET + SHADOW_BLUR * 2.5)
        }
        ObjectKind::Image(_) => 0.0,
        _ => data.style.stroke_width / 2.0,
    }
}

fn color(rgba: Rgba) -> Color {
    let [r, g, b, a] = rgba.to_bytes();
    Color::from_rgba8(r, g, b, a)
}
