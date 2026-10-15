//! Scene to GPU: camera, culling, level of detail.

mod gpu;
mod grid;
mod objects;

pub use gpu::Gpu;
pub use grid::{GridColors, GridLevel, GridStyle, draw_grid};
pub use vello;

use pb_core::{Object, ObjectData};
use pb_geom::Camera;
use vello::Scene;
use vello::kurbo::{Affine, Size};
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
    /// Board objects in z-order, back to front.
    pub objects: &'a [Object],
    /// Object being drawn by the active tool, shown on top.
    pub preview: Option<ObjectData>,
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

    let transform = Affine::scale(view.scale) * view.camera.transform();
    let visible = view.camera.visible_world_rect(view.viewport);
    // TODO: query an R-tree instead of scanning every object.
    let on_screen = view
        .objects
        .iter()
        .map(|o| &o.data)
        .filter(|data| objects::painted_bounds(data).overlaps(visible));
    for data in on_screen.chain(view.preview.as_ref()) {
        objects::draw_object(scene, transform, data);
    }
}
