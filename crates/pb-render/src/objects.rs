use pb_core::{ObjectData, ObjectKind, Rgba};
use vello::Scene;
use vello::kurbo::{Affine, Rect, Stroke};
use vello::peniko::{Color, Fill};

/// Draws one object. `transform` maps world to physical pixels.
pub fn draw_object(scene: &mut Scene, transform: Affine, data: &ObjectData) {
    match data.kind {
        ObjectKind::Rectangle => {
            let shape = data.bounds;
            scene.fill(
                Fill::NonZero,
                transform,
                color(data.style.fill),
                None,
                &shape,
            );
            if data.style.stroke_width > 0.0 {
                scene.stroke(
                    &Stroke::new(data.style.stroke_width),
                    transform,
                    color(data.style.stroke),
                    None,
                    &shape,
                );
            }
        }
    }
}

/// World-space area an object paints, including its stroke.
pub fn painted_bounds(data: &ObjectData) -> Rect {
    data.bounds
        .inflate(data.style.stroke_width / 2.0, data.style.stroke_width / 2.0)
}

fn color(rgba: Rgba) -> Color {
    let [r, g, b, a] = rgba.to_bytes();
    Color::from_rgba8(r, g, b, a)
}
