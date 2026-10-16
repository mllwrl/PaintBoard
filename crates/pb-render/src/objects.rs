use pb_core::{ObjectData, ObjectKind, Rgba};
use vello::Scene;
use vello::kurbo::{Affine, Join, Rect, Stroke};
use vello::peniko::{Color, Fill, Mix};

/// Draws one object. `transform` maps world to physical pixels.
pub fn draw_object(scene: &mut Scene, transform: Affine, data: &ObjectData) {
    let transform = transform * data.frame().transform();
    let style = &data.style;
    if style.opacity <= 0.0 {
        return;
    }
    match data.kind {
        ObjectKind::Shape(shape) => {
            let path = shape.outline(data.bounds);
            // Opacity applies to fill and outline together, so overlaps do not show.
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
            if layered {
                scene.pop_layer();
            }
        }
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
