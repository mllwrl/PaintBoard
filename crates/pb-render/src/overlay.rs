use pb_geom::{Camera, Frame};
use vello::Scene;
use vello::kurbo::{Affine, BezPath, Circle, Line, Point, Rect, Stroke};
use vello::peniko::{Color, Fill};

/// Interaction feedback drawn above the board, in screen space so it keeps a constant size.
#[derive(Clone, Debug, Default)]
pub struct Overlay {
    /// Frame of each selected object (world). Outlined individually when more than one.
    pub selected: Vec<Frame>,
    /// Frame the handles attach to (world).
    pub selection: Option<Frame>,
    pub show_handles: bool,
    /// Rotation knob position (world), shown with the handles.
    pub rotate_knob: Option<Point>,
    /// Object under the pointer (world).
    pub hover: Option<Frame>,
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
    let outline = |frame: &Frame| frame_path(frame.corners().map(to_px), frame.rotation == 0.0);
    let thin = Stroke::new(scale.max(1.0));
    let thick = Stroke::new(2.0 * scale);
    let id = Affine::IDENTITY;

    if let Some(hover) = &overlay.hover {
        scene.stroke(&thick, id, colors.accent, None, &outline(hover));
    }
    if overlay.selected.len() > 1 {
        let faint = colors.accent.multiply_alpha(0.5);
        for frame in &overlay.selected {
            scene.stroke(&thin, id, faint, None, &outline(frame));
        }
    }
    if let Some(frame) = &overlay.selection {
        let corners = frame.corners().map(to_px);
        scene.stroke(&thick, id, colors.accent, None, &outline(frame));
        if overlay.show_handles {
            let mid = |a: Point, b: Point| a.midpoint(b);
            let [tl, tr, br, bl] = corners;
            let mut handles = vec![
                tl,
                mid(tl, tr),
                tr,
                mid(tr, br),
                br,
                mid(br, bl),
                bl,
                mid(bl, tl),
            ];
            if let Some(knob) = overlay.rotate_knob {
                let knob = to_px(knob);
                scene.stroke(
                    &thin,
                    id,
                    colors.accent,
                    None,
                    &Line::new(mid(tl, tr), knob),
                );
                handles.push(knob);
            }
            let handle_stroke = Stroke::new(1.5 * scale);
            for center in handles {
                let handle = Circle::new(center, HANDLE_RADIUS * scale);
                scene.fill(Fill::NonZero, id, colors.handle_fill, None, &handle);
                scene.stroke(&handle_stroke, id, colors.accent, None, &handle);
            }
        }
    }
    if let Some(marquee) = overlay.marquee {
        let r = frame_path(Frame::axis_aligned(marquee).corners().map(to_px), true);
        let fill = colors.accent.multiply_alpha(0.08);
        scene.fill(Fill::NonZero, id, fill, None, &r);
        scene.stroke(&thin, id, colors.accent, None, &r);
    }
}

/// Closed outline through screen-space corners. Axis-aligned outlines are snapped to pixel
/// centers so thin lines stay crisp.
fn frame_path(corners: [Point; 4], snap: bool) -> BezPath {
    let snap_px = |p: Point| {
        if snap {
            Point::new(p.x.round() + 0.5, p.y.round() + 0.5)
        } else {
            p
        }
    };
    let mut path = BezPath::new();
    path.move_to(snap_px(corners[0]));
    for &c in &corners[1..] {
        path.line_to(snap_px(c));
    }
    path.close_path();
    path
}
