//! Scene to GPU: camera, culling, level of detail.

mod gpu;
mod grid;

pub use gpu::Gpu;
pub use grid::{GridColors, GridLevel, GridStyle, draw_grid};
pub use vello;

use pb_geom::Camera;
use vello::Scene;
use vello::kurbo::Size;
use vello::peniko::Color;

pub struct Theme {
    pub background: Color,
    pub grid: GridColors,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            background: Color::from_rgb8(0xF5, 0xF5, 0xF2),
            grid: GridColors {
                dot: Color::from_rgb8(0xBD, 0xBD, 0xB8),
                line: Color::from_rgb8(0xE2, 0xE2, 0xDE),
            },
        }
    }
}

/// Everything needed to draw one frame of the canvas.
pub struct View<'a> {
    pub camera: &'a Camera,
    /// Viewport size in logical pixels.
    pub viewport: Size,
    /// Display scale factor (physical pixels per logical pixel).
    pub scale: f64,
    pub grid: GridStyle,
    pub theme: &'a Theme,
}

/// Rebuilds `scene` for the given view.
pub fn build_scene(scene: &mut Scene, view: &View<'_>) {
    scene.reset();
    draw_grid(
        scene,
        view.camera,
        view.viewport,
        view.scale,
        view.grid,
        &view.theme.grid,
    );
}
