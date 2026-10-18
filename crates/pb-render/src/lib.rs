//! Scene to GPU: camera, culling, level of detail.

mod gpu;
mod grid;
mod images;
mod objects;
mod overlay;

pub use gpu::{FrameTarget, Gpu};
pub use grid::{GridColors, GridStyle, draw_grid};
pub use images::ImageCache;
pub use objects::{TextRenderer, fitted_font_size, painted_bounds, text_align};
pub use overlay::{
    COMMENT_PIN_RADIUS, LINK_BADGE_RADIUS, Overlay, OverlayColors, TextEditOverlay,
    comment_pin_center, link_badge_center,
};
pub use vello;

use std::collections::{HashMap, HashSet};

use pb_core::{Assets, Document, FrameData, Object, ObjectData, ObjectId, ObjectKind};
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
    /// The board; what is drawn is looked up in it by area.
    pub document: &'a Document,
    /// Files of the board's images.
    pub assets: &'a Assets,
    /// Objects mid-gesture (moved, resized) that replace their stored state.
    pub overrides: &'a [Object],
    /// Objects not to draw (about to be erased).
    pub hidden: &'a [ObjectId],
    /// Objects the active tool is about to create, shown on top.
    pub previews: &'a [ObjectData],
    pub overlay: &'a Overlay,
    /// Objects it does not let through are faded.
    pub filter: &'a pb_core::Filter,
    /// Who is looking: their own reactions stand out.
    pub me: &'a str,
    /// Vote counts to show on objects, and whether each is among the top ones.
    pub votes: &'a HashMap<ObjectId, (u32, bool)>,
    /// Drawn for export: no grid, overlay or vote badges, hidden objects left out, and
    /// nothing faded by the filter or focus.
    pub plain: bool,
    /// When set, only these objects are drawn.
    pub only: Option<&'a HashSet<ObjectId>>,
    /// Frames show their titles above them (left out in a picture of one frame, where
    /// the titles of frames below it would show inside).
    pub frame_titles: bool,
}

/// Opacity of objects a filter hides, relative to their own.
const FILTERED_OPACITY: f64 = 0.15;

/// Opacity of objects outside the focus area, relative to their own.
const UNFOCUSED_OPACITY: f64 = 0.06;

/// Opacity of hidden objects while hidden ones are shown, relative to their own.
const HIDDEN_OPACITY: f64 = 0.3;

/// Ids of the objects that may show in `visible` at `zoom`, back to front: those the
/// spatial index finds there (with room for frame titles and sticky shadows), the ones
/// mid-gesture, and connectors attached to those (their stored path is out of date).
fn shown_objects(
    doc: &Document,
    visible: vello::kurbo::Rect,
    zoom: f64,
    overrides: &HashMap<ObjectId, &ObjectData>,
) -> Vec<ObjectId> {
    let title = FrameData::title_band(vello::kurbo::Rect::ZERO, zoom).height();
    let margin = title + visible.width().max(visible.height()) * 0.02;
    let mut ids: Vec<ObjectId> = doc
        .objects_in(visible.inflate(margin, margin))
        .into_iter()
        .map(|o| o.id)
        .collect();
    if !overrides.is_empty() {
        ids.extend(overrides.keys().copied());
        let follows = |end: &pb_core::End| end.object().is_some_and(|o| overrides.contains_key(&o));
        ids.extend(doc.objects().iter().filter_map(|o| match &o.data.kind {
            ObjectKind::Connector(c) if follows(&c.start) || follows(&c.end) => Some(o.id),
            _ => None,
        }));
        ids.sort_by_key(|&id| doc.paint_order(id));
        ids.dedup();
    }
    ids
}

