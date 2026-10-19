//! Scene to GPU: camera, culling, level of detail.

mod gpu;
mod grid;
mod images;
mod objects;
mod overlay;

pub use gpu::{FrameTarget, Gpu, Offscreen, PendingImage};
pub use grid::{GridColors, GridStyle, draw_grid};
pub use images::ImageCache;
pub use objects::{TextRenderer, fitted_font_size, painted_bounds, text_align};
pub use overlay::{
    COMMENT_PIN_RADIUS, LINK_BADGE_RADIUS, Overlay, OverlayColors, PAGE_TURN_RADIUS,
    TextEditOverlay, comment_pin_center, link_badge_center, page_turn_center,
};
pub use vello;

use std::collections::{HashMap, HashSet};

use pb_core::{Assets, Document, FrameData, Object, ObjectData, ObjectId, ObjectKind};
use pb_geom::Camera;
use vello::Scene;
use vello::kurbo::{Affine, Size};
use vello::peniko::Color;

pub struct Theme {
    /// Colors picked for a light board are turned around where they would not show
    /// (see [`for_dark`]).
    pub dark: bool,
    pub background: Color,
    pub grid: GridColors,
    pub overlay: OverlayColors,
    /// A frame's edge and title.
    pub frame_edge: Color,
    pub frame_title: Color,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            dark: false,
            frame_edge: Color::from_rgb8(0xD0, 0xD0, 0xCB),
            frame_title: Color::from_rgb8(0x5F, 0x63, 0x68),
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

    pub fn dark() -> Self {
        Self {
            dark: true,
            frame_edge: Color::from_rgb8(0x4A, 0x4D, 0x52),
            frame_title: Color::from_rgb8(0xA3, 0xA7, 0xAE),
            background: Color::from_rgb8(0x1E, 0x1F, 0x22),
            grid: GridColors {
                dot: Color::from_rgb8(0x48, 0x4A, 0x4F),
                line: Color::from_rgb8(0x2C, 0x2E, 0x32),
            },
            overlay: OverlayColors {
                accent: Color::from_rgb8(0x6E, 0x95, 0xFF),
                handle_fill: Color::WHITE,
                guide: Color::from_rgb8(0xFF, 0x5C, 0x93),
                // Shows on dark text boxes and light sticky notes alike.
                caret: Color::from_rgb8(0x6E, 0x95, 0xFF),
            },
        }
    }
}

/// Fill of frames left white, on a dark board.
const DARK_FRAME: pb_core::Rgba = pb_core::Rgba::rgb(0x2A, 0x2C, 0x30);

/// `data` as drawn on a dark board, if that differs: near-black lines and text lying on
/// the board itself (ink, connectors, text, shapes without a fill) turn light, and white
/// frames dark. Colors on a fill of their own (notes, tables, cards) stay as they are.
pub fn for_dark(data: &ObjectData) -> Option<ObjectData> {
    let on_board = match &data.kind {
        // Text objects never paint their fill: their text always lies on the board.
        ObjectKind::Ink(_) | ObjectKind::Connector(_) | ObjectKind::Text(_) => true,
        ObjectKind::Shape(_) => data.style.fill.to_bytes()[3] == 0,
        ObjectKind::Frame(_) => {
            if data.style.fill.to_bytes() != [0xFF, 0xFF, 0xFF, 0xFF] {
                return None;
            }
            let mut copy = data.clone();
            copy.style.fill = DARK_FRAME;
            return Some(copy);
        }
        _ => false,
    };
    if !on_board {
        return None;
    }
    let stroke = lighten(data.style.stroke);
    let text = data.style.text_color.and_then(lighten);
    if stroke.is_none() && text.is_none() {
        return None;
    }
    let mut copy = data.clone();
    if let Some(stroke) = stroke {
        copy.style.stroke = stroke;
    }
    if let Some(text) = text {
        copy.style.text_color = Some(text);
    }
    Some(copy)
}

