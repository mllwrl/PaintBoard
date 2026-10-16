use pb_geom::{Camera, GridLevel};
use vello::Scene;
use vello::kurbo::{Affine, BezPath, Circle, Shape, Size, Stroke};
use vello::peniko::{Color, Fill};

/// Background pattern of the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridStyle {
    #[default]
    Dots,
    Lines,
    None,
}

impl GridStyle {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dots => "Dots",
            Self::Lines => "Lines",
            Self::None => "No grid",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Dots => Self::Lines,
            Self::Lines => Self::None,
            Self::None => Self::Dots,
        }
    }
}

pub struct GridColors {
    pub dot: Color,
    pub line: Color,
}

/// Draws the grid in physical pixels so lines stay crisp at any zoom.
///
/// `viewport` is in logical pixels, `scale` is the display scale factor.
pub fn draw_grid(
    scene: &mut Scene,
    camera: &Camera,
    viewport: Size,
    scale: f64,
    style: GridStyle,
    colors: &GridColors,
) {
    if style == GridStyle::None {
        return;
    }
    let level = GridLevel::for_zoom(camera.zoom());
    let s = level.spacing;
    let world = camera.visible_world_rect(viewport);
    let (x0, x1) = ((world.x0 / s).floor() as i64, (world.x1 / s).ceil() as i64);
    let (y0, y1) = ((world.y0 / s).floor() as i64, (world.y1 / s).ceil() as i64);
    let to_px = |i: i64, origin: f64| (i as f64 * s - origin) * camera.zoom() * scale;
    let origin = camera.origin();
    let width_px = viewport.width * scale;
    let height_px = viewport.height * scale;

    let mut fine = BezPath::new();
    let mut coarse = BezPath::new();

    match style {
        GridStyle::Lines => {
            // Snap to pixel centers so 1px lines are not smeared over two pixels.
            let snap = |v: f64| v.floor() + 0.5;
            for i in x0..=x1 {
                let x = snap(to_px(i, origin.x));
                let path = if i % 2 == 0 { &mut coarse } else { &mut fine };
                path.move_to((x, 0.0));
                path.line_to((x, height_px));
            }
            for j in y0..=y1 {
                let y = snap(to_px(j, origin.y));
                let path = if j % 2 == 0 { &mut coarse } else { &mut fine };
                path.move_to((0.0, y));
                path.line_to((width_px, y));
            }
            let stroke = Stroke::new(scale.round().max(1.0));
            draw_level(
                scene,
                &fine,
                &coarse,
                level.fine_alpha,
                colors.line,
                |sc, p, c| {
                    sc.stroke(&stroke, Affine::IDENTITY, c, None, p);
                },
            );
        }
        GridStyle::Dots => {
            let radius = 1.1 * scale;
            for i in x0..=x1 {
                let x = to_px(i, origin.x);
                for j in y0..=y1 {
                    let y = to_px(j, origin.y);
                    let path = if i % 2 == 0 && j % 2 == 0 {
                        &mut coarse
                    } else {
                        &mut fine
                    };
                    path.extend(Circle::new((x, y), radius).path_elements(0.1));
                }
            }
            draw_level(
                scene,
                &fine,
                &coarse,
                level.fine_alpha,
                colors.dot,
                |sc, p, c| {
                    sc.fill(Fill::NonZero, Affine::IDENTITY, c, None, p);
                },
            );
        }
        GridStyle::None => {}
    }
}

fn draw_level(
    scene: &mut Scene,
    fine: &BezPath,
    coarse: &BezPath,
    fine_alpha: f64,
    color: Color,
    draw: impl Fn(&mut Scene, &BezPath, Color),
) {
    if fine_alpha > 0.01 {
        draw(scene, fine, color.multiply_alpha(fine_alpha as f32));
    }
    draw(scene, coarse, color);
}
