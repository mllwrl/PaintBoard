use pb_geom::{Camera, Frame};
use vello::Scene;
use vello::kurbo::{
    Affine, Arc, BezPath, Circle, Line, Point, Rect, RoundedRect, Shape, Stroke, Vec2,
};
use vello::peniko::{Color, Fill};

/// Interaction feedback drawn above the board, in screen space so it keeps a constant size.
#[derive(Clone, Debug, Default)]
pub struct Overlay {
    /// Frames outlined faintly inside the selection (world): each selected object, or the
    /// members of a selected group.
    pub selected: Vec<Frame>,
    /// Frame the handles attach to (world).
    pub selection: Option<Frame>,
    pub show_handles: bool,
    /// The selection contains locked objects: shows a padlock next to it.
    pub locked: bool,
    /// Rotation knob position (world), shown with the handles.
    pub rotate_knob: Option<Point>,
    /// Object under the pointer (world).
    pub hover: Option<Frame>,
    /// Rubber-band selection rectangle (world).
    pub marquee: Option<Rect>,
    /// Smart guides showing what a dragged object lines up with (world).
    pub guides: Vec<Line>,
    /// The eraser: center and radius (world).
    pub eraser: Option<(Point, f64)>,
    /// Text being edited.
    pub text_edit: Option<TextEditOverlay>,
    /// Lines outlined like the hovered or selected object: connectors (world).
    pub lines: Vec<BezPath>,
    /// Where `lines` break for connector labels (world).
    pub gaps: Vec<Rect>,
    /// Handles on the ends of a selected connector (world).
    pub end_handles: Vec<Point>,
    /// Where a connector can attach on the object under the pointer (world), and the one it
    /// would snap to.
    pub connection_points: Vec<Point>,
    pub connection_active: Option<Point>,
    /// Quick create's "+" dots (world) and their radius in logical pixels.
    pub quick_dots: Vec<Point>,
    pub quick_dot_radius: f64,
}

/// The text box being edited, in world space.
#[derive(Clone, Debug)]
pub struct TextEditOverlay {
    pub frame: Frame,
    /// Selected ranges as quads (they follow the text's rotation).
    pub selection: Vec<[Point; 4]>,
    /// Top and bottom of the caret, hidden while text is selected.
    pub caret: Option<(Point, Point)>,
    pub caret_width: f64,
}