/// A light counterpart of a dark gray (black included); `None` for other colors.
fn lighten(color: pb_core::Rgba) -> Option<pb_core::Rgba> {
    let [r, g, b, a] = color.to_bytes();
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if max >= 0x60 || max - min > 0x30 {
        return None;
    }
    let flip = |v: u8| 0xF0 - v / 2;
    let [r, g, b, _] = pb_core::Rgba::rgb(flip(r), flip(g), flip(b)).to_bytes();
    Some(pb_core::Rgba(u32::from_be_bytes([r, g, b, a])))
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

/// Objects smaller than this on screen (logical pixels) are drawn as plain boxes.
const LOD_BOX: f64 = 6.0;

/// Objects smaller than this on screen (logical pixels) are drawn without their text,
/// tags and badges, which could not be read anyway.
const LOD_TEXT: f64 = 24.0;

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
        let adapted;
        let data = match view.theme.dark.then(|| for_dark(data)).flatten() {
            Some(copy) => {
                adapted = copy;
                &adapted
            }
            None => data,
        };
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
            let bounds = path.bounds();
            let readable = bounds.width().max(bounds.height()) * zoom >= LOD_TEXT;
            let label = c
                .label
                .rect(&path)
                .filter(|_| readable)
                .map(|r| (&c.label, r));
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
        let on_screen = painted.width().max(painted.height()) * zoom;
        if on_screen < LOD_BOX {
            objects::draw_box(scene, transform, data);
            continue;
        }
        text.skip = on_screen < LOD_TEXT;
        let resources = objects::Resources {
            text: &mut *text,
            images: &mut *images,
            assets: view.assets,
            zoom,
            frame_titles: view.frame_titles,
            frame_colors: (view.theme.frame_edge, view.theme.frame_title),
        };
        objects::draw_object(scene, transform, data, resources);
        let small = std::mem::replace(&mut text.skip, false);
        if !focused || small {
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

#[cfg(test)]
mod tests {
    use pb_core::{FrameData, Rgba, ShapeKind, Style, Text};
    use vello::kurbo::Rect;

    use super::*;

    fn data(kind: ObjectKind, style: Style) -> ObjectData {
        ObjectData {
            kind,
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            rotation: 0.0,
            style,
            tags: Default::default(),
            link: None,
        }
    }

    #[test]
    #[ignore = "timing, run by hand"]
    fn big_board_scene_timing() {
        use pb_core::{Command, Sticky};
        const N: usize = 100_000;
        let mut doc = Document::new();
        let items = (0..N)
            .map(|i| {
                let (x, y) = ((i % 316) as f64 * 220.0, (i / 316) as f64 * 220.0);
                let mut d = data(
                    ObjectKind::Sticky(Sticky {
                        text: format!("Note {i}").into(),
                        spans: Default::default(),
                    }),
                    Style::default(),
                );
                d.bounds = Rect::new(x, y, x + 200.0, y + 200.0);
                d.into()
            })
            .collect();
        doc.apply(Command::Create(items)).unwrap();
        let theme = Theme::light();
        let (overlay, filter, votes) = (
            Overlay::default(),
            pb_core::Filter::default(),
            HashMap::new(),
        );
        let assets = doc.assets().clone();
        let mut text = TextRenderer::new();
        let mut images = ImageCache::default();
        let mut scene = Scene::new();
        let viewport = Size::new(1600.0, 900.0);
        for (label, zoom) in [
            ("100%", 1.0),
            ("25%", 0.25),
            ("8%", 0.08),
            ("whole board", 0.02),
        ] {
            let camera = Camera::new(vello::kurbo::Point::ZERO, zoom);
            for pass in 0..2 {
                let view = View {
                    camera: &camera,
                    viewport,
                    scale: 1.0,
                    grid: GridStyle::Dots,
                    theme: &theme,
                    document: &doc,
                    assets: &assets,
                    overrides: &[],
                    hidden: &[],
                    previews: &[],
                    overlay: &overlay,
                    filter: &filter,
                    me: "me",
                    votes: &votes,
                    plain: false,
                    only: None,
                    frame_titles: true,
                };
                let t = std::time::Instant::now();
                build_scene(&mut scene, &view, &mut text, &mut images);
                let shown = doc.objects_in(camera.visible_world_rect(viewport)).len();
                eprintln!("{label} pass {pass}: {shown} shown, {:?}", t.elapsed());
            }
        }
    }

    #[test]
    fn dark_board_lightens_dark_lines_on_it_and_darkens_white_frames() {
        let text = ObjectKind::Text(Text {
            text: "hi".into(),
            spans: Default::default(),
            size: 16.0,
        });
        let dark_text = Style {
            fill: Rgba(0),
            stroke: Rgba::rgb(0x1F, 0x29, 0x37),
            ..Style::default()
        };
        let light = for_dark(&data(text.clone(), dark_text)).unwrap();
        let [r, g, b, a] = light.style.stroke.to_bytes();
        assert!(r > 0xC0 && g > 0xC0 && b > 0xC0 && a == 0xFF);
        // A chosen color, or a shape's dark text on a fill of its own, stays.
        let red = Style {
            stroke: Rgba::rgb(0xD0, 0x20, 0x20),
            ..dark_text
        };
        assert!(for_dark(&data(text.clone(), red)).is_none());
        let filled = Style {
            fill: Rgba::rgb(0xFF, 0xF1, 0x76),
            ..dark_text
        };
        assert!(for_dark(&data(ShapeKind::Rectangle.into(), filled)).is_none());
        // A text object never paints its fill, so its text turns light whatever the fill.
        assert!(for_dark(&data(text, filled)).is_some());
        let frame = ObjectKind::Frame(FrameData { title: "".into() });
        let white = Style {
            fill: Rgba::rgb(0xFF, 0xFF, 0xFF),
            ..Style::default()
        };
        assert_eq!(
            for_dark(&data(frame, white)).unwrap().style.fill,
            DARK_FRAME
        );
    }
}
