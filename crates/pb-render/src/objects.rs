use pb_core::{Assets, Image, ObjectData, ObjectKind, Rgba, Sticky};
use std::collections::HashMap;
use std::sync::Arc;

use pb_geom::ink::{smooth_path, variable_outline};
use pb_text::{TextAlign, TextEngine};
use vello::Scene;
use vello::kurbo::{Affine, Cap, Join, Line, Rect, Stroke};
use vello::peniko::{Blob, FontData};
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

/// Fitted sizes kept by [`TextRenderer`]; the cache is dropped when it grows past this.
const FIT_CACHE_LIMIT: usize = 4096;

/// Font size of a sticky note's text: as large as fits in its text area.
pub fn sticky_font_size(engine: &mut TextEngine, text: &str, bounds: Rect) -> f64 {
    let area = Sticky::text_area(bounds);
    engine.fit_size(
        text,
        area.width(),
        area.height(),
        Sticky::max_font_size(bounds),
    )
}

/// Lays out and draws board text with the app font.
///
/// TODO: cache layouts per object; laying out every text each frame will not scale.
pub struct TextRenderer {
    engine: TextEngine,
    font: FontData,
    /// Sticky note font sizes by text and note size, as fitting takes several layouts.
    fitted: HashMap<(Arc<str>, u64, u64), f64>,
}

impl TextRenderer {
    pub fn new() -> Self {
        Self {
            engine: TextEngine::new(),
            font: FontData::new(Blob::new(Arc::new(pb_text::FONT)), 0),
            fitted: HashMap::new(),
        }
    }

    fn sticky_font_size(&mut self, text: &Arc<str>, bounds: Rect) -> f64 {
        let key = (
            text.clone(),
            bounds.width().to_bits(),
            bounds.height().to_bits(),
        );
        if let Some(&size) = self.fitted.get(&key) {
            return size;
        }
        if self.fitted.len() >= FIT_CACHE_LIMIT {
            self.fitted.clear();
        }
        let size = sticky_font_size(&mut self.engine, text, bounds);
        self.fitted.insert(key, size);
        size
    }

    /// Draws `text` laid out at `size`, its top-left corner at `origin` in `transform`.
    fn draw(
        &mut self,
        scene: &mut Scene,
        transform: Affine,
        origin: (f64, f64),
        size: f64,
        glyphs: &[pb_text::Glyph],
        brush: Color,
    ) {
        let glyphs = glyphs.iter().map(|g| vello::Glyph {
            id: g.id,
            x: g.x,
            y: g.y,
        });
        scene
            .draw_glyphs(&self.font)
            .font_size(size as f32)
            .transform(transform * Affine::translate(origin))
            .brush(brush)
            .draw(Fill::NonZero, glyphs);
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

/// What drawing objects needs besides the scene.
pub(crate) struct Resources<'a> {
    pub text: &'a mut TextRenderer,
    pub images: &'a mut crate::ImageCache,
    pub assets: &'a Assets,
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
        let clip = data.bounds.inflate(pad, pad);
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
            let path = shape.outline(data.bounds);
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
        }
        ObjectKind::Text(content) => {
            let b = data.bounds;
            // A hair of slack, so text measured to fit exactly does not wrap.
            let layout = text
                .engine
                .layout(&content.text, content.size, Some(b.width() + 0.5));
            let top = b.y0 + (b.height() - layout.height) / 2.0;
            let brush = color(style.stroke);
            text.draw(
                scene,
                transform,
                (b.x0, top),
                content.size,
                &layout.glyphs,
                brush,
            );
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
            if !sticky.text.is_empty() {
                let size = text.sticky_font_size(&sticky.text, b);
                let area = Sticky::text_area(b);
                let layout = text.engine.layout_aligned(
                    &sticky.text,
                    size,
                    Some(area.width()),
                    TextAlign::Center,
                );
                let top = area.y0 + (area.height() - layout.height) / 2.0;
                let brush = color(style.stroke);
                text.draw(
                    scene,
                    transform,
                    (area.x0, top),
                    size,
                    &layout.glyphs,
                    brush,
                );
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