pub struct OverlayColors {
    pub accent: Color,
    pub handle_fill: Color,
    pub guide: Color,
    pub caret: Color,
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
    let world_to_px = Affine::scale(scale) * camera.transform();
    let gapped = !overlay.lines.is_empty() && !overlay.gaps.is_empty();
    if gapped {
        let area = overlay
            .lines
            .iter()
            .map(|l| l.bounding_box())
            .chain(overlay.gaps.iter().copied())
            .reduce(|a, b| a.union(b))
            .unwrap_or_default();
        // Wide enough for the outline's width at any zoom.
        let margin = area.width().max(area.height()) + 1.0;
        let area = area.inflate(margin, margin);
        crate::objects::clip_out(scene, world_to_px, area, &overlay.gaps);
    }
    for line in &overlay.lines {
        scene.stroke(
            &thick,
            id,
            colors.accent,
            None,
            &(world_to_px * line.clone()),
        );
    }
    if gapped {
        scene.pop_layer();
    }
    if !overlay.selected.is_empty() {
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
        if overlay.locked {
            // Just outside the top-right corner, whatever the rotation.
            let top_right = corners
                .into_iter()
                .fold(corners[0], |a, b| if b.x - b.y > a.x - a.y { b } else { a });
            let center = top_right + Vec2::new(LOCK_GAP + LOCK_SIZE / 2.0, -LOCK_GAP) * scale;
            draw_padlock(scene, center, scale, colors);
        }
    }
    for guide in &overlay.guides {
        let line = Line::new(to_px(guide.p0), to_px(guide.p1));
        // Snap to pixel centers like outlines, so the 1px line stays crisp.
        let snap = |v: f64| v.round() + 0.5;
        let line = if line.p0.x == line.p1.x {
            Line::new((snap(line.p0.x), line.p0.y), (snap(line.p1.x), line.p1.y))
        } else {
            Line::new((line.p0.x, snap(line.p0.y)), (line.p1.x, snap(line.p1.y)))
        };
        scene.stroke(&thin, id, colors.guide, None, &line);
    }
    let handle_stroke = Stroke::new(1.5 * scale);
    for &end in &overlay.end_handles {
        let handle = Circle::new(to_px(end), HANDLE_RADIUS * scale);
        scene.fill(Fill::NonZero, id, colors.handle_fill, None, &handle);
        scene.stroke(&handle_stroke, id, colors.accent, None, &handle);
    }
    for &point in &overlay.connection_points {
        let active = overlay.connection_active == Some(point);
        let radius = if active { 5.0 } else { 3.5 } * scale;
        let dot = Circle::new(to_px(point), radius);
        let fill = if active {
            colors.accent
        } else {
            colors.handle_fill
        };
        scene.fill(Fill::NonZero, id, fill, None, &dot);
        scene.stroke(&handle_stroke, id, colors.accent, None, &dot);
    }
    for &point in &overlay.quick_dots {
        let center = to_px(point);
        let r = overlay.quick_dot_radius * scale;
        let dot = Circle::new(center, r);
        scene.fill(Fill::NonZero, id, colors.handle_fill, None, &dot);
        scene.stroke(&handle_stroke, id, colors.accent, None, &dot);
        let arm = r * 0.5;
        for d in [Vec2::new(arm, 0.0), Vec2::new(0.0, arm)] {
            let line = Line::new(center - d, center + d);
            scene.stroke(&handle_stroke, id, colors.accent, None, &line);
        }
    }
    if let Some(edit) = &overlay.text_edit {
        let selection = colors.accent.multiply_alpha(0.25);
        for quad in &edit.selection {
            let path = frame_path(quad.map(to_px), false);
            scene.fill(Fill::NonZero, id, selection, None, &path);
        }
        scene.stroke(&thin, id, colors.accent, None, &outline(&edit.frame));
        if let Some((top, bottom)) = edit.caret {
            let width = (edit.caret_width * camera.zoom() * scale).max(1.5 * scale);
            let caret = Line::new(to_px(top), to_px(bottom));
            scene.stroke(&Stroke::new(width), id, colors.caret, None, &caret);
        }
    }
    if let Some((center, radius)) = overlay.eraser {
        let circle = Circle::new(to_px(center), radius * camera.zoom() * scale);
        scene.fill(
            Fill::NonZero,
            id,
            colors.handle_fill.multiply_alpha(0.6),
            None,
            &circle,
        );
        scene.stroke(&thin, id, Color::from_rgb8(0x6B, 0x72, 0x80), None, &circle);
    }
    if let Some(marquee) = overlay.marquee {
        let r = frame_path(Frame::axis_aligned(marquee).corners().map(to_px), true);
        let fill = colors.accent.multiply_alpha(0.08);
        scene.fill(Fill::NonZero, id, fill, None, &r);
        scene.stroke(&thin, id, colors.accent, None, &r);
    }
}

/// Padlock icon size and distance from the selection, in logical pixels.
const LOCK_SIZE: f64 = 10.0;
const LOCK_GAP: f64 = 6.0;

/// A small padlock centered on `center` (physical pixels).
fn draw_padlock(scene: &mut Scene, center: Point, scale: f64, colors: &OverlayColors) {
    let s = LOCK_SIZE * scale;
    let body = RoundedRect::new(
        center.x - s / 2.0,
        center.y - s * 0.1,
        center.x + s / 2.0,
        center.y + s * 0.6,
        1.5 * scale,
    );
    let shackle = Arc::new(
        (center.x, center.y - s * 0.1),
        (s * 0.28, s * 0.4),
        std::f64::consts::PI,
        std::f64::consts::PI,
        0.0,
    );
    let id = Affine::IDENTITY;
    scene.stroke(&Stroke::new(1.6 * scale), id, colors.accent, None, &shackle);
    scene.fill(Fill::NonZero, id, colors.accent, None, &body);
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
