//! Scene to GPU: camera, culling, level of detail.

mod gpu;
mod grid;
mod images;
mod objects;
mod overlay;

pub use gpu::{FrameTarget, Gpu};
pub use grid::{GridColors, GridStyle, draw_grid};
pub use images::ImageCache;
pub use objects::{TextRenderer, fitted_font_size, text_align};
pub use overlay::{Overlay, OverlayColors, TextEditOverlay};
pub use vello;

use std::collections::HashMap;

use pb_core::{Assets, Object, ObjectData, ObjectId, ObjectKind};
use pb_geom::Camera;
use vello::Scene;
use vello::kurbo::{Affine, Size};
use vello::peniko::Color;

pub struct Theme {
    pub background: Color,
    pub grid: GridColors,
    pub overlay: OverlayColors,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            background: Color::from_rgb8(0xF5, 0xF5, 0xF2),
            grid: GridColors {
                dot: Color::from_rgb8(0xBD, 0xBD, 0xB8),
                line: Color::from_rgb8(0xE2, 0xE2, 0xDE),
            },
            overlay: OverlayColors {
                accent: Color::from_rgb8(0x3B, 0x6E, 0xF5),
                handle_fill: Color::WHITE,
                guide: Color::from_rgb8(0xF2, 0x3D, 0x7A),
                caret: Color::from_rgb8(0x1F, 0x29, 0x37),
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
    /// Files of the board's images.
    pub assets: &'a Assets,
    /// Objects mid-gesture (moved, resized) that replace their stored state.
    pub overrides: &'a [Object],
    /// Objects not to draw (about to be erased).
    pub hidden: &'a [ObjectId],
    /// Objects the active tool is about to create, shown on top.
    pub previews: &'a [ObjectData],
    pub overlay: &'a Overlay,
}

/// Rebuilds `scene` for the given view.
pub fn build_scene(
    scene: &mut Scene,
    view: &View<'_>,
    text: &mut TextRenderer,
    images: &mut ImageCache,
) {
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
    let overrides: HashMap<_, _> = view.overrides.iter().map(|o| (o.id, &o.data)).collect();
    // Objects as they are now, mid-gesture included, for connectors to follow.
    let current: HashMap<_, _> = view
        .objects
        .iter()
        .map(|o| (o.id, overrides.get(&o.id).copied().unwrap_or(&o.data)))
        .collect();
    let shown = view
        .objects
        .iter()
        .filter(|o| !view.hidden.contains(&o.id))
        .map(|o| current[&o.id]);
    for data in shown.chain(view.previews) {
        if let ObjectKind::Connector(c) = &data.kind {
            let width = data.style.stroke_width;
            let path = pb_core::connector::layout(c, width, |id| current.get(&id).copied());
            let label = c.label.rect(&path).map(|r| (&c.label, r));
            let area = path.bounds().inflate(width, width);
            let area = label.map_or(area, |(_, r)| area.union(r));
            if area.overlaps(visible) {
                objects::draw_connector(scene, transform, &data.style, &path, label, text);
            }
            continue;
        }
        if !objects::painted_bounds(data).overlaps(visible) {
            continue;
        }
        let resources = objects::Resources {
            text: &mut *text,
            images: &mut *images,
            assets: view.assets,
        };
        objects::draw_object(scene, transform, data, resources);
    }

    overlay::draw_overlay(
        scene,
        view.camera,
        view.scale,
        view.overlay,
        &view.theme.overlay,
    );
}
