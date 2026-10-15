use pb_geom::Camera;
use vello::Scene;
use vello::kurbo::{Affine, Circle, Point, Rect, Stroke};
use vello::peniko::{Color, Fill};

/// Interaction feedback drawn above the board, in screen space so it keeps a constant size.
#[derive(Clone, Debug, Default)]
pub struct Overlay {
    /// Bounds of each selected object (world). Outlined individually when more than one.
    pub selected: Vec<Rect>,
    /// Union of the selection (world).
    pub selection_bounds: Option<Rect>,
    pub show_handles: bool,
    /// Object under the pointer (world).
    pub hover: Option<Rect>,
    /// Rubber-band selection rectangle (world).
    pub marquee: Option<Rect>,
}

pub struct OverlayColors {
    pub accent: Color,
    pub handle_fill: Color,
}

/// Handle radius in logical pixels.
const HANDLE_RADIUS: f64 = 4.5;

pub fn draw_overlay(
    scene: &mut Scene,
    camera: &Camera,
    scale: f64,
    overlay: &Overlay,
    colors: &OverlayColors,
) {
    let to_px = |p: Point| (camera.world_to_screen(p).to_vec2() * scale).to_point();
    // Snapped to pixel centers so thin outlines stay crisp.
    let screen_rect = |r: Rect| {
        let snap = |p: Point| Point::new(p.x.round() + 0.5, p.y.round() + 0.5);
        Rect::from_points(snap(to_px(r.origin())), snap(to_px(Point::new(r.x1, r.y1))))
    };
    let thin = Stroke::new(scale.max(1.0));
    let thick = Stroke::new(2.0 * scale);

    if let Some(hover) = overlay.hover {
        scene.stroke(
            &thick,
            Affine::IDENTITY,
            colors.accent,
            None,
            &screen_rect(hover),
        );
    }
    if overlay.selected.len() > 1 {
        let faint = colors.accent.multiply_alpha(0.5);
        for &r in &overlay.selected {
            scene.stroke(&thin, Affine::IDENTITY, faint, None, &screen_rect(r));
        }
    }
    if let Some(bounds) = overlay.selection_bounds {
        let r = screen_rect(bounds);
        scene.stroke(&thick, Affine::IDENTITY, colors.accent, None, &r);
        if overlay.show_handles {
            let handle_stroke = Stroke::new(1.5 * scale);
            let corners_and_edges = [
                (r.x0, r.y0),
                (r.center().x, r.y0),
                (r.x1, r.y0),
                (r.x1, r.center().y),
                (r.x1, r.y1),
                (r.center().x, r.y1),
                (r.x0, r.y1),
                (r.x0, r.center().y),
            ];
            for center in corners_and_edges {
                let handle = Circle::new(center, HANDLE_RADIUS * scale);
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    colors.handle_fill,
                    None,
                    &handle,
                );
                scene.stroke(
                    &handle_stroke,
                    Affine::IDENTITY,
                    colors.accent,
                    None,
                    &handle,
                );
            }
        }
    }
    if let Some(marquee) = overlay.marquee {
        let r = screen_rect(marquee);
        let fill = colors.accent.multiply_alpha(0.08);
        scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &r);
        scene.stroke(&thin, Affine::IDENTITY, colors.accent, None, &r);
    }
}