/// Rebuilds `scene` for the given view.
pub fn build_scene(
    scene: &mut Scene,
    view: &View<'_>,
    text: &mut TextRenderer,
    images: &mut ImageCache,
) {
    scene.reset();
    if !view.plain {
        draw_grid(
            scene,
            view.camera,
            view.viewport,
            view.scale,
            view.grid,
            &view.theme.grid,
        );
    }

    let transform = Affine::scale(view.scale) * view.camera.transform();
    let visible = view.camera.visible_world_rect(view.viewport);
    let zoom = view.camera.zoom();
    let doc = view.document;
    let overrides: HashMap<_, _> = view.overrides.iter().map(|o| (o.id, &o.data)).collect();
    // Objects as they are now, mid-gesture included.
    let current = |id: ObjectId| {
        overrides
            .get(&id)
            .copied()
            .or_else(|| doc.get(id).map(|o| &o.data))
    };
    let shown: Vec<(Option<ObjectId>, &ObjectData)> = shown_objects(doc, visible, zoom, &overrides)
        .into_iter()
        .filter(|id| !view.hidden.contains(id) && doc.is_shown(*id))
        .filter(|&id| !view.plain || !doc.is_hidden(id))
        .filter(|id| view.only.is_none_or(|only| only.contains(id)))
        .filter_map(|id| current(id).map(|data| (Some(id), data)))
        .collect();
    // Obstacles for elbow connectors, where they are now.
    let obstacles_in = |area: vello::kurbo::Rect| -> Vec<vello::kurbo::Rect> {
        let stored = doc
            .objects_in(area)
            .into_iter()
            .filter(|o| !overrides.contains_key(&o.id))
            .map(|o| &o.data);
        stored
            .chain(overrides.values().copied())
            .filter_map(pb_core::connector::obstacle)
            .filter(|r| r.overlaps(area))
            .collect()
    };
    // Frames lie under everything else.
    let frames = shown.iter().filter(|(_, d)| d.kind.is_frame());
    let others = shown.iter().filter(|(_, d)| !d.kind.is_frame());
    let previews = view.previews.iter().map(|d| (None, d));
    for (id, data) in frames.chain(others).copied().chain(previews) {
        let mut opacity = 1.0;
        if !view.plain && !view.filter.matches(data) {
            opacity *= FILTERED_OPACITY;
        }
        let focused = view.plain || id.is_none_or(|id| doc.in_focus(id));
        if !focused {
            opacity *= UNFOCUSED_OPACITY;
        }
        if !view.plain && id.is_some_and(|id| doc.is_hidden(id)) {
            opacity *= HIDDEN_OPACITY;
        }
        let faded;
        let data = if opacity == 1.0 {
            data
        } else {
            let mut copy = data.clone();
            copy.style.opacity *= opacity;
            faded = copy;
            &faded
        };
        if let ObjectKind::Connector(c) = &data.kind {
            let width = data.style.stroke_width;
            let (_, _, path) = pb_core::connector::layout(c, width, current, obstacles_in);
            let label = c.label.rect(&path).map(|r| (&c.label, r));
            let area = path.bounds().inflate(width, width);
            let area = label.map_or(area, |(_, r)| area.union(r));
            if area.overlaps(visible) {
                objects::draw_connector(scene, transform, &data.style, &path, label, text);
            }
            continue;
        }
        let mut painted = objects::painted_bounds(data);
        if data.kind.is_frame() {
            painted = painted.union(FrameData::title_band(data.bounds, zoom));
        }
        if !painted.overlaps(visible) {
            continue;
        }
        let resources = objects::Resources {
            text: &mut *text,
            images: &mut *images,
            assets: view.assets,
            zoom,
            frame_titles: view.frame_titles,
        };
        objects::draw_object(scene, transform, data, resources);
        if !focused {
            continue;
        }
        let tags: Vec<_> = data.tags.iter().filter_map(|t| doc.tag(t)).collect();
        if !tags.is_empty() {
            objects::draw_tags(scene, transform, data, &tags, text);
        }
        if let Some(&(count, top)) = id
            .and_then(|id| view.votes.get(&id))
            .filter(|_| !view.plain)
        {
            objects::draw_vote_badge(scene, transform, data, count, top, zoom, text);
        }
        if let Some(id) = id {
            let reactions = doc.reactions(id);
            if !reactions.is_empty() {
                objects::draw_reactions(scene, transform, data, reactions, view.me, text);
            }
        }
    }

    if view.plain {
        return;
    }
    overlay::draw_overlay(
        scene,
        view.camera,
        view.scale,
        view.overlay,
        &view.theme.overlay,
    );
}
