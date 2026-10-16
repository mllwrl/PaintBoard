use pb_core::{ObjectData, ObjectKind, Rgba};
use std::sync::Arc;

use pb_geom::ink::{smooth_path, variable_outline};
use pb_text::TextEngine;
use vello::Scene;
use vello::kurbo::{Affine, Cap, Join, Rect, Stroke};
use vello::peniko::{Blob, FontData};
use vello::peniko::{Color, Fill, Mix};

/// Ink never gets thinner than this fraction of its width, however light the pressure.
const MIN_PRESSURE: f64 = 0.15;

/// Lays out and draws board text with the app font.
///
/// TODO: cache layouts per object; laying out every text each frame will not scale.
pub struct TextRenderer {
    engine: TextEngine,
    font: FontData,
}

impl TextRenderer {
    pub fn new() -> Self {
        Self {
            engine: TextEngine::new(),
            font: FontData::new(Blob::new(Arc::new(pb_text::FONT)), 0),
        }
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

/// Draws one object. `transform` maps world to physical pixels.
pub fn draw_object(
    scene: &mut Scene,
    transform: Affine,
    data: &ObjectData,
    text: &mut TextRenderer,
) {
    let transform = transform * data.frame().transform();
    let style = &data.style;
    if style.opacity <= 0.0 {
        return;
    }
    // Opacity applies to the object as a whole, so its own overlaps do not show.
    let layered = style.opacity < 1.0;
    if layered {
        let pad = style.stroke_width;
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
            let offset = (b.width() - layout.width) / 2.0;
            let glyphs = layout.glyphs.iter().map(|g| vello::Glyph {
                id: g.id,
                x: g.x + offset as f32,
                y: g.y,
            });
            scene
                .draw_glyphs(&text.font)
                .font_size(content.size as f32)
                .transform(transform * Affine::translate((b.x0, top)))
                .brush(color(style.stroke))
                .draw(Fill::NonZero, glyphs);
        }
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

/// World-space axis-aligned area an object paints, including its stroke.
pub fn painted_bounds(data: &ObjectData) -> Rect {
    let pad = data.style.stroke_width / 2.0;
    data.frame().aabb().inflate(pad, pad)
}

fn color(rgba: Rgba) -> Color {
    let [r, g, b, a] = rgba.to_bytes();
    Color::from_rgba8(r, g, b, a)
}
