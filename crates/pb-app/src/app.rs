use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pb_core::{
    Command, Document, Link, Object, ObjectData, ObjectId, ObjectKind, Rgba, ShapeKind, Sticky,
    Style, Text, ZOrder,
};
use pb_geom::Camera;
use pb_geom::kurbo::{Point, Size, Vec2};
use pb_platform::Instant;
use pb_platform::{clipboard, dialog, handwriting};
use pb_render::vello::Scene;
use pb_render::{
    Gpu, GridStyle, ImageCache, Overlay, TextRenderer, Theme, View, build_scene, fitted_font_size,
};
use pb_tools::{
    Align, Axis, Clipboard, Crop, CursorHint, Pointer, ToolKind, Tools, math, rotation_knob,
    selection_frame, unit_frame,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, Touch, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::editing::{self, KeyOutcome, NEW_TEXT_SIZE, TextEdit};
use crate::export;
use crate::i18n::{tr, trf};
use crate::navigation::Navigation;
use crate::pictures::{self, Picture};
use crate::session::Session;
use crate::talktrack::{Playback, Recording};
use crate::ui::{
    Action, CommentView, ContextMenuInfo, ExportInfo, LayersView, LinkInfo, Minimap, Model,
    PlaybackView, STICKY_COLORS, SearchInfo, SelectionInfo, ShapeText, TableInfo, TextFormat,
    ThreadView, Ui, VersionBanner, VersionItem, VotingView,
};

/// A saved version shown instead of the board.
struct VersionView {
    id: Arc<str>,
    /// The board as it was.
    doc: Document,
    /// Objects changed or removed since, where they were (world).
    highlights: Vec<pb_geom::Frame>,
    /// How much changed since, for people.
    summary: String,
}

pub enum UserEvent {
    /// GPU initialization finished (asynchronous on the web).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    GpuReady(Box<Gpu>),
    /// Handwriting recognition finished on its background thread.
    Recognized {
        request: u64,
        result: Result<String, String>,
    },
    /// A web page card's page was fetched and read on a background thread.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Previewed {
        id: ObjectId,
        url: String,
        /// The card's title before the fetch: kept unless it is still the address.
        replace_title: bool,
        result: Result<pb_io::preview::Preview, String>,
    },
    /// A background save finished; the session takes the outcome in on the next tick.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Saved,
}

/// What recognized handwriting is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recognition {
    /// Replace the strokes with typed text.
    ToText,
    /// Solve the written expression and write the answer next to it.
    Calculate,
}

/// A recognition request waiting for its result.
struct PendingRecognition {
    request: u64,
    purpose: Recognition,
    /// The selected strokes (or groups of them).
    ids: Vec<ObjectId>,
    bounds: pb_geom::kurbo::Rect,
    color: Rgba,
}

/// Two presses closer than this in time and distance (logical pixels) are a double click.
const DOUBLE_CLICK_TIME: pb_platform::Duration = pb_platform::Duration::from_millis(450);
const DOUBLE_CLICK_DISTANCE: f64 = 5.0;
/// Space kept around a frame shown from the frames menu, in logical pixels.
const FRAME_MARGIN: f64 = 80.0;

/// Recognized text is sized so its letters are about as tall as the handwriting.
const HANDWRITING_TO_FONT_SIZE: f64 = 1.0 / 0.9;
/// Space between an expression and its answer, relative to the font size.
const ANSWER_GAP: f64 = 0.4;

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    scene: Scene,
    theme: Theme,
    camera: Camera,
    grid: GridStyle,
    navigation: Navigation,
    document: Document,
    tools: Tools,
    clipboard: Clipboard,
    session: Session,
    ui: Option<Ui>,
    /// A left-button drag started on the canvas: its events go to the tools even when the
    /// pointer passes over the UI.
    canvas_drag: bool,
    /// A style slider is being dragged, and its changes form one undo step.
    style_drag: bool,
    /// Pressure of a pen touching the screen or tablet; `None` for a mouse.
    pen_pressure: Option<f64>,
    /// The pen or finger acting as the mouse on the canvas, while it touches the screen.
    touch: Option<u64>,
    text: TextRenderer,
    images: ImageCache,
    /// Files dropped on the window since the last pass of the event loop; dropping several
    /// files sends one event each, and they are placed together.
    dropped: Vec<PathBuf>,
    /// The text being typed into, if any.
    editing: Option<TextEdit>,
    /// The image being cropped, if any.
    cropping: Option<Crop>,
    /// Color of new sticky notes.
    sticky_color: Rgba,
    /// Whether the minimap is shown.
    show_minimap: bool,
    /// Objects it does not let through are faded. Not saved with the board.
    filter: pb_core::Filter,
    /// Who reactions and comments are by.
    me: Arc<str>,
    /// The structure tool picked last, which the toolbar's shared button offers.
    last_structure: ToolKind,
    /// The comment thread shown, or a new one being started.
    thread_panel: Option<ThreadPanel>,
    /// Resolved threads keep their pins.
    show_resolved: bool,
    /// The object whose link target is being picked on the board, by clicking it.
    picking_link: Option<ObjectId>,
    /// Searching the board, while the search bar is open.
    search: Option<Search>,
    /// The right-click menu, where it was opened: on screen (logical pixels) and on the
    /// board.
    context_menu: Option<(Point, Point)>,
    /// The keyboard shortcuts window is open.
    show_shortcuts: bool,
    /// The template gallery is open.
    show_templates: bool,
    /// Presenting the board frame by frame.
    presentation: Option<Presentation>,
    /// In focus mode (the area is the document's): where the camera was before.
    focus_saved: Option<Camera>,
    /// The layers panel is open.
    show_layers: bool,
    /// A talktrack being recorded.
    recording: Option<Recording>,
    /// A talktrack being played back.
    playback: Option<Playback>,
    /// The countdown timer, while shown.
    timer: Option<Timer>,
    /// The voting results window is open.
    show_results: bool,
    /// The camera gliding to a new place.
    camera_glide: Option<Glide>,
    /// The window for making a diagram from text is open, with what was wrong with the
    /// last try.
    diagram: Option<Option<String>>,
    /// The export window is open.
    show_export: bool,
    /// A saved version being looked at instead of the board.
    version_view: Option<VersionView>,
    /// The pen's or marker's options show, for the tool they were opened for.
    ink_panel: bool,
    ink_panel_for: Option<ToolKind>,
    /// What the minimap shows, by the board's revision, theme and hidden objects shown it
    /// was made for.
    #[allow(clippy::type_complexity)]
    minimap_marks: std::cell::RefCell<
        Option<(
            (u64, bool, bool),
            pb_geom::kurbo::Rect,
            Vec<(pb_geom::kurbo::Rect, Rgba)>,
        )>,
    >,
    /// The UI's size relative to normal, and whether high contrast is on.
    ui_scale: f32,
    high_contrast: bool,
    /// The search field should take the keyboard on the next frame.
    focus_search: bool,
    /// Time and screen position of the last left press, to detect double clicks.
    last_press: Option<(Instant, Point)>,
    recognition: Option<PendingRecognition>,
    next_request: u64,
    shown_title: String,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        // A board file named on the command line (a double click on it) opens instead.
        #[cfg(not(target_arch = "wasm32"))]
        let file = std::env::args_os().nth(1).map(PathBuf::from);
        #[cfg(target_arch = "wasm32")]
        let file = None;
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let (mut session, document) = Session::start(file);
        // Background saves wake the app when done (the web saves right away).
        #[cfg(not(target_arch = "wasm32"))]
        {
            let wake = proxy.clone();
            session.set_waker(move || {
                let _ = wake.send_event(UserEvent::Saved);
            });
        }
        Self {
            proxy,
            window: None,
            gpu: None,
            scene: Scene::new(),
            theme: Theme::light(),
            camera: Camera::default(),
            grid: GridStyle::default(),
            navigation: Navigation::default(),
            document,
            tools: Tools::default(),
            clipboard: Clipboard::default(),
            session,
            ui: None,
            canvas_drag: false,
            style_drag: false,
            pen_pressure: None,
            touch: None,
            text: TextRenderer::new(),
            images: ImageCache::default(),
            dropped: Vec::new(),
            editing: None,
            cropping: None,
            sticky_color: STICKY_COLORS[0],
            show_minimap: true,
            filter: pb_core::Filter::default(),
            me: pb_platform::user_name().into(),
            last_structure: ToolKind::Frame,
            thread_panel: None,
            show_resolved: false,
            picking_link: None,
            search: None,
            focus_search: false,
            context_menu: None,
            show_shortcuts: false,
            show_templates: false,
            presentation: None,
            focus_saved: None,
            show_layers: false,
            recording: None,
            playback: None,
            camera_glide: None,
            timer: None,
            show_results: false,
            diagram: None,
            show_export: false,
            version_view: None,
            ink_panel: true,
            ink_panel_for: None,
            minimap_marks: std::cell::RefCell::new(None),
            ui_scale: 1.0,
            high_contrast: false,
            last_press: None,
            recognition: None,
            next_request: 0,
            shown_title: String::new(),
        }
    }

    fn scale(&self) -> f64 {
        self.window.as_ref().map_or(1.0, |w| w.scale_factor())
    }

    fn viewport(&self) -> Size {
        self.window.as_ref().map_or(Size::ZERO, |w| {
            let size = w.inner_size().to_logical::<f64>(w.scale_factor());
            Size::new(size.width, size.height)
        })
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if self.gpu.is_none() {
            return;
        }
        // A read-only board can be looked at and commented on, nothing more.
        if self.document.read_only()
            && !matches!(self.tools.active(), ToolKind::Select | ToolKind::Comment)
        {
            self.tools.cancel();
            self.tools.set_active(ToolKind::Select);
        }
        let scale = window.scale_factor();
        let size = window.inner_size().to_logical::<f64>(scale);
        let mut overrides = self.tools.overrides(&self.document);
        let mut previews = self.tools.previews(&self.document);
        if let Some(edit) = &mut self.editing {
            let data = edit.data(self.text.engine());
            match edit.target {
                editing::Target::Existing(id) => overrides.push(Object {
                    id,
                    data,
                    parent: self.document.parent(id),
                    locked: false,
                    hidden: false,
                }),
                editing::Target::New => previews.push(data),
            }
        }
        if let Some(glide) = &self.camera_glide {
            let viewport = Size::new(size.width, size.height);
            let (center, zoom, done) = glide.at(Instant::now());
            self.camera.look_at(center, zoom, viewport);
            if done {
                self.camera_glide = None;
            } else {
                window.request_redraw();
            }
        }
        let now = Instant::now();
        if let Some(playback) = &self.playback {
            if let Some((center, zoom, _)) = playback.view(now) {
                self.camera
                    .look_at(center, zoom, Size::new(size.width, size.height));
            }
            if !playback.is_paused() && !playback.is_over(now) {
                window.request_redraw();
            }
        }
        let mut overlay = overlay(&self.document, &self.tools, &overrides, self.camera.zoom());
        overlay.link_badges = self.link_badges().into_iter().map(|(p, _)| p).collect();
        overlay.comment_pins = self
            .comment_pins()
            .into_iter()
            .map(|(p, id, resolved)| {
                let open = self
                    .thread_panel
                    .as_ref()
                    .is_some_and(|t| t.thread.as_deref() == Some(&*id));
                (p, resolved, open)
            })
            .collect();
        self.refresh_search();
        if let Some(search) = &self.search {
            overlay.search_hits = search.hits.iter().map(|h| h.area).collect();
            overlay.search_current = search.hits.get(search.current).map(|h| h.area);
        }
        if let Some(edit) = &mut self.editing {
            let engine = self.text.engine();
            // The edit box replaces the selection frame and handles.
            overlay.selection = None;
            overlay.selected.clear();
            overlay.show_handles = false;
            overlay.rotate_knob = None;
            overlay.quick_dots.clear();
            overlay.text_edit = Some(edit.overlay(engine, Instant::now()));
            let area = edit.ime_area(engine);
            let a = self.camera.world_to_screen(area.origin());
            let b = self.camera.world_to_screen(Point::new(area.x1, area.y1));
            window.set_ime_cursor_area(
                winit::dpi::LogicalPosition::new(a.x, a.y),
                winit::dpi::LogicalSize::new((b.x - a.x).max(1.0), (b.y - a.y).max(1.0)),
            );
        }
        if let Some(presentation) = &mut self.presentation {
            // The laser's trail fades away.
            let now = Instant::now();
            presentation
                .trail
                .retain(|(_, at)| now.duration_since(*at) < LASER_TRAIL);
            if !presentation.trail.is_empty() {
                window.request_redraw();
            }
        }
        if let Some(presentation) = &self.presentation {
            let now = Instant::now();
            let laser = presentation
                .trail
                .iter()
                .map(|&(p, at)| {
                    let age = now.duration_since(at).as_secs_f64() / LASER_TRAIL.as_secs_f64();
                    (p, age)
                })
                .collect();
            // Only the frame shown: no selection, pins or badges.
            overlay = Overlay {
                laser,
                spotlight: presentation
                    .frames
                    .get(presentation.index)
                    .and_then(|&id| self.document.get(id))
                    .map(|o| o.data.bounds),
                ..Overlay::default()
            };
        }
        if let Some(playback) = &self.playback {
            // Only the board and where the presenter pointed.
            let pointer = playback.view(now).and_then(|(_, _, p)| p);
            overlay = Overlay {
                laser: pointer.map(|p| vec![(p, 0.0)]).unwrap_or_default(),
                ..Overlay::default()
            };
        }
        let mut hidden = self.tools.hidden().to_vec();
        if let Some(crop) = &self.cropping {
            hidden.push(crop.id);
            previews.extend(crop.previews());
            overlay.selection = Some(crop.area_frame());
            overlay.selected.clear();
            overlay.show_handles = true;
            overlay.rotate_knob = None;
            overlay.quick_dots.clear();
            overlay.hover = None;
        }
        let mut votes = self.vote_badges();
        let mut board = &self.document;
        let (mut overrides, mut hidden, mut previews) = (overrides, hidden, previews);
        if let Some(view) = &self.version_view {
            // The version as it was, what changed since marked, nothing to edit.
            board = &view.doc;
            overrides.clear();
            hidden.clear();
            previews.clear();
            votes.clear();
            overlay = Overlay {
                marked: view.highlights.clone(),
                ..Overlay::default()
            };
        }
        build_scene(
            &mut self.scene,
            &View {
                camera: &self.camera,
                viewport: Size::new(size.width, size.height),
                scale,
                grid: self.grid,
                theme: &self.theme,
                document: board,
                assets: board.assets(),
                overrides: &overrides,
                hidden: &hidden,
                previews: &previews,
                overlay: &overlay,
                filter: &self.filter,
                me: &self.me,
                votes: &votes,
                plain: false,
                only: None,
                frame_titles: true,
            },
            &mut self.text,
            &mut self.images,
        );

        let board = self.session.name();
        let frames = self
            .document
            .objects()
            .iter()
            .filter_map(|o| match &o.data.kind {
                ObjectKind::Frame(frame) => Some((o.id, frame.title.to_string())),
                _ => None,
            })
            .collect();
        if matches!(
            self.tools.active(),
            ToolKind::Frame
                | ToolKind::Table
                | ToolKind::Kanban
                | ToolKind::MindMap
                | ToolKind::Doc
        ) {
            self.last_structure = self.tools.active();
        }
        // The pen's options open whenever a pen or marker is picked (button or key).
        if self.ink_panel_for != Some(self.tools.active()) {
            self.ink_panel_for = Some(self.tools.active());
            self.ink_panel = true;
        }
        let model = Model {
            board: &board,
            frames,
            tool: self.tools.active(),
            last_shape: self.tools.last_shape(),
            last_structure: self.last_structure,
            ink: self
                .tools
                .ink_style(self.tools.active() == ToolKind::Marker),
            ink_panel: self.ink_panel,
            sticky_color: self.sticky_color,
            connector_route: self.tools.connector_route(),
            can_undo: self.document.can_undo(),
            can_redo: self.document.can_redo(),
            zoom: self.camera.zoom(),
            grid: self.grid,
            snap: self.tools.snap_to_grid(),
            selection: self.selection_info(),
            text_format: self.text_format(),
            minimap: self.minimap(),
            tags: self.tag_counts(),
            fill_colors: self.fill_colors(),
            filter: &self.filter,
            context_menu: self
                .context_menu
                .map(|(screen, _)| self.context_menu_info(screen)),
            show_shortcuts: self.show_shortcuts,
            hidden: (self.document.hidden_count(), self.document.shows_hidden()),
            read_only: self.document.read_only(),
            layers: self.layers_view(),
            recording: self
                .recording
                .as_ref()
                .map(|r| (r.seconds(now), r.has_sound())),
            playback: self.playback.as_ref().map(|p| PlaybackView {
                title: p.track.title.to_string(),
                position: p.position(now),
                length: p.length(),
                paused: p.is_paused(),
            }),
            talktracks: self
                .document
                .talktracks()
                .iter()
                .map(|t| (t.id.clone(), t.title.to_string(), t.duration))
                .collect(),
            diagram: self.diagram.clone(),
            versions: {
                let now = pb_platform::now_ms();
                self.document
                    .versions()
                    .iter()
                    .rev()
                    .map(|v| VersionItem {
                        id: v.id.clone(),
                        name: v.name.to_string(),
                        when: time_ago(now, v.at_ms),
                        author: v.author.to_string(),
                    })
                    .collect()
            },
            version_view: self.version_view.as_ref().and_then(|view| {
                let version = self.document.version(&view.id)?;
                Some(VersionBanner {
                    name: version.name.to_string(),
                    when: time_ago(pb_platform::now_ms(), version.at_ms),
                    author: version.author.to_string(),
                    summary: view.summary.clone(),
                })
            }),
            dark: self.theme.dark,
            ui_scale: self.ui_scale,
            high_contrast: self.high_contrast,
            export: self.show_export.then(|| ExportInfo {
                has_selection: !self.tools.selection().is_empty(),
            }),
            show_templates: self.show_templates,
            focus: self.focus_saved.is_some(),
            laser: self.presentation.as_ref().is_some_and(|p| p.laser),
            presentation: self.presentation.as_ref().map(|p| {
                let title = p
                    .frames
                    .get(p.index)
                    .and_then(|&id| match self.document.get(id).map(|o| &o.data.kind) {
                        Some(ObjectKind::Frame(f)) => Some(f.title.to_string()),
                        _ => None,
                    })
                    .unwrap_or_default();
                (p.index, p.frames.len(), title)
            }),
            timer: self.timer.as_ref().map(|t| t.view(Instant::now())),
            voting: self.document.voting().map(|v| VotingView {
                open: v.open,
                limit: v.limit,
                left: v.limit.saturating_sub(v.votes_of(&self.me)),
            }),
            results: self.show_results.then(|| self.vote_results()),
            can_present: self
                .document
                .objects()
                .iter()
                .any(|o| o.data.kind.is_frame()),
            thread: self.thread_view(),
            threads: self.thread_list(),
            show_resolved: self.show_resolved,
            search: self.search.as_ref().map(|s| SearchInfo {
                count: s.hits.len(),
                current: s.current,
                has_query: !s.query.trim().is_empty(),
                focus: self.focus_search,
            }),
        };
        self.focus_search = false;
        let actions = match &mut self.ui {
            Some(ui) => ui.run(&window, &model),
            None => Vec::new(),
        };
        if let Some(gpu) = &mut self.gpu {
            let ui = &mut self.ui;
            gpu.render(&self.scene, self.theme.background, |target| {
                if let Some(ui) = ui {
                    ui.paint(target);
                }
            });
        }

        let title = format!("{board} — PaintBoard");
        if self.shown_title != title {
            window.set_title(&title);
            self.shown_title = title;
        }

        if !actions.is_empty() {
            for action in actions {
                self.perform(action);
            }
            self.request_redraw();
        }
    }

    /// The minimap's contents, when it is shown and the board has something on it.
    fn minimap(&self) -> Option<Minimap> {
        if !self.show_minimap {
            return None;
        }
        let key = (
            self.document.revision(),
            self.theme.dark,
            self.document.shows_hidden(),
        );
        let mut cache = self.minimap_marks.borrow_mut();
        if cache.as_ref().is_none_or(|(k, ..)| *k != key) {
            *cache = self
                .minimap_content()
                .map(|(content, marks)| (key, content, marks));
        }
        let (_, content, marks) = cache.as_ref()?;
        let view = self.camera.visible_world_rect(self.viewport());
        let area = content.union(view);
        let pad = area.width().max(area.height()) * 0.04;
        Some(Minimap {
            area: area.inflate(pad, pad),
            objects: marks.clone(),
            view,
        })
    }

    /// What the minimap shows of the board: the area objects cover and a box per object,
    /// or per cell of a coarse grid when there are many.
    #[allow(clippy::type_complexity)]
    fn minimap_content(&self) -> Option<(pb_geom::kurbo::Rect, Vec<(pb_geom::kurbo::Rect, Rgba)>)> {
        let objects = self.document.objects();
        // Frames under everything, like on the board.
        let shown = objects
            .iter()
            .filter(|o| o.data.kind.is_frame())
            .chain(objects.iter().filter(|o| !o.data.kind.is_frame()))
            .filter(|o| self.document.is_shown(o.id));
        let mut content = None;
        let mut marks = Vec::new();
        for o in shown {
            let data = &o.data;
            let bounds = data.frame().aabb();
            content = Some(content.map_or(bounds, |c: pb_geom::kurbo::Rect| c.union(bounds)));
            let color = match data.kind {
                ObjectKind::Shape(_)
                | ObjectKind::Sticky(_)
                | ObjectKind::Table(_)
                | ObjectKind::Kanban(_)
                    if data.style.fill.to_bytes()[3] > 0 =>
                {
                    data.style.fill
                }
                // Frames and the rest as the board shows them in either theme.
                ObjectKind::Frame(_) if self.theme.dark => Rgba::rgb(0x2A, 0x2C, 0x30),
                ObjectKind::Frame(_) => Rgba::rgb(0xFF, 0xFF, 0xFF),
                ObjectKind::Image(_) => Rgba::rgb(0xB8, 0xB8, 0xB2),
                // Lines and strokes would show as boxes.
                ObjectKind::Connector(_) | ObjectKind::Ink(_) => continue,
                _ if self.theme.dark => Rgba::rgb(0x6A, 0x6D, 0x73),
                _ => Rgba::rgb(0xC8, 0xC8, 0xC2),
            };
            marks.push((bounds, color, data.kind.is_frame()));
        }
        let content = content?;
        if marks.len() <= MINIMAP_MAX_MARKS {
            return Some((content, marks.into_iter().map(|(r, c, _)| (r, c)).collect()));
        }
        // Too many to draw one by one: frames as they are, the rest by grid cell, each
        // cell in the color of the topmost object in it.
        let side = content.width().max(content.height()) / f64::from(MINIMAP_CELLS);
        let mut cells: HashMap<(i64, i64), Rgba> = HashMap::new();
        let mut out = Vec::new();
        for (bounds, color, frame) in marks {
            if frame {
                out.push((bounds, color));
                continue;
            }
            let c = bounds.center();
            let cell = (
                ((c.x - content.x0) / side).floor() as i64,
                ((c.y - content.y0) / side).floor() as i64,
            );
            cells.insert(cell, color);
        }
        let mut cells: Vec<_> = cells.into_iter().collect();
        cells.sort_by_key(|(cell, _)| *cell);
        out.extend(cells.into_iter().map(|((cx, cy), color)| {
            let (x, y) = (content.x0 + cx as f64 * side, content.y0 + cy as f64 * side);
            (pb_geom::kurbo::Rect::new(x, y, x + side, y + side), color)
        }));
        Some((content, out))
    }

    /// What a link points at, for people.
    fn describe_link(&self, link: &Link) -> String {
        match link {
            Link::Url(url) => url.to_string(),
            Link::Object(id) => match self.document.get(*id).map(|o| &o.data.kind) {
                Some(ObjectKind::Frame(f)) if !f.title.trim().is_empty() => {
                    trf("Frame “{}”", &[&f.title])
                }
                Some(ObjectKind::Frame(_)) => tr("Untitled frame").into(),
                Some(kind) => match kind.text().filter(|t| !t.trim().is_empty()) {
                    Some(text) => {
                        let short: String = text.chars().take(30).collect();
                        format!("“{}”", short.trim())
                    }
                    None => tr("An object on the board").into(),
                },
                None => tr("A deleted object").into(),
            },
        }
    }

    /// The board's frames with their titles, back to front, except `skip`.
    fn frame_titles(&self, skip: Option<ObjectId>) -> Vec<(ObjectId, String)> {
        self.document
            .objects()
            .iter()
            .filter(|o| Some(o.id) != skip)
            .filter_map(|o| match &o.data.kind {
                ObjectKind::Frame(frame) => Some((o.id, frame.title.to_string())),
                _ => None,
            })
            .collect()
    }

    /// The board's tags with how many objects carry each.
    fn tag_counts(&self) -> Vec<(pb_core::Tag, usize)> {
        let objects = self.document.objects();
        self.document
            .tags()
            .iter()
            .map(|tag| {
                let count = objects
                    .iter()
                    .filter(|o| o.data.tags.contains(&tag.id))
                    .count();
                (tag.clone(), count)
            })
            .collect()
    }

    /// Fill colors of sticky notes and shapes on the board, most used first, for the filter
    /// menu.
    fn fill_colors(&self) -> Vec<Rgba> {
        let mut counts: Vec<(Rgba, usize)> = Vec::new();
        for o in self.document.objects() {
            let data = &o.data;
            if !matches!(data.kind, ObjectKind::Sticky(_) | ObjectKind::Shape(_))
                || data.style.fill.to_bytes()[3] == 0
            {
                continue;
            }
            match counts.iter_mut().find(|(c, _)| *c == data.style.fill) {
                Some((_, n)) => *n += 1,
                None => counts.push((data.style.fill, 1)),
            }
        }
        counts.sort_by_key(|c| std::cmp::Reverse(c.1));
        counts.into_iter().take(16).map(|(c, _)| c).collect()
    }

    /// Unlocked selected objects, groups expanded.
    fn selected_leaves(&self) -> Vec<&pb_core::Object> {
        self.tools
            .selection()
            .ids()
            .iter()
            .filter(|&&id| !self.document.is_locked(id))
            .flat_map(|&id| self.document.leaves(id))
            .collect()
    }

    /// Puts tag `id` on the unlocked selection, or takes it off if all of it has the tag.
    fn toggle_tag(&mut self, id: Arc<str>) {
        let leaves = self.selected_leaves();
        if leaves.is_empty() {
            return;
        }
        let all = leaves.iter().all(|o| o.data.tags.contains(&id));
        let ids = leaves.iter().map(|o| o.id).collect();
        let (add, remove) = if all {
            (Vec::new(), vec![id])
        } else {
            (vec![id], Vec::new())
        };
        self.apply(Command::SetTags { ids, add, remove });
    }

    /// Puts the tag called `name` on the selection, adding it to the board if there is no
    /// such tag yet; one undo step.
    fn new_tag(&mut self, name: String) {
        let existing = self
            .document
            .tags()
            .iter()
            .find(|t| t.name.to_lowercase() == name.to_lowercase())
            .map(|t| t.id.clone());
        self.document.begin_undo_group();
        let id = match existing {
            Some(id) => id,
            None => {
                let colors = pb_core::Tag::COLORS;
                let tag = pb_core::Tag {
                    id: self.document.new_line_id().into(),
                    name: name.into(),
                    color: colors[self.document.tags().len() % colors.len()],
                };
                let id = tag.id.clone();
                self.apply(Command::DefineTag(tag));
                id
            }
        };
        let ids: Vec<_> = self.selected_leaves().iter().map(|o| o.id).collect();
        if !ids.is_empty() {
            self.apply(Command::SetTags {
                ids,
                add: vec![id],
                remove: Vec::new(),
            });
        }
        self.document.end_undo_group();
    }

    /// What the formatting bar needs while text that takes styles is edited.
    fn text_format(&mut self) -> Option<TextFormat> {
        let edit = self.editing.as_mut()?;
        let marks = edit.marks()?;
        let area = edit.frame(self.text.engine()).aabb();
        let a = self.camera.world_to_screen(area.origin());
        let b = self.camera.world_to_screen(Point::new(area.x1, area.y1));
        Some(TextFormat {
            screen: pb_geom::kurbo::Rect::from_points(a, b),
            marks,
        })
    }

    /// What the selection toolbar needs; nothing while a gesture is in progress.
    fn selection_info(&mut self) -> Option<SelectionInfo> {
        if self.editing.is_some()
            || self.cropping.is_some()
            || self.tools.is_busy()
            || self.tools.active() != ToolKind::Select
        {
            return None;
        }
        let selection = self.tools.selection();
        let ids = selection.ids();
        let bounds = selection.frame(&self.document)?.aabb();
        let leaves: Vec<_> = ids
            .iter()
            .flat_map(|&id| self.document.leaves(id))
            .collect();
        let first = leaves.first()?;
        let shape_text = leaves.iter().find_map(|o| match &o.data.kind {
            ObjectKind::Table(_) => {
                let style = o.data.style;
                Some(ShapeText {
                    color: style.text_color(),
                    size: style.font_size.unwrap_or(pb_core::Table::FONT_SIZE),
                    auto: style.font_size.is_none(),
                    align: style.text_align,
                    valign: style.text_valign,
                })
            }
            ObjectKind::Shape(shape) => {
                let style = o.data.style;
                let text_box = o.data.text_box()?;
                let size =
                    fitted_font_size(self.text.engine(), &shape.text, &shape.spans, text_box);
                Some(ShapeText {
                    color: style.text_color(),
                    size,
                    auto: style.font_size.is_none(),
                    align: style.text_align,
                    valign: style.text_valign,
                })
            }
            _ => None,
        });
        let corner = |x, y| self.camera.world_to_screen(Point::new(x, y));
        let mut screen = pb_geom::kurbo::Rect::from_points(
            corner(bounds.x0, bounds.y0),
            corner(bounds.x1, bounds.y1),
        );
        // The toolbar keeps clear of quick create's dots.
        if !self
            .tools
            .quick_dots(&self.document, self.camera.zoom())
            .is_empty()
        {
            let reach = pb_tools::quick::DOT_OFFSET + pb_tools::quick::DOT_RADIUS;
            screen = screen.inflate(reach, reach);
        }
        Some(SelectionInfo {
            screen,
            style: first.data.style,
            has_group: ids.iter().any(|&id| self.document.group(id).is_some()),
            has_fill: leaves.iter().any(|o| {
                matches!(
                    o.data.kind,
                    ObjectKind::Shape(_)
                        | ObjectKind::Sticky(_)
                        | ObjectKind::Frame(_)
                        | ObjectKind::Table(_)
                        | ObjectKind::Kanban(_)
                        | ObjectKind::MindMap(_)
                )
            }),
            has_width: leaves.iter().any(|o| {
                matches!(
                    o.data.kind,
                    ObjectKind::Shape(_) | ObjectKind::Ink(_) | ObjectKind::Connector(_)
                )
            }),
            connector: leaves.iter().find_map(|o| match &o.data.kind {
                ObjectKind::Connector(c) => Some(c.clone()),
                _ => None,
            }),
            has_stroke: leaves
                .iter()
                .any(|o| !matches!(o.data.kind, ObjectKind::Image(_) | ObjectKind::Frame(_))),
            stroke_label: if leaves.iter().any(|o| {
                matches!(
                    o.data.kind,
                    ObjectKind::Shape(_) | ObjectKind::Kanban(_) | ObjectKind::MindMap(_)
                )
            }) {
                "Outline"
            } else if leaves
                .iter()
                .any(|o| matches!(o.data.kind, ObjectKind::Ink(_) | ObjectKind::Connector(_)))
            {
                "Color"
            } else {
                "Text color"
            },
            handwriting: (handwriting::is_available()
                && leaves.iter().all(|o| o.data.kind.is_ink()))
            .then_some(self.recognition.is_some()),
            shape_text,
            table: match (ids, leaves.first().map(|o| &o.data.kind)) {
                ([_], Some(ObjectKind::Table(t))) if leaves.len() == 1 => Some(TableInfo {
                    rows: t.rows.len(),
                    cols: t.cols.len(),
                    cells: self.tools.table_cells(&self.document).map(|(_, r)| r),
                    can_merge: self.tools.can_merge_cells(&self.document, true),
                    can_split: self.tools.can_merge_cells(&self.document, false),
                }),
                _ => None,
            },
            mind_node: self.tools.node_state(&self.document),
            kanban: self
                .tools
                .kanban_columns(&self.document)
                .map(|n| (n, self.tools.selected_card(&self.document).is_some())),
            tags_on_all: self
                .document
                .tags()
                .iter()
                .filter(|t| leaves.iter().all(|o| o.data.tags.contains(&t.id)))
                .map(|t| t.id.clone())
                .collect(),
            tags_on_some: self
                .document
                .tags()
                .iter()
                .filter(|t| leaves.iter().any(|o| o.data.tags.contains(&t.id)))
                .map(|t| t.id.clone())
                .collect(),
            board_tags: self.document.tags().to_vec(),
            my_reactions: match ids {
                [id] if self.document.get(*id).is_some() => Some(
                    self.document
                        .reactions(*id)
                        .iter()
                        .filter(|(_, who)| who.contains(&self.me))
                        .map(|(r, _)| *r)
                        .collect(),
                ),
                _ => None,
            },
            link: match ids {
                [id] if self.document.get(*id).is_some()
                    && !selection.has_locked(&self.document) =>
                {
                    Some(LinkInfo {
                        current: self.document.get(*id).and_then(|o| {
                            o.data.link.as_ref().map(|link| self.describe_link(link))
                        }),
                        frames: self.frame_titles(Some(*id)),
                        picking: self.picking_link.is_some(),
                    })
                }
                _ => None,
            },
            is_embed: matches!(
                ids,
                [id] if matches!(
                    self.document.get(*id).map(|o| &o.data.kind),
                    Some(ObjectKind::Embed(_))
                )
            ),
            can_crop: matches!(ids, [id] if Crop::new(&self.document, *id).is_some()),
            several: ids.len() > 1,
            locked: selection.has_locked(&self.document),
            hidden: ids.iter().all(|&id| self.document.is_hidden(id)),
            read_only: self.document.read_only(),
            layers: self.layer_choices(),
            layer: {
                let mut layers = ids.iter().map(|&id| &self.document.layer_of(id).id);
                let first = layers.next().cloned();
                first.filter(|f| layers.all(|l| l == f))
            },
        })
    }

    /// Carries out a UI action.
    fn perform(&mut self, action: Action) {
        let viewport = self.viewport();
        match action {
            Action::Tool(tool) => {
                self.finish_editing();
                // The active pen's button opens and closes its options.
                if tool == self.tools.active() && tool.draws_ink() {
                    self.ink_panel = !self.ink_panel;
                }
                self.tools.set_active(tool);
            }
            Action::InkStyle { marker, style } => self.tools.set_ink_style(marker, style),
            Action::StickyColor(color) => self.sticky_color = color,
            Action::ConnectorRoute(route) => self.tools.set_connector_route(route),
            Action::SetConnector(change) => {
                let ids: Vec<_> = self
                    .tools
                    .selection()
                    .ids()
                    .iter()
                    .filter(|&&id| !self.document.is_locked(id))
                    .flat_map(|&id| self.document.leaves(id))
                    .filter(|o| matches!(o.data.kind, ObjectKind::Connector(_)))
                    .map(|o| o.id)
                    .collect();
                if !ids.is_empty() {
                    self.apply(Command::SetConnector { ids, change });
                }
            }
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::NewBoard => self.new_board(),
            Action::Open => self.open_board(),
            Action::Save => self.session.save(&self.document),
            Action::SaveAs => {
                self.session.save_as(&self.document);
            }
            Action::ZoomIn => self.navigation.zoom_in(&mut self.camera, viewport),
            Action::ZoomOut => self.navigation.zoom_out(&mut self.camera, viewport),
            Action::ZoomReset => self.navigation.zoom_reset(&mut self.camera, viewport),
            Action::GoToFrame(id) => {
                if let Some(object) = self.document.get(id) {
                    let area = object.data.bounds;
                    // Room for the title and the floating panels around the canvas.
                    self.camera.fit(area, viewport, FRAME_MARGIN);
                    self.finish_editing();
                    self.tools.set_active(ToolKind::Select);
                    self.tools.select_only([id]);
                }
            }
            Action::NextGrid => self.grid = self.grid.next(),
            Action::ToggleSnap => self.tools.set_snap_to_grid(!self.tools.snap_to_grid()),
            Action::ToggleMinimap => self.show_minimap = !self.show_minimap,
            Action::AddTableLine { rows, after } => {
                if let Some(command) = self.tools.add_table_line(&self.document, rows, after) {
                    self.apply(command);
                }
            }
            Action::RemoveTableLines(rows) => {
                if let Some(command) = self.tools.remove_table_lines(&self.document, rows) {
                    self.apply(command);
                }
            }
            Action::Copy => {
                self.copy_selection();
            }
            Action::Cut => {
                if self.copy_selection()
                    && let Some(command) = self.tools.delete_selected(&self.document)
                {
                    self.apply(command);
                }
            }
            Action::Paste => {
                let at = self.context_menu.map(|(_, world)| world);
                self.paste_at(at);
            }
            Action::SelectAll => self.tools.select_all(&self.document),
            Action::EditText => {
                self.edit_selected_text();
            }
            Action::AddStickyHere => {
                if let Some((_, world)) = self.context_menu {
                    self.add_sticky_at(world);
                }
            }
            Action::AddTextHere => {
                if let Some((_, world)) = self.context_menu {
                    self.add_text_at(world);
                }
            }
            Action::CloseContextMenu => self.context_menu = None,
            Action::ToggleShortcuts => self.show_shortcuts = !self.show_shortcuts,
            Action::StartTimer(minutes) => {
                self.timer = Some(Timer::new(pb_platform::Duration::from_secs(minutes * 60)));
            }
            Action::PauseTimer => {
                if let Some(timer) = &mut self.timer {
                    timer.toggle(Instant::now());
                }
            }
            Action::ResetTimer => {
                if let Some(timer) = &mut self.timer {
                    *timer = Timer::new(timer.duration);
                }
            }
            Action::CloseTimer => self.timer = None,
            Action::StartVoting(limit) => {
                self.finish_editing();
                self.tools.clear_selection();
                self.show_results = false;
                self.apply(Command::StartVoting { limit });
            }
            Action::EndVoting => {
                self.apply(Command::EndVoting);
                self.show_results = true;
            }
            Action::ClearVotes => {
                self.show_results = false;
                self.apply(Command::ClearVotes);
            }
            Action::ToggleResults => self.show_results = !self.show_results,
            Action::Present => self.start_presentation(),
            Action::PresentStep(step) => self.present_step(step),
            Action::StopPresenting => self.stop_presentation(),
            Action::ToggleFocus => self.toggle_focus(),
            Action::ToggleLaser => self.toggle_laser(),
            Action::ToggleTemplates => self.show_templates = !self.show_templates,
            Action::InsertTemplate(i) => {
                self.show_templates = false;
                self.insert_template(i);
            }
            Action::OpenDiagram => {
                self.finish_editing();
                self.diagram = Some(None);
            }
            Action::CloseDiagram => self.diagram = None,
            Action::SaveVersion(name) => {
                let name = name.trim();
                let name = if name.is_empty() {
                    trf("Version {}", &[&(self.document.versions().len() + 1)])
                } else {
                    name.to_owned()
                };
                self.apply(Command::SaveVersion {
                    id: self.document.new_line_id().into(),
                    name: name.into(),
                    author: self.me.clone(),
                    at_ms: pb_platform::now_ms(),
                });
            }
            Action::OpenVersion(id) => self.open_version(id),
            Action::CloseVersion => self.version_view = None,
            Action::RestoreVersion => {
                if let Some(view) = self.version_view.take() {
                    self.tools.clear_selection();
                    self.apply(Command::RestoreVersion(view.id));
                }
            }
            Action::RemoveVersion(id) => {
                let name = self.document.version(&id).map(|v| v.name.to_string());
                let question = trf(
                    "Forget the version \"{}\"? This cannot be undone; the board itself stays as it is.",
                    &[&name.unwrap_or_default()],
                );
                if dialog::confirm(tr("Remove version"), &question) {
                    if self.version_view.as_ref().is_some_and(|v| v.id == id) {
                        self.version_view = None;
                    }
                    self.apply(Command::RemoveVersion(id));
                }
            }
            Action::ToggleLanguage => {
                use crate::i18n::Lang;
                let lang = match crate::i18n::lang() {
                    Lang::En => Lang::Ru,
                    Lang::Ru => Lang::En,
                };
                crate::i18n::set_lang(lang);
                crate::settings::save_language(lang);
                self.request_redraw();
            }
            Action::SetUiScale(scale) => {
                self.set_view(scale, self.high_contrast);
                crate::settings::save_view(self.ui_scale, self.high_contrast);
            }
            Action::ToggleHighContrast => {
                self.set_view(self.ui_scale, !self.high_contrast);
                crate::settings::save_view(self.ui_scale, self.high_contrast);
            }
            Action::ToggleTheme => {
                let dark = !self.theme.dark;
                self.set_dark(dark);
                crate::settings::save_dark_theme(dark);
            }
            Action::OpenExport => self.open_export(),
            Action::Print => self.print(),
            Action::CloseExport => self.show_export = false,
            Action::Export(settings) => {
                if self.export_picture(&settings) {
                    self.show_export = false;
                }
            }
            Action::InsertDiagram(source) => match self.insert_diagram(&source) {
                Ok(()) => self.diagram = None,
                Err(err) => self.diagram = Some(Some(err.to_string())),
            },
            Action::OpenSearch => self.open_search(),
            Action::Search(query) => self.search_for(query),
            Action::SearchStep(forward) => self.search_step(forward),
            Action::CloseSearch => self.search = None,
            Action::PostComment(text) => self.post_comment(text),
            Action::DeleteComment(comment) => {
                if let Some(thread) = self.thread_panel.as_ref().and_then(|p| p.thread.clone()) {
                    self.apply(Command::RemoveComment {
                        thread: thread.clone(),
                        comment,
                    });
                    if self.document.thread(&thread).is_none() {
                        self.thread_panel = None;
                    }
                }
            }
            Action::ResolveThread(resolved) => {
                if let Some(thread) = self.thread_panel.as_ref().and_then(|p| p.thread.clone()) {
                    self.apply(Command::SetResolved { thread, resolved });
                    if resolved && !self.show_resolved {
                        self.thread_panel = None;
                    }
                }
            }
            Action::DeleteThread => {
                if let Some(thread) = self.thread_panel.take().and_then(|p| p.thread) {
                    self.apply(Command::RemoveThread(thread));
                }
            }
            Action::CloseThread => self.thread_panel = None,
            Action::OpenThread(id) => self.open_thread(id),
            Action::ToggleShowResolved => {
                self.show_resolved = !self.show_resolved;
                let hidden = self
                    .thread_panel
                    .as_ref()
                    .and_then(|p| p.thread.as_deref())
                    .and_then(|id| self.document.thread(id))
                    .is_some_and(|t| t.resolved);
                if hidden && !self.show_resolved {
                    self.thread_panel = None;
                }
            }
            Action::SetLink(text) => {
                if let ([id], Some(link)) = (self.tools.selection().ids(), Link::from_input(&text))
                {
                    let id = *id;
                    self.apply(Command::SetLink {
                        ids: vec![id],
                        link: Some(link),
                    });
                }
            }
            Action::LinkTo(target) => {
                if let [id] = self.tools.selection().ids() {
                    let id = *id;
                    self.apply(Command::SetLink {
                        ids: vec![id],
                        link: Some(Link::Object(target)),
                    });
                }
            }
            Action::RemoveLink => {
                if let [id] = self.tools.selection().ids() {
                    let id = *id;
                    self.apply(Command::SetLink {
                        ids: vec![id],
                        link: None,
                    });
                }
            }
            Action::PickLinkTarget => {
                if let [id] = self.tools.selection().ids() {
                    self.picking_link = Some(*id);
                }
            }
            Action::CopyAnchor => {
                if let [id] = self.tools.selection().ids() {
                    pb_platform::clipboard::set_text(&Link::anchor(*id));
                }
            }
            Action::FollowLink => {
                let link = match self.tools.selection().ids() {
                    [id] => self.document.get(*id).and_then(|o| o.data.link.clone()),
                    _ => None,
                };
                if let Some(link) = link {
                    self.follow(&link);
                }
            }
            Action::React(reaction) => {
                if let [id] = self.tools.selection().ids() {
                    let id = *id;
                    let mine = self
                        .document
                        .reactions(id)
                        .iter()
                        .any(|(r, who)| *r == reaction && who.contains(&self.me));
                    self.apply(Command::React {
                        id,
                        reaction,
                        author: self.me.clone(),
                        on: !mine,
                    });
                }
            }
            Action::Arrange(None) => {
                self.apply_with(|tools, doc| pb_tools::arrange_grid(doc, tools.selection()));
            }
            Action::Arrange(Some(by)) => self.cluster(by),
            Action::FetchPreview => {
                if let [id] = self.tools.selection().ids() {
                    self.fetch_preview(*id, true);
                }
            }
            Action::ToggleTag(id) => self.toggle_tag(id),
            Action::NewTag(name) => self.new_tag(name),
            Action::RecolorTag(id) => {
                if let Some(tag) = self.document.tag(&id) {
                    let colors = pb_core::Tag::COLORS;
                    let next = colors
                        .iter()
                        .position(|&c| c == tag.color)
                        .map_or(0, |i| (i + 1) % colors.len());
                    let tag = pb_core::Tag {
                        color: colors[next],
                        ..tag.clone()
                    };
                    self.apply(Command::DefineTag(tag));
                }
            }
            Action::RemoveTag(id) => {
                self.filter.tags.retain(|t| *t != id);
                self.apply(Command::RemoveTag(id));
            }
            Action::FilterTag(id) => self.filter.toggle_tag(&id),
            Action::FilterColor(color) => self.filter.toggle_color(color),
            Action::ClearFilter => self.filter = pb_core::Filter::default(),
            Action::AddNode(child) => self.add_node(child),
            Action::ToggleNode => {
                if let Some(command) = self.tools.toggle_node(&self.document, None) {
                    self.apply(command);
                }
            }
            Action::DeleteNode => {
                if let Some(command) = self.tools.remove_selected_node(&self.document) {
                    self.apply(command);
                }
            }
            Action::AddKanbanColumn => {
                if let Some(command) = self.tools.add_kanban_column(&self.document) {
                    self.apply(command);
                }
            }
            Action::RemoveKanbanColumn => {
                if let Some(command) = self.tools.remove_kanban_column(&self.document) {
                    self.apply(command);
                }
            }
            Action::DeleteCard => {
                if let Some(command) = self.tools.remove_selected_card(&self.document) {
                    self.apply(command);
                }
            }
            Action::MergeCells(merge) => {
                if let Some(command) = self.tools.merge_cells(&self.document, merge) {
                    self.apply(command);
                }
            }
            Action::ResetBends => {
                if let Some(command) = self.tools.reset_bends_selected(&self.document) {
                    self.apply(command);
                }
            }
            Action::LookAt(at) => {
                let zoom = self.camera.zoom();
                self.camera.look_at(at, zoom, viewport);
            }
            Action::SetStyle(change) => {
                let ids = self.tools.selection().ids();
                let ids: Vec<_> = ids
                    .iter()
                    .copied()
                    .filter(|&id| !self.document.is_locked(id))
                    .collect();
                if !ids.is_empty() {
                    self.apply(Command::SetStyle { ids, change });
                }
            }
            Action::BeginStyleDrag => {
                if !self.style_drag {
                    self.document.begin_undo_group();
                    self.style_drag = true;
                }
            }
            Action::EndStyleDrag => self.end_style_drag(),
            Action::Group => self.apply_with(Tools::group_selected),
            Action::Ungroup => self.apply_with(Tools::ungroup_selected),
            Action::ToggleLock => self.apply_with(Tools::toggle_lock_selected),
            Action::ToggleHidden => self.toggle_hidden(),
            Action::ToggleLayers => self.show_layers = !self.show_layers,
            Action::StartRecording => {
                if self.recording.is_none() {
                    self.recording = Some(Recording::start(Instant::now()));
                }
            }
            Action::StopRecording => self.stop_recording(),
            Action::CancelRecording => {
                // Stops the microphone too.
                self.recording = None;
            }
            Action::PlayTalktrack(id) => self.play_talktrack(&id),
            Action::TogglePlayback => {
                if let Some(p) = &mut self.playback {
                    p.toggle(Instant::now());
                }
            }
            Action::SeekPlayback(seconds) => {
                if let Some(p) = &mut self.playback {
                    p.seek(seconds, Instant::now());
                }
            }
            Action::StopPlayback => self.stop_playback(),
            Action::RemoveTalktrack(id) => self.apply(Command::RemoveTalktrack(id)),
            Action::AddLayer => {
                let layers = self.document.layers();
                let order = layers.iter().map(|l| l.order).fold(0.0, f64::max) + 1.0;
                let id: Arc<str> = self.document.new_line_id().into();
                let layer = pb_core::Layer {
                    id: id.clone(),
                    name: trf("Layer {}", &[&(layers.len() + 1)]).into(),
                    order,
                    visible: true,
                    locked: false,
                };
                self.apply(Command::DefineLayer(layer));
                self.document.set_active_layer(id);
            }
            Action::ActivateLayer(id) => self.document.set_active_layer(id),
            Action::UpdateLayer(layer) => {
                // New objects should not land where they cannot be seen or changed.
                if layer.id == self.document.active_layer().id
                    && (!layer.visible || layer.locked)
                    && let Some(other) = self
                        .document
                        .layers()
                        .iter()
                        .rev()
                        .find(|l| l.id != layer.id && l.visible && !l.locked)
                {
                    let other = other.id.clone();
                    self.document.set_active_layer(other);
                }
                self.apply(Command::DefineLayer(layer));
                self.drop_unreachable();
            }
            Action::MoveLayer(id, up) => self.move_layer(&id, up),
            Action::RemoveLayer(id) => self.apply(Command::RemoveLayer(id)),
            Action::MoveToLayer(layer) => {
                let ids = self.tools.selection().ids().to_vec();
                if !ids.is_empty() {
                    self.apply(Command::SetLayer { ids, layer });
                }
            }
            Action::ToggleReadOnly => {
                self.finish_editing();
                self.tools.cancel();
                let on = !self.document.read_only();
                self.apply(Command::SetReadOnly(on));
            }
            Action::ShowHidden => {
                let show = !self.document.shows_hidden();
                self.document.set_show_hidden(show);
                self.drop_unreachable();
            }
            Action::Duplicate => {
                self.apply_with(|tools, doc| pb_tools::duplicate(doc, tools.selection()));
            }
            Action::Delete => self.delete_selected(),
            Action::Reorder(to) => self.reorder(to),
            Action::InsertImage => self.insert_image_files(),
            Action::Crop => {
                if let [id] = self.tools.selection().ids() {
                    self.start_crop(*id);
                }
            }
            Action::ToText => self.recognize(Recognition::ToText),
            Action::Calculate => self.recognize(Recognition::Calculate),
            Action::ToggleMark(mark) => {
                if let Some(edit) = &mut self.editing {
                    edit.toggle_mark(self.text.engine(), mark);
                }
            }
            Action::MarkColor(color) => {
                if let Some(edit) = &mut self.editing {
                    edit.set_mark_color(self.text.engine(), color);
                }
            }
        }
    }

    fn start_editing(&mut self, edit: TextEdit) {
        if self.document.read_only() {
            return;
        }
        self.finish_editing();
        self.tools.cancel();
        if let editing::Target::Existing(id) = edit.target {
            self.tools.select_only([id]);
        }
        self.editing = Some(edit);
        if let Some(window) = &self.window {
            window.set_ime_allowed(true);
        }
    }

    /// Starts cropping an image; returns false for other objects.
    fn start_crop(&mut self, id: ObjectId) -> bool {
        let Some(crop) = Crop::new(&self.document, id) else {
            return false;
        };
        self.finish_editing();
        self.tools.cancel();
        self.tools.select_only([id]);
        self.cropping = Some(crop);
        true
    }

    /// Ends cropping, keeping the new crop or (`keep` false, Escape) dropping it.
    fn finish_crop(&mut self, keep: bool) {
        if let Some(crop) = self.cropping.take()
            && keep
            && let Some(command) = crop.finish()
        {
            self.apply(command);
        }
    }

    /// Ends text editing or cropping, keeping the edit as one command.
    fn finish_editing(&mut self) {
        self.finish_crop(true);
        let Some(edit) = self.editing.take() else {
            return;
        };
        if let Some(window) = &self.window {
            window.set_ime_allowed(false);
        }
        if let Some(command) = edit.finish(self.text.engine()) {
            self.apply(command);
        }
    }

    /// Asks for files to add (images, PDF, CSV, Markdown) and places them in the middle of
    /// the view.
    fn insert_image_files(&mut self) {
        let mut extensions = pb_io::image::EXTENSIONS.to_vec();
        extensions.extend(["svg", "svgz", "pdf", "csv", "tsv", "md", "markdown", "txt"]);
        let paths = dialog::pick_open_many(tr("Images, PDF, tables, documents"), &extensions);
        self.insert_files(&paths, None);
    }

    /// Places image files at `at` (world), or in the middle of the view. Other files are
    /// skipped; unreadable images are reported.
    fn insert_files(&mut self, paths: &[PathBuf], at: Option<Point>) {
        let mut errors = Vec::new();
        let documents: Vec<(String, String)> = paths
            .iter()
            .filter(|p| is_document_file(p))
            .filter_map(|p| match pb_platform::fs::read(p) {
                Ok(bytes) => Some((
                    p.display().to_string(),
                    String::from_utf8_lossy(&bytes).replace("\r\n", "\n"),
                )),
                Err(err) => {
                    errors.push(format!("{}: {err}", p.display()));
                    None
                }
            })
            .collect();
        self.insert_documents(&documents, at);
        let zoom = self.camera.zoom();
        let tables = paths.iter().filter(|p| is_table_file(p));
        for (i, path) in tables.enumerate() {
            let rows = pb_platform::fs::read(path).map(|bytes| {
                let text = String::from_utf8_lossy(&bytes);
                if has_extension(path, "tsv") {
                    pb_io::csv::parse_tsv(&text)
                } else {
                    pb_io::csv::parse(&text)
                }
            });
            // Several tables step down and to the right, so none hides another.
            let at = at.map(|p| p + Vec2::splat(40.0 * i as f64 / zoom));
            match rows {
                Ok(rows) if rows.is_empty() => errors.push(trf("{}: no cells", &[&path.display()])),
                Ok(rows) => {
                    if let Some(note) = self.insert_table(rows, at) {
                        errors.push(format!("{}: {note}", path.display()));
                    }
                }
                Err(err) => errors.push(format!("{}: {err}", path.display())),
            }
        }
        for path in paths.iter().filter(|p| is_pdf_file(p)) {
            if let Err(err) = self.insert_pdf(path, at) {
                errors.push(format!("{}: {err}", path.display()));
            }
        }
        let pictures: Vec<_> = paths
            .iter()
            .filter(|p| pictures::is_image_file(p) || pictures::is_svg_file(p))
            .filter_map(|p| Picture::from_file(p).map_err(|e| errors.push(e)).ok())
            .collect();
        self.insert_pictures(pictures, at);
        if !errors.is_empty() {
            dialog::show_error(tr("Some files could not be added"), &errors.join("\n"));
        }
    }

    /// Adds the pages of the PDF file at `path` side by side as a group, as one undo step,
    /// and selects it. Very long files are cut, with a note.
    fn insert_pdf(&mut self, path: &std::path::Path, at: Option<Point>) -> Result<(), String> {
        let bytes = pb_platform::fs::read(path).map_err(|e| e.to_string())?;
        let count = pb_io::pdf::page_count(&bytes)?;
        let pages = pb_io::pdf::render_pages(&bytes, MAX_PDF_PAGES)?;
        let pictures: Vec<Picture> = pages
            .into_iter()
            .map(|page| Picture {
                bytes: page.png,
                // Shown at the page's own size in points: rendered at twice that, so it
                // stays sharp up close.
                width: (page.width / 2).max(1),
                height: (page.height / 2).max(1),
            })
            .collect();
        let several = pictures.len() > 1;
        self.document.begin_undo_group();
        self.insert_pictures(pictures, at);
        if several {
            let ids = self.tools.selection().ids().to_vec();
            self.apply(Command::Group(ids.clone()));
            if let Some(&first) = ids.first() {
                let group = self.document.top_level(first);
                self.tools.select_only([group]);
            }
        }
        self.document.end_undo_group();
        if count > MAX_PDF_PAGES {
            dialog::show_error(
                tr("Only the first pages were added"),
                &trf(
                    "{} has {} pages; the first {} are on the board.",
                    &[&path.display(), &count, &MAX_PDF_PAGES],
                ),
            );
        }
        Ok(())
    }

    /// Adds pictures side by side as one undo step, and selects them.
    fn insert_pictures(&mut self, pictures: Vec<Picture>, at: Option<Point>) {
        if pictures.is_empty() {
            return;
        }
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        let viewport = self.viewport();
        let visible = self.camera.visible_world_rect(viewport);
        let at = at.unwrap_or(visible.center());
        let sizes: Vec<_> = pictures.iter().map(|p| (p.width, p.height)).collect();
        let pixels_per_unit = self.camera.zoom() * self.scale();
        let bounds = pictures::place(&sizes, at, pixels_per_unit, visible);
        let items = pictures
            .into_iter()
            .zip(bounds)
            .map(|(picture, bounds)| {
                let asset = self.document.add_asset(picture.bytes);
                pictures::image_object(asset, bounds).into()
            })
            .collect();
        self.apply(Command::Create(items));
        self.request_redraw();
    }

    /// Ctrl+V: a picture or image files from the system clipboard, or else board objects
    /// copied in the app. Lands at the pointer, like Miro; copied objects cascade when it is
    /// off the window.
    fn paste(&mut self) {
        let at = self
            .navigation
            .hovered_cursor()
            .map(|c| self.camera.screen_to_world(c));
        self.paste_at(at);
    }

    /// Pastes centered on `at` (world), or where the copy came from.
    fn paste_at(&mut self, at: Option<Point>) {
        if let Some(image) = clipboard::image() {
            match Picture::from_clipboard(image) {
                Ok(picture) => self.insert_pictures(vec![picture], at),
                Err(err) => dialog::show_error(tr("Could not paste the image"), &err.to_string()),
            }
            return;
        }
        let files = clipboard::files();
        if files
            .iter()
            .any(|f| pictures::is_image_file(f) || pictures::is_svg_file(f) || is_table_file(f))
        {
            self.insert_files(&files, at);
            return;
        }
        // Cells copied from a spreadsheet arrive as tab-separated text.
        if let Some(text) = clipboard::text().filter(|t| t.contains('\t')) {
            let rows = pb_io::csv::parse_tsv(&text);
            if let Some(note) = self.insert_table(rows, at) {
                dialog::show_error(
                    tr("Pasted table"),
                    &trf("The table is too large: {}.", &[&note]),
                );
            }
            return;
        }
        // A web address copied elsewhere becomes a card for the page. Copying on the board
        // empties the system clipboard, so text there is newer than the board's own copy.
        if let Some(Link::Url(url)) = clipboard::text().and_then(|t| Link::from_input(&t))
            && !url.starts_with("mailto:")
        {
            self.insert_embed(&url, at);
            return;
        }
        for bytes in self.clipboard.assets().to_vec() {
            self.document.add_asset(bytes);
        }
        if let Some(command) = self.clipboard.paste(at) {
            self.apply(command);
        }
    }

    /// Adds a card for the web page at `url`, centered on `at` (world) or in the middle of
    /// the view, linked to the page, and selects it.
    fn insert_embed(&mut self, url: &str, at: Option<Point>) {
        let zoom = self.camera.zoom();
        let (w, h) = pb_core::Embed::SIZE;
        let center = at.unwrap_or_else(|| self.camera.visible_world_rect(self.viewport()).center());
        let data = ObjectData {
            kind: ObjectKind::Embed(pb_core::Embed::new(url)),
            bounds: pb_geom::kurbo::Rect::from_center_size(center, (w / zoom, h / zoom)),
            rotation: 0.0,
            style: pb_core::Embed::style(),
            tags: Default::default(),
            link: Some(Link::Url(url.into())),
        };
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        self.apply(Command::Create(vec![data.into()]));
        if let [id] = self.tools.selection().ids() {
            self.fetch_preview(*id, false);
        }
    }

    /// Fetches the page of web page card `id` in the background to fill in its title,
    /// description and site; its title is replaced only if it is still the address, or
    /// with `force`.
    fn fetch_preview(&mut self, id: ObjectId, force: bool) {
        let Some(ObjectKind::Embed(embed)) = self.document.get(id).map(|o| &o.data.kind) else {
            return;
        };
        let url = embed.url.to_string();
        let replace_title = force || embed.title == pb_core::Embed::new(&url).title;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                let result =
                    pb_platform::web::fetch_page(&url).map(|html| pb_io::preview::parse(&html));
                let _ = proxy.send_event(UserEvent::Previewed {
                    id,
                    url,
                    replace_title,
                    result,
                });
            });
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (url, replace_title);
    }

    /// Fills in a web page card from its fetched page, if the card is still there and
    /// still for that page.
    fn finish_preview(
        &mut self,
        id: ObjectId,
        url: &str,
        replace_title: bool,
        result: Result<pb_io::preview::Preview, String>,
    ) {
        let preview = match result {
            Ok(preview) => preview,
            Err(err) => {
                log::warn!("no preview for {url}: {err}");
                return;
            }
        };
        let Some(ObjectKind::Embed(embed)) = self.document.get(id).map(|o| &o.data.kind) else {
            return;
        };
        if &*embed.url != url {
            return;
        }
        let pick = |new: &str, old: &Arc<str>| -> Arc<str> {
            if new.is_empty() {
                old.clone()
            } else {
                new.into()
            }
        };
        let filled = pb_core::Embed {
            title: if replace_title {
                pick(&preview.title, &embed.title)
            } else {
                embed.title.clone()
            },
            description: pick(&preview.description, &embed.description),
            site: pick(&preview.site, &embed.site),
            url: embed.url.clone(),
        };
        if filled != *embed {
            self.apply(Command::SetEmbed { id, embed: filled });
            self.request_redraw();
        }
    }

    /// Copies the selection inside the app. The system clipboard is emptied, so an older
    /// picture there does not win over it on paste.
    fn copy_selection(&mut self) -> bool {
        let copied = self.clipboard.copy(&self.document, self.tools.selection());
        if copied {
            clipboard::clear();
        }
        copied
    }

    /// Tab on a sticky note: keeps it and starts the next one beside it, in view.
    /// Tab in a table cell: keeps it and goes on typing in the next cell.
    fn next_cell(&mut self, back: bool) {
        let Some((id, cell)) = self.editing.as_ref().and_then(|e| e.next_cell(back)) else {
            return;
        };
        self.finish_editing();
        let engine = self.text.engine();
        if let Some(mut edit) = TextEdit::table_cell(engine, &self.document, id, cell) {
            edit.select_all(engine);
            self.start_editing(edit);
        }
    }

    /// Starts typing a new text object at `at` (world).
    fn add_text_at(&mut self, at: Point) {
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        let size = NEW_TEXT_SIZE / self.camera.zoom();
        let color = Style::default().stroke;
        let edit = TextEdit::new_text(self.text.engine(), at, size, color);
        self.start_editing(edit);
    }

    /// Starts typing on a new sticky note centered on `at` (world).
    fn add_sticky_at(&mut self, at: Point) {
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        let side = Sticky::SIZE / self.camera.zoom();
        let bounds = pb_geom::kurbo::Rect::from_center_size(at, (side, side));
        let style = Style {
            fill: self.sticky_color,
            stroke_width: 0.0,
            ..Style::default()
        };
        let edit = TextEdit::new_sticky(self.text.engine(), bounds, style);
        self.start_editing(edit);
    }

    /// Starts typing a new Markdown document with its top middle at `at` (world), from a
    /// short example that the first keystroke replaces.
    fn add_doc_at(&mut self, at: Point) {
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        let zoom = self.camera.zoom();
        let (width, size) = (pb_core::Doc::WIDTH / zoom, pb_core::Doc::SIZE / zoom);
        let corner = at - pb_geom::kurbo::Vec2::new(width / 2.0, 0.0);
        let engine = self.text.engine();
        let mut edit = TextEdit::new_doc(engine, corner, width, size, DOC_TEMPLATE);
        edit.select_all(engine);
        self.start_editing(edit);
    }

    /// Adds a table of `rows` centered on `at` (world) or in the middle of the view, sized
    /// to its text, and selects it. Returns a note when it had to be cut.
    fn insert_table(&mut self, mut rows: pb_io::csv::Rows, at: Option<Point>) -> Option<String> {
        use pb_core::{Cell, Table};

        let (row_count, col_count) = (rows.len(), rows.first().map_or(0, Vec::len));
        let note = (row_count > MAX_TABLE_ROWS || col_count > MAX_TABLE_COLS).then(|| {
            trf(
                "only the first {} rows and {} columns were added",
                &[
                    &row_count.min(MAX_TABLE_ROWS),
                    &col_count.min(MAX_TABLE_COLS),
                ],
            )
        });
        rows.truncate(MAX_TABLE_ROWS);
        for row in &mut rows {
            row.truncate(MAX_TABLE_COLS);
        }
        let (row_count, col_count) = (rows.len(), rows.first().map_or(0, Vec::len));
        if row_count == 0 || col_count == 0 {
            return note;
        }
        // Sizes in world units at 100%, scaled for the current zoom below.
        let (min_w, min_h) = Table::CELL_SIZE;
        let size = Table::FONT_SIZE;
        let pad = 2.0 * TABLE_CELL_PAD;
        let engine = self.text.engine();
        let widths: Vec<f64> = (0..col_count)
            .map(|c| {
                let widest = rows
                    .iter()
                    .map(|row| engine.layout(&row[c], size, None).width)
                    .fold(0.0, f64::max);
                (widest + pad).clamp(min_w * 0.6, TABLE_MAX_COLUMN)
            })
            .collect();
        let heights: Vec<f64> = rows
            .iter()
            .map(|row| {
                row.iter()
                    .zip(&widths)
                    .filter(|(text, _)| !text.is_empty())
                    .map(|(text, w)| engine.layout(text, size, Some(w - pad)).height + pad)
                    .fold(min_h, f64::max)
            })
            .collect();
        let mut table = Table::new(row_count, col_count, || self.document.new_line_id());
        let rows_lines: Vec<_> = table.rows.to_vec();
        let cols_lines: Vec<_> = table.cols.to_vec();
        let mut cells = pb_core::table::Cells::new();
        for (row, line) in rows.into_iter().zip(&rows_lines) {
            for (text, col) in row.into_iter().zip(&cols_lines) {
                if !text.is_empty() {
                    let cell = Cell {
                        text: text.into(),
                        spans: Default::default(),
                    };
                    cells.insert((line.id.clone(), col.id.clone()), cell);
                }
            }
        }
        let weigh = |lines: Vec<pb_core::table::Line>, sizes: &[f64]| {
            lines
                .into_iter()
                .zip(sizes)
                .map(|(line, &weight)| pb_core::table::Line { weight, ..line })
                .collect()
        };
        table.rows = weigh(rows_lines, &heights);
        table.cols = weigh(cols_lines, &widths);
        table.cells = Arc::new(cells);
        let zoom = self.camera.zoom();
        let natural = pb_geom::kurbo::Size::new(widths.iter().sum(), heights.iter().sum());
        let center = at.unwrap_or_else(|| self.camera.visible_world_rect(self.viewport()).center());
        let mut style = Table::style();
        style.font_size = Some(size / zoom);
        let data = ObjectData {
            kind: ObjectKind::Table(table),
            bounds: pb_geom::kurbo::Rect::from_center_size(center, natural / zoom),
            rotation: 0.0,
            style,
            tags: Default::default(),
            link: None,
        };
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        self.apply(Command::Create(vec![data.into()]));
        note
    }

    /// Adds Markdown and text files as documents side by side, at `at` (world) or in the
    /// middle of the view, as one undo step.
    fn insert_documents(&mut self, files: &[(String, String)], at: Option<Point>) {
        if files.is_empty() {
            return;
        }
        let zoom = self.camera.zoom();
        let (width, size) = (pb_core::Doc::WIDTH / zoom, pb_core::Doc::SIZE / zoom);
        let pad = size * pb_core::Doc::PADDING;
        let gap = 40.0 / zoom;
        let total = files.len() as f64 * (width + gap) - gap;
        let center = at.unwrap_or_else(|| self.camera.visible_world_rect(self.viewport()).center());
        let mut x = center.x - total / 2.0;
        let mut items = Vec::new();
        for (_, text) in files {
            let height =
                pb_text::markdown::layout(self.text.engine(), text, width - 2.0 * pad, size)
                    .height
                    .max(size);
            let bounds = pb_geom::kurbo::Rect::new(
                x,
                center.y - 100.0 / zoom,
                x + width,
                center.y - 100.0 / zoom + height + 2.0 * pad,
            );
            items.push(
                ObjectData {
                    kind: ObjectKind::Doc(pb_core::Doc {
                        text: text.as_str().into(),
                        size,
                    }),
                    bounds,
                    rotation: 0.0,
                    style: pb_core::Doc::style(),
                    tags: Default::default(),
                    link: None,
                }
                .into(),
            );
            x += width + gap;
        }
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        self.apply(Command::Create(items));
    }

    /// Places a mind map with its central topic on `at` (world) and starts typing it.
    fn add_mindmap_at(&mut self, at: Point) {
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        let topic = tr("Central topic");
        let probe = pb_core::MindMap::new("probe", topic, pb_geom::kurbo::Size::ZERO);
        let size = probe.font_size(0);
        let layout = self
            .text
            .engine()
            .layout(topic, size, Some(pb_core::MindMap::WRAP_WIDTH));
        let text_size = pb_geom::kurbo::Size::new(layout.width, layout.height);
        let map = pb_core::MindMap::new(self.document.new_line_id(), topic, text_size);
        let root = map.nodes[0].id.clone();
        let natural = map.layout().bounds.size();
        let zoom = self.camera.zoom();
        let bounds = pb_geom::kurbo::Rect::from_center_size(at, natural / zoom);
        let data = ObjectData {
            kind: ObjectKind::MindMap(map),
            bounds,
            rotation: 0.0,
            style: pb_core::MindMap::style(),
            tags: Default::default(),
            link: None,
        };
        self.apply(Command::Create(vec![data.into()]));
        if let [id] = self.tools.selection().ids() {
            let id = *id;
            let engine = self.text.engine();
            if let Some(mut edit) = TextEdit::node(engine, &self.document, id, &root) {
                edit.select_all(engine);
                self.start_editing(edit);
            }
        }
    }

    /// Tab or Enter while typing a node: keeps it and starts a new child or sibling.
    fn next_node(&mut self, child: bool) {
        let Some((id, parent, order, left)) =
            self.editing.as_ref().and_then(|e| e.next_node(child))
        else {
            return;
        };
        self.finish_editing();
        let engine = self.text.engine();
        if let Some(edit) = TextEdit::new_node(engine, &self.document, id, &parent, order, left) {
            self.start_editing(edit);
        }
    }

    /// Tab or Enter on the selected node: starts typing a new child or sibling of it.
    fn add_node(&mut self, child: bool) {
        let Some((id, i)) = self.tools.selected_node(&self.document) else {
            return;
        };
        let place = match self.document.get(id).map(|o| &o.data.kind) {
            Some(ObjectKind::MindMap(map)) => map.new_node_place(i, child),
            _ => None,
        };
        let Some((parent, order, left)) = place else {
            return;
        };
        let engine = self.text.engine();
        if let Some(edit) = TextEdit::new_node(engine, &self.document, id, &parent, order, left) {
            self.start_editing(edit);
        }
    }

    /// Adds a Mermaid flowchart in the middle of the view, at a size that reads like
    /// normal text there, as one undo step, and selects it.
    /// Shows saved version `id` instead of the board, with what changed since marked.
    fn open_version(&mut self, id: Arc<str>) {
        self.finish_editing();
        self.tools.cancel();
        self.tools.set_active(ToolKind::Select);
        self.tools.clear_selection();
        self.context_menu = None;
        self.thread_panel = None;
        self.search = None;
        let doc = match self.document.version_view(&id) {
            Ok(doc) => doc,
            Err(err) => {
                dialog::show_error(tr("Could not show the version"), &err.to_string());
                return;
            }
        };
        let changes = pb_core::version::compare(&doc, &self.document);
        let highlights = changes
            .changed
            .iter()
            .chain(&changes.removed)
            .filter_map(|&id| doc.get(id))
            .map(|o| o.data.frame())
            .collect();
        let count = |n: usize, what: &'static str| (n > 0).then(|| trf(what, &[&n]));
        let parts: Vec<String> = [
            count(changes.changed.len(), "{} changed"),
            count(changes.removed.len(), "{} removed"),
            count(changes.added.len(), "{} added"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let summary = if parts.is_empty() {
            tr("the same as now").to_owned()
        } else {
            trf("{} since", &[&parts.join(", ")])
        };
        self.version_view = Some(VersionView {
            id,
            doc,
            highlights,
            summary,
        });
    }

    /// Writes a PDF of the board (a page per frame, or the whole board without frames) to
    /// the temporary folder and opens it in the system's viewer, which prints it.
    fn print(&mut self) {
        self.finish_editing();
        let has_frames = self
            .document
            .objects()
            .iter()
            .any(|o| o.data.kind.is_frame());
        let settings = export::Settings {
            scope: if has_frames {
                export::Scope::Frames
            } else {
                export::Scope::Board
            },
            format: export::Format::Pdf,
            // Sharp on paper: a board unit at 100% is a pixel at 96 dpi.
            scale: 3.0,
            transparent: false,
        };
        let targets = export::targets(&self.document, settings.scope, &[]);
        if targets.is_empty() {
            dialog::show_error(tr("Print"), tr("There is nothing to print."));
            return;
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        // Pictures are always light, like paper.
        let light = Theme::light();
        let painters = export::Painters {
            gpu,
            text: &mut self.text,
            images: &mut self.images,
            theme: &light,
            me: &self.me,
        };
        let title = self.session.name();
        let name = file_name(&title);
        let written =
            export::file(&self.document, &targets, &settings, &title, painters).and_then(|bytes| {
                let path = pb_platform::fs::temp_dir()
                    .map_err(|err| err.to_string())?
                    .join(format!("{name}.pdf"));
                pb_platform::fs::write_atomic(&path, &bytes).map_err(|err| err.to_string())?;
                Ok(path)
            });
        match written {
            Ok(path) => {
                if !pb_platform::browser::open_file(&path) {
                    dialog::show_error(
                        "Print",
                        &trf(
                            "No app opens PDF files. The file is at {}",
                            &[&path.display()],
                        ),
                    );
                }
            }
            Err(err) => {
                dialog::show_error(tr("Print"), &trf("Could not make the PDF: {}", &[&err]))
            }
        }
    }

    /// Deletes the selection; asks first when that is a lot of the board.
    fn delete_selected(&mut self) {
        let Some(command) = self.tools.delete_selected(&self.document) else {
            return;
        };
        if let Command::Delete(ids) = &command {
            let count: usize = ids.iter().map(|&id| self.document.leaves(id).len()).sum();
            let everything = count >= DELETE_ASK_ALL && count == self.document.objects().len();
            if count >= DELETE_ASK || everything {
                let question = trf("Delete {} objects? Ctrl+Z brings them back.", &[&count]);
                if !dialog::confirm(tr("Delete"), &question) {
                    return;
                }
            }
        }
        self.apply(command);
    }

    /// Sets the UI's size relative to normal and high contrast.
    fn set_view(&mut self, scale: f32, high_contrast: bool) {
        self.ui_scale = scale.clamp(*crate::ui::UI_SCALES.start(), *crate::ui::UI_SCALES.end());
        self.high_contrast = high_contrast;
        if let Some(ui) = &mut self.ui {
            ui.set_scale(self.ui_scale);
            ui.set_high_contrast(high_contrast);
        }
        self.request_redraw();
    }

    /// Switches between the dark and the light theme, the board and the UI alike.
    fn set_dark(&mut self, dark: bool) {
        self.theme = if dark { Theme::dark() } else { Theme::light() };
        if let Some(ui) = &mut self.ui {
            ui.set_dark(dark);
        }
        self.request_redraw();
    }

    fn open_export(&mut self) {
        self.finish_editing();
        self.context_menu = None;
        self.show_export = true;
    }

    /// Asks where to save a picture `settings` describe and saves it; `false` if there
    /// was nothing to draw or it was cancelled.
    fn export_picture(&mut self, settings: &export::Settings) -> bool {
        let selection = self.tools.selection().ids().to_vec();
        let targets = export::targets(&self.document, settings.scope, &selection);
        if targets.is_empty() {
            dialog::show_error(tr("Export"), tr("There is nothing to export."));
            return false;
        }
        let name = match settings.scope {
            export::Scope::Frame(id) => match self.document.get(id).map(|o| &o.data.kind) {
                Some(ObjectKind::Frame(frame)) if !frame.title.trim().is_empty() => {
                    frame.title.trim().to_owned()
                }
                _ => tr("Frame").to_owned(),
            },
            export::Scope::Selection => trf("{} selection", &[&self.session.name()]),
            export::Scope::Board | export::Scope::Frames => self.session.name(),
        };
        let name = file_name(&name);
        let format = settings.format;
        let file_type = dialog::FileType {
            name: format.name(),
            extension: format.extension(),
        };
        let suggested = PathBuf::from(format!("{name}.{}", format.extension()));
        let Some(path) = dialog::pick_save(&file_type, &suggested) else {
            return false;
        };
        let Some(gpu) = self.gpu.as_mut() else {
            return false;
        };
        // Pictures are always light, like paper.
        let light = Theme::light();
        let painters = export::Painters {
            gpu,
            text: &mut self.text,
            images: &mut self.images,
            theme: &light,
            me: &self.me,
        };
        let title = self.session.name();
        let saved =
            export::file(&self.document, &targets, settings, &title, painters).and_then(|bytes| {
                pb_platform::fs::write_atomic(&path, &bytes).map_err(|err| err.to_string())
            });
        match saved {
            Ok(()) => true,
            Err(err) => {
                dialog::show_error(tr("Export"), &trf("Could not save the file: {}", &[&err]));
                false
            }
        }
    }

    fn insert_diagram(&mut self, source: &str) -> Result<(), pb_io::mermaid::ParseError> {
        use pb_core::{Anchor, Cap, Connector, End, Label, Route, Shape};
        use pb_geom::kurbo::{Rect, Size};
        use pb_io::mermaid::{self, LineStyle, NodeShape};

        let chart = mermaid::parse(source)?;
        let engine = self.text.engine();
        // Sized in world units at 100%, then scaled for the current zoom.
        let sizes: Vec<Size> = chart
            .nodes
            .iter()
            .map(|node| {
                let layout = engine.layout(&node.text, DIAGRAM_FONT, Some(DIAGRAM_WRAP));
                let (w, h) = (
                    (layout.width + 40.0).max(120.0),
                    (layout.height + 28.0).max(56.0),
                );
                let (w, h) = match node.shape {
                    NodeShape::Decision => (w * 1.6, h * 1.6),
                    NodeShape::Circle => {
                        let side = w.max(h) * 1.2;
                        (side, side)
                    }
                    NodeShape::Database => (w, h + 30.0),
                    NodeShape::Stadium => (w + h * 0.5, h),
                    NodeShape::Hexagon | NodeShape::Data => (w + 50.0, h),
                    NodeShape::Flag => (w * 1.2, h * 1.3),
                    NodeShape::Box | NodeShape::Rounded => (w, h),
                };
                Size::new(w, h)
            })
            .collect();
        let centers = mermaid::layout(&chart, &sizes);
        let scale = 1.0 / self.camera.zoom();
        let rects: Vec<Rect> = centers
            .iter()
            .zip(&sizes)
            .map(|(&c, &s)| Rect::from_center_size(c, s))
            .collect();
        let all = rects
            .iter()
            .copied()
            .reduce(|a, b| a.union(b))
            .unwrap_or_default();
        let view = self.camera.visible_world_rect(self.viewport()).center();
        let place = |r: Rect| {
            let origin = view + (r.origin() - all.center()) * scale;
            Rect::from_origin_size(origin, r.size() * scale)
        };
        let shapes: Vec<pb_core::Content> = chart
            .nodes
            .iter()
            .zip(&rects)
            .map(|(node, &r)| {
                ObjectData {
                    kind: ObjectKind::Shape(Shape {
                        kind: node.shape.kind(),
                        text: node.text.as_str().into(),
                        spans: Default::default(),
                    }),
                    bounds: place(r),
                    rotation: 0.0,
                    style: Style {
                        font_size: Some(DIAGRAM_FONT * scale),
                        ..Style::shape()
                    },
                    tags: Default::default(),
                    link: None,
                }
                .into()
            })
            .collect();
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        self.document.begin_undo_group();
        let ids = match self.document.apply(Command::Create(shapes)) {
            Ok(applied) => applied.created,
            Err(err) => {
                log::error!("diagram shapes failed: {err}");
                self.document.end_undo_group();
                return Ok(());
            }
        };
        let engine = self.text.engine();
        let connectors: Vec<pb_core::Content> = chart
            .edges
            .iter()
            .filter_map(|edge| {
                let (from, to) = (*ids.get(edge.from)?, *ids.get(edge.to)?);
                // A line going back against the flow leaves and enters from the side, so it
                // runs around the shapes rather than over the line going forward.
                let back = match chart.direction {
                    mermaid::Direction::Down => centers[edge.to].y < centers[edge.from].y,
                    mermaid::Direction::Up => centers[edge.to].y > centers[edge.from].y,
                    mermaid::Direction::Right => centers[edge.to].x < centers[edge.from].x,
                    mermaid::Direction::Left => centers[edge.to].x > centers[edge.from].x,
                };
                let anchor = match (back, chart.direction) {
                    (false, _) => Anchor::Auto,
                    (true, mermaid::Direction::Down | mermaid::Direction::Up) => {
                        Anchor::Fixed(pb_geom::kurbo::Point::new(1.0, 0.5))
                    }
                    (true, _) => Anchor::Fixed(pb_geom::kurbo::Point::new(0.5, 1.0)),
                };
                let end = |object, rect: Rect| End::Attached {
                    object,
                    anchor,
                    at: place(rect).center(),
                };
                let label = match &edge.label {
                    Some(text) => {
                        let layout = engine.layout(text, Label::FONT_SIZE, None);
                        Label {
                            text: text.as_str().into(),
                            at: 0.5,
                            size: Size::new(layout.width, layout.height),
                        }
                    }
                    None => Label::default(),
                };
                let (start, end) = (end(from, rects[edge.from]), end(to, rects[edge.to]));
                let style = Style::default();
                Some(
                    ObjectData {
                        kind: ObjectKind::Connector(Connector {
                            start,
                            end,
                            route: Route::Elbow,
                            start_cap: Cap::None,
                            end_cap: if edge.arrow { Cap::Arrow } else { Cap::None },
                            label,
                            bends: Default::default(),
                        }),
                        bounds: Rect::from_points(start.point(), end.point()),
                        rotation: 0.0,
                        style: Style {
                            stroke_width: match edge.line {
                                LineStyle::Thick => style.stroke_width * 2.0,
                                LineStyle::Solid | LineStyle::Dotted => style.stroke_width,
                            },
                            ..style
                        },
                        tags: Default::default(),
                        link: None,
                    }
                    .into(),
                )
            })
            .collect();
        let mut created = ids;
        if !connectors.is_empty() {
            match self.document.apply(Command::Create(connectors)) {
                Ok(applied) => created.extend(applied.created),
                Err(err) => log::error!("diagram connectors failed: {err}"),
            }
        }
        self.document.end_undo_group();
        self.tools.sync(&self.document);
        self.tools.select_only(created);
        // Show all of it.
        let placed = place(all);
        let viewport = self.viewport();
        let room = self
            .camera
            .visible_world_rect(viewport)
            .inset(-FRAME_MARGIN / self.camera.zoom());
        if placed.width() > room.width() || placed.height() > room.height() {
            self.camera.fit(placed, viewport, FRAME_MARGIN);
        }
        Ok(())
    }

    /// Vote counts to show on objects: yours while voting, everyone's once it ended (the
    /// top three marked).
    fn vote_badges(&self) -> HashMap<ObjectId, (u32, bool)> {
        let Some(voting) = self.document.voting() else {
            return HashMap::new();
        };
        if voting.open {
            return voting
                .votes
                .keys()
                .map(|&id| (id, (voting.count(id, &self.me), false)))
                .filter(|(_, (n, _))| *n > 0)
                .collect();
        }
        let ranking = voting.ranking();
        let third = ranking
            .get(2.min(ranking.len().saturating_sub(1)))
            .map_or(0, |r| r.1);
        ranking
            .into_iter()
            .map(|(id, n)| (id, (n, n >= third.max(1))))
            .collect()
    }

    /// Objects with votes, most first, each with a line about it.
    fn vote_results(&self) -> Vec<(ObjectId, u32, String)> {
        let Some(voting) = self.document.voting() else {
            return Vec::new();
        };
        voting
            .ranking()
            .into_iter()
            .filter_map(|(id, n)| {
                let object = self.document.get(id)?;
                let text = object
                    .data
                    .kind
                    .text()
                    .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                    .filter(|t| !t.is_empty())
                    .map(|t| {
                        let short: String = t.chars().take(48).collect();
                        if t.chars().count() > 48 {
                            format!("{short}…")
                        } else {
                            short
                        }
                    })
                    .unwrap_or_else(|| tr("An object without text").to_owned());
                Some((id, n, text))
            })
            .collect()
    }

    /// Starts presenting the board's frames, in the order the Frames menu lists them.
    fn start_presentation(&mut self) {
        let frames: Vec<ObjectId> = self
            .document
            .objects()
            .iter()
            .filter(|o| o.data.kind.is_frame())
            .map(|o| o.id)
            .collect();
        if frames.is_empty() {
            return;
        }
        self.finish_editing();
        self.tools.cancel();
        self.tools.set_active(ToolKind::Select);
        self.tools.clear_selection();
        self.context_menu = None;
        self.thread_panel = None;
        self.search = None;
        self.show_templates = false;
        self.show_shortcuts = false;
        // The presentation shows whole frames, and ends where focus mode began.
        if let Some(saved) = self.focus_saved.take() {
            self.document.set_focus(None);
            self.camera = saved;
        }
        self.presentation = Some(Presentation {
            index: 0,
            frames,
            saved: self.camera,
            laser: false,
            trail: Vec::new(),
        });
        self.show_slide(0);
    }

    /// Moves `step` frames on (back when negative); `i64::MIN` and `i64::MAX` go to the
    /// first and the last.
    fn present_step(&mut self, step: i64) {
        let Some(p) = &self.presentation else {
            return;
        };
        let last = p.frames.len().saturating_sub(1) as i64;
        let next = match step {
            i64::MIN => 0,
            i64::MAX => last,
            s => (p.index as i64 + s).clamp(0, last),
        };
        self.show_slide(next as usize);
    }

    /// Shows frame `index` of the presentation, gliding there.
    fn show_slide(&mut self, index: usize) {
        let Some(p) = &mut self.presentation else {
            return;
        };
        p.index = index.min(p.frames.len().saturating_sub(1));
        let Some(area) = p
            .frames
            .get(p.index)
            .and_then(|&id| self.document.get(id))
            .map(|o| o.data.bounds)
        else {
            return;
        };
        let viewport = self.viewport();
        let mut target = self.camera;
        target.fit(area, viewport, PRESENT_MARGIN);
        self.glide_to(&target);
    }

    /// Ends the presentation, gliding back to where the board was.
    fn stop_presentation(&mut self) {
        if let Some(p) = self.presentation.take() {
            self.glide_to(&p.saved);
        }
        if let Some(window) = &self.window {
            window.set_cursor_visible(true);
        }
    }

    /// Enters focus mode on the selection (or on what is in view when nothing is
    /// selected), or leaves it.
    fn toggle_focus(&mut self) {
        if self.focus_saved.is_some() {
            self.stop_focus();
            return;
        }
        let viewport = self.viewport();
        let area = match self.tools.selection().frame(&self.document) {
            Some(frame) => frame.aabb(),
            None => self.camera.visible_world_rect(viewport),
        };
        self.finish_editing();
        self.tools.cancel();
        self.context_menu = None;
        self.search = None;
        // A hair larger, so what was picked keeps its center inside however it rounds.
        let area = area.inflate(1.0, 1.0);
        self.document.set_focus(Some(area));
        self.focus_saved = Some(self.camera);
        let mut target = self.camera;
        target.fit(area, viewport, FRAME_MARGIN);
        self.glide_to(&target);
    }

    /// Stops recording and keeps the talktrack on the board.
    fn stop_recording(&mut self) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        let id: Arc<str> = self.document.new_line_id().into();
        let title = trf("Talktrack {}", &[&(self.document.talktracks().len() + 1)]);
        let track = recording.finish(Instant::now(), &mut self.document, id, title);
        self.apply(Command::SaveTalktrack(track));
    }

    /// Plays talktrack `id` back: the view follows it, the UI steps aside.
    fn play_talktrack(&mut self, id: &str) {
        let Some(track) = self
            .document
            .talktracks()
            .iter()
            .find(|t| &*t.id == id)
            .cloned()
        else {
            return;
        };
        if self.presentation.is_some() {
            self.stop_presentation();
        }
        self.finish_editing();
        self.tools.cancel();
        self.tools.clear_selection();
        self.context_menu = None;
        self.camera_glide = None;
        let saved = self.playback.take().map_or(self.camera, |p| p.saved);
        self.playback = Some(Playback::start(
            track,
            &self.document,
            saved,
            Instant::now(),
        ));
        if let Some(window) = &self.window {
            window.set_cursor_visible(true);
        }
    }

    /// Ends playback, gliding back to where the board was.
    fn stop_playback(&mut self) {
        if let Some(p) = self.playback.take() {
            self.glide_to(&p.saved);
        }
    }

    /// A key while a talktrack plays. Returns true if the view needs a redraw.
    fn playback_key(&mut self, event: &KeyEvent) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return false;
        };
        let now = Instant::now();
        match code {
            KeyCode::Escape => self.stop_playback(),
            KeyCode::Space | KeyCode::KeyK => {
                if let Some(p) = &mut self.playback {
                    p.toggle(now);
                }
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                if let Some(p) = &mut self.playback {
                    let step = if code == KeyCode::ArrowLeft {
                        -5.0
                    } else {
                        5.0
                    };
                    let at = p.position(now) + step;
                    p.seek(at, now);
                }
            }
            _ => return false,
        }
        true
    }

    /// Layers to move the selection to, top first, when there is more than one.
    fn layer_choices(&self) -> Vec<(Arc<str>, String)> {
        let layers = self.document.layers();
        if layers.len() < 2 {
            return Vec::new();
        }
        layers
            .iter()
            .rev()
            .map(|l| (l.id.clone(), l.name.to_string()))
            .collect()
    }

    /// The layers panel's contents, when it is open.
    fn layers_view(&self) -> Option<LayersView> {
        if !self.show_layers {
            return None;
        }
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for &root in self.document.roots() {
            *counts.entry(&self.document.layer_of(root).id).or_default() += 1;
        }
        Some(LayersView {
            layers: self
                .document
                .layers()
                .iter()
                .rev()
                .map(|l| (l.clone(), counts.get(&*l.id).copied().unwrap_or(0)))
                .collect(),
            active: self.document.active_layer().id.clone(),
        })
    }

    /// Swaps layer `id` with the one above it (`up`) or below it.
    fn move_layer(&mut self, id: &str, up: bool) {
        let layers = self.document.layers();
        let Some(i) = layers.iter().position(|l| &*l.id == id) else {
            return;
        };
        let j = if up { i + 1 } else { i.wrapping_sub(1) };
        let (Some(a), Some(b)) = (layers.get(i), layers.get(j)) else {
            return;
        };
        let (mut a, mut b) = (a.clone(), b.clone());
        std::mem::swap(&mut a.order, &mut b.order);
        if a.order == b.order {
            // Same order (stacked by id): spread them apart.
            a.order += if up { 0.5 } else { -0.5 };
        }
        self.document.begin_undo_group();
        self.apply(Command::DefineLayer(a));
        self.apply(Command::DefineLayer(b));
        self.document.end_undo_group();
    }

    /// Hides the selection, or shows it again (Ctrl+Shift+H).
    fn toggle_hidden(&mut self) {
        self.finish_editing();
        self.apply_with(Tools::toggle_hidden_selected);
        self.drop_unreachable();
    }

    /// Takes what can no longer be picked (hidden) out of the selection.
    fn drop_unreachable(&mut self) {
        let document = &self.document;
        let ids = self.tools.selection().ids();
        if ids.iter().any(|&id| !document.reachable(id)) {
            let kept: Vec<_> = ids
                .iter()
                .copied()
                .filter(|&id| document.reachable(id))
                .collect();
            self.tools.select_only(kept);
        }
    }

    /// Leaves focus mode, gliding back to where the board was.
    fn stop_focus(&mut self) {
        self.document.set_focus(None);
        if let Some(saved) = self.focus_saved.take() {
            self.glide_to(&saved);
        }
    }

    /// Turns the laser pointer on or off while presenting.
    fn toggle_laser(&mut self) {
        let Some(p) = &mut self.presentation else {
            return;
        };
        p.laser = !p.laser;
        p.trail.clear();
        let laser = p.laser;
        if let Some(window) = &self.window {
            window.set_cursor_visible(!laser);
        }
        self.request_redraw();
    }

    /// Starts the camera gliding from where it is to `target`.
    fn glide_to(&mut self, target: &Camera) {
        let viewport = self.viewport();
        self.camera_glide = Some(Glide {
            from: (
                self.camera.visible_world_rect(viewport).center(),
                self.camera.zoom(),
            ),
            to: (target.visible_world_rect(viewport).center(), target.zoom()),
            start: Instant::now(),
        });
        self.request_redraw();
    }

    /// A key while presenting. Returns true if the view needs a redraw.
    fn presentation_key(&mut self, event: &KeyEvent) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return false;
        };
        match code {
            KeyCode::ArrowRight
            | KeyCode::ArrowDown
            | KeyCode::Space
            | KeyCode::PageDown
            | KeyCode::Enter
            | KeyCode::NumpadEnter => self.present_step(1),
            KeyCode::ArrowLeft | KeyCode::ArrowUp | KeyCode::PageUp | KeyCode::Backspace => {
                self.present_step(-1);
            }
            KeyCode::Home => self.present_step(i64::MIN),
            KeyCode::End => self.present_step(i64::MAX),
            KeyCode::Escape | KeyCode::F5 => self.stop_presentation(),
            KeyCode::KeyL => self.toggle_laser(),
            _ => return false,
        }
        true
    }

    /// Lays the selection out in clusters by `by`, each with a heading, as one undo step.
    fn cluster(&mut self, by: pb_tools::ClusterBy) {
        let Some((moves, labels)) = pb_tools::cluster(&self.document, self.tools.selection(), by)
        else {
            return;
        };
        let selected = self.tools.selection().ids().to_vec();
        let engine = self.text.engine();
        let headings: Vec<pb_core::Content> = labels
            .iter()
            .map(|label| {
                let layout = engine.layout(&label.text, label.size, None);
                let spans: pb_core::Spans = std::sync::Arc::new([pb_core::Span {
                    start: 0,
                    end: label.text.len(),
                    marks: pb_core::TextMarks {
                        bold: true,
                        ..Default::default()
                    },
                }]);
                ObjectData {
                    kind: ObjectKind::Text(Text {
                        text: label.text.as_str().into(),
                        spans,
                        size: label.size,
                    }),
                    bounds: pb_geom::kurbo::Rect::from_origin_size(
                        label.at,
                        (layout.width, layout.height),
                    ),
                    rotation: 0.0,
                    style: Style {
                        stroke: Rgba::rgb(0x1F, 0x29, 0x37),
                        ..Style::default()
                    },
                    tags: Default::default(),
                    link: None,
                }
                .into()
            })
            .collect();
        self.document.begin_undo_group();
        if let Some(moves) = moves {
            self.apply(moves);
        }
        self.apply(Command::Create(headings));
        self.document.end_undo_group();
        // The clustered objects stay selected, not the headings.
        self.tools.select_only(selected);
        self.tools.sync(&self.document);
    }

    /// Adds template `i` in the middle of the view as one undo step, selects it, and shows
    /// all of it.
    fn insert_template(&mut self, i: usize) {
        let Some(template) = crate::templates::TEMPLATES.get(i) else {
            return;
        };
        let objects = match template.make {
            crate::templates::Make::Mermaid(source) => {
                if let Err(err) = self.insert_diagram(source) {
                    log::error!("template {}: {err}", template.name);
                }
                return;
            }
            crate::templates::Make::Objects(make) => make,
        };
        self.finish_editing();
        self.tools.set_active(ToolKind::Select);
        let viewport = self.viewport();
        let center = self.camera.visible_world_rect(viewport).center();
        let mut builder = crate::templates::Builder::new(&self.document, self.text.engine());
        objects(&mut builder);
        let items = builder.finish(center);
        let area = items
            .iter()
            .flat_map(pb_core::Content::shapes)
            .map(|d| d.bounds)
            .reduce(|a, b| a.union(b));
        self.apply(Command::Create(items));
        if let Some(area) = area {
            let room = self
                .camera
                .visible_world_rect(viewport)
                .inset(-FRAME_MARGIN / self.camera.zoom());
            if area.width() > room.width() || area.height() > room.height() {
                self.camera.fit(area, viewport, FRAME_MARGIN);
            }
        }
    }

    /// Opens the right-click menu at the pointer. Like elsewhere, the object under it gets
    /// selected unless it already is; on empty board the selection is dropped.
    fn open_context_menu(&mut self) {
        let (Some(pointer), Some(screen)) = (self.pointer(), self.navigation.cursor()) else {
            return;
        };
        self.finish_editing();
        self.tools.cancel();
        self.tools.set_active(ToolKind::Select);
        match self.tools.object_at(&self.document, pointer) {
            Some(id) if !self.tools.selection().contains(id) => self.tools.select_only([id]),
            Some(_) => {}
            None => {
                self.tools.clear_selection();
            }
        }
        self.context_menu = Some((screen, pointer.world));
    }

    /// What the right-click menu offers.
    fn context_menu_info(&self, screen: Point) -> ContextMenuInfo {
        let ids = self.tools.selection().ids();
        let single = match ids {
            [id] => self.document.get(*id),
            _ => None,
        };
        ContextMenuInfo {
            pos: (screen.x as f32, screen.y as f32),
            selection: !ids.is_empty(),
            several: ids.len() > 1,
            has_group: ids.iter().any(|&id| self.document.group(id).is_some()),
            locked: self.tools.selection().has_locked(&self.document),
            has_text: single.is_some_and(|o| {
                matches!(
                    o.data.kind,
                    ObjectKind::Text(_)
                        | ObjectKind::Sticky(_)
                        | ObjectKind::Shape(_)
                        | ObjectKind::Frame(_)
                        | ObjectKind::Connector(_)
                        | ObjectKind::Table(_)
                        | ObjectKind::Doc(_)
                        | ObjectKind::Embed(_)
                )
            }),
            single_object: single.is_some(),
            focus: self.focus_saved.is_some(),
            hidden: !ids.is_empty() && ids.iter().all(|&id| self.document.is_hidden(id)),
            read_only: self.document.read_only(),
        }
    }

    /// Opens the search bar, or gives its field the keyboard again.
    fn open_search(&mut self) {
        self.finish_editing();
        self.search.get_or_insert_with(Search::default);
        self.focus_search = true;
    }

    /// Searches for `query` and shows the first result.
    fn search_for(&mut self, query: String) {
        let hits = pb_core::search::search(&self.document, &query);
        self.search = Some(Search {
            query,
            hits,
            current: 0,
            revision: self.document.revision(),
        });
        self.show_search_result();
    }

    /// Goes to the next result, or the previous one.
    fn search_step(&mut self, forward: bool) {
        if let Some(search) = &mut self.search
            && !search.hits.is_empty()
        {
            let n = search.hits.len();
            search.current = if forward {
                (search.current + 1) % n
            } else {
                (search.current + n - 1) % n
            };
            self.show_search_result();
        }
    }

    /// Searches again after the board changed, staying on the same result if it can.
    fn refresh_search(&mut self) {
        let revision = self.document.revision();
        let Some(search) = &mut self.search else {
            return;
        };
        if search.revision == revision {
            return;
        }
        let hits = pb_core::search::search(&self.document, &search.query);
        search.current = search.current.min(hits.len().saturating_sub(1));
        search.hits = hits;
        search.revision = revision;
    }

    /// Brings the current result into view: pans to it, zooming out only if it does not
    /// fit.
    fn show_search_result(&mut self) {
        let Some(area) = self
            .search
            .as_ref()
            .and_then(|s| s.hits.get(s.current))
            .map(|h| h.area.aabb())
        else {
            return;
        };
        let viewport = self.viewport();
        let zoom = self.camera.zoom();
        let margin = FRAME_MARGIN / zoom;
        let view = self.camera.visible_world_rect(viewport);
        let room = view.inset(-margin);
        if room.contains(area.origin()) && room.contains(Point::new(area.x1, area.y1)) {
            return;
        }
        if area.width() <= room.width() && area.height() <= room.height() {
            self.camera.look_at(area.center(), zoom, viewport);
        } else {
            self.camera.fit(area, viewport, FRAME_MARGIN);
        }
    }

    /// Pins of the threads shown: the spot each marks (world), its id and whether it is
    /// resolved.
    fn comment_pins(&self) -> Vec<(Point, Arc<str>, bool)> {
        self.document
            .threads()
            .iter()
            .filter(|t| self.show_resolved || !t.resolved)
            .filter_map(|t| Some((self.document.thread_point(t)?, t.id.clone(), t.resolved)))
            .filter(|(at, _, _)| self.document.focus().is_none_or(|area| area.contains(*at)))
            .collect()
    }

    /// The thread whose pin is at `screen` (logical pixels), if any.
    fn comment_pin_at(&self, screen: Point) -> Option<Arc<str>> {
        self.comment_pins()
            .into_iter()
            .rev()
            .find_map(|(at, id, _)| {
                let tip = self.camera.world_to_screen(at);
                let center = pb_render::comment_pin_center(tip, 1.0);
                ((center - screen).hypot() <= pb_render::COMMENT_PIN_RADIUS + 3.0).then_some(id)
            })
    }

    /// What the thread panel shows.
    fn thread_view(&self) -> Option<ThreadView> {
        let panel = self.thread_panel.as_ref()?;
        let (at, thread) = match &panel.thread {
            Some(id) => {
                let thread = self.document.thread(id)?;
                (self.document.thread_point(thread)?, Some(thread))
            }
            None => match panel.anchor? {
                pb_core::ThreadAnchor::Point(p) => (p, None),
                pb_core::ThreadAnchor::Object(id) => {
                    (self.document.get(id)?.data.frame().corners()[0], None)
                }
            },
        };
        let screen = self.camera.world_to_screen(at);
        let now = pb_platform::now_ms();
        Some(ThreadView {
            pos: (screen.x as f32, screen.y as f32),
            is_new: thread.is_none(),
            resolved: thread.is_some_and(|t| t.resolved),
            comments: thread
                .map(|t| {
                    t.comments
                        .iter()
                        .map(|c| CommentView {
                            id: c.id.clone(),
                            author: c.author.to_string(),
                            when: time_ago(now, c.at_ms),
                            text: c.text.to_string(),
                            mine: c.author == self.me,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// The board's threads for the Comments menu: id, a line about it, and whether it
    /// is resolved.
    fn thread_list(&self) -> Vec<(Arc<str>, String, bool)> {
        self.document
            .threads()
            .iter()
            .map(|t| {
                let first = &t.comments[0];
                let text: String = first.text.chars().take(40).collect();
                let more = if first.text.chars().count() > 40 {
                    "…"
                } else {
                    ""
                };
                let replies = match t.comments.len() {
                    1 => String::new(),
                    n => format!(" ({n})"),
                };
                (
                    t.id.clone(),
                    format!("{}: {text}{more}{replies}", first.author),
                    t.resolved,
                )
            })
            .collect()
    }

    /// Adds `text` to the thread shown, starting it if it is new.
    fn post_comment(&mut self, text: String) {
        let Some(panel) = &self.thread_panel else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        let thread: Arc<str> = match &panel.thread {
            Some(id) => id.clone(),
            None => self.document.new_line_id().into(),
        };
        let comment = pb_core::Comment {
            id: self.document.new_line_id().into(),
            author: self.me.clone(),
            at_ms: pb_platform::now_ms(),
            text: text.trim().into(),
        };
        let anchor = panel.anchor;
        self.apply(Command::AddComment {
            thread: thread.clone(),
            anchor,
            comment,
        });
        self.thread_panel = Some(ThreadPanel {
            thread: Some(thread),
            anchor: None,
        });
    }

    /// Opens a thread from the Comments menu, bringing its pin into view.
    fn open_thread(&mut self, id: Arc<str>) {
        let Some(at) = self
            .document
            .thread(&id)
            .and_then(|t| self.document.thread_point(t))
        else {
            return;
        };
        let viewport = self.viewport();
        let view = self.camera.visible_world_rect(viewport);
        if !view.inset(-FRAME_MARGIN / self.camera.zoom()).contains(at) {
            let zoom = self.camera.zoom();
            self.camera.look_at(at, zoom, viewport);
        }
        if self.document.thread(&id).is_some_and(|t| t.resolved) {
            self.show_resolved = true;
        }
        self.thread_panel = Some(ThreadPanel {
            thread: Some(id),
            anchor: None,
        });
    }

    /// Objects on screen with links, with the top right corners their badges sit by.
    fn link_badges(&self) -> Vec<(Point, Link)> {
        let visible = self.camera.visible_world_rect(self.viewport());
        self.document
            .objects_in(visible)
            .into_iter()
            .filter(|o| self.document.reachable(o.id))
            .filter_map(|o| {
                let link = o.data.link.clone()?;
                let corner = o.data.frame().corners()[1];
                Some((corner, link))
            })
            .collect()
    }

    /// The link whose badge is at `screen` (logical pixels), if any.
    fn link_at(&self, screen: Point) -> Option<Link> {
        self.link_badges().into_iter().find_map(|(corner, link)| {
            let corner = self.camera.world_to_screen(corner);
            let center = pb_render::link_badge_center(corner, 1.0);
            ((center - screen).hypot() <= pb_render::LINK_BADGE_RADIUS + 2.0).then_some(link)
        })
    }

    /// Opens a web link, or shows and selects the linked object.
    fn follow(&mut self, link: &Link) {
        match link {
            Link::Url(url) => {
                if !pb_platform::browser::open_url(url) {
                    dialog::show_error(tr("Could not open the link"), url);
                }
            }
            Link::Object(id) => {
                let Some(object) = self.document.get(*id) else {
                    dialog::show_error(
                        tr("Link target is gone"),
                        tr("The object this links to has been deleted."),
                    );
                    return;
                };
                let area = object.data.frame().aabb();
                let viewport = self.viewport();
                self.camera.fit(area, viewport, FRAME_MARGIN);
                self.finish_editing();
                self.tools.set_active(ToolKind::Select);
                let unit = self.document.top_level(*id);
                self.tools.select_only([unit]);
            }
        }
    }

    /// Starts a new card below the one being edited.
    fn next_card(&mut self) {
        let Some((id, col, at)) = self.editing.as_ref().and_then(|e| e.next_card()) else {
            return;
        };
        self.finish_editing();
        let engine = self.text.engine();
        if let Some(edit) = TextEdit::new_card(engine, &self.document, id, col, at) {
            self.start_editing(edit);
        }
    }

    /// Empties the selected table cells as one undo step.
    fn clear_cells(&mut self) {
        let commands = self.tools.clear_cells_text(&self.document);
        if commands.is_empty() {
            return;
        }
        self.document.begin_undo_group();
        for command in commands {
            self.apply(command);
        }
        self.document.end_undo_group();
    }

    fn next_sticky(&mut self, below: bool) {
        let Some(edit) = &mut self.editing else {
            return;
        };
        let Some(mut next) = edit.next_sticky(self.text.engine(), below) else {
            return;
        };
        self.finish_editing();
        let rect = next.frame(self.text.engine()).aabb();
        let visible = self.camera.visible_world_rect(self.viewport());
        let margin = rect.width().min(rect.height()) * 0.25;
        let shift = |lo: f64, hi: f64, min: f64, max: f64| {
            if hi + margin > max {
                hi + margin - max
            } else if lo - margin < min {
                lo - margin - min
            } else {
                0.0
            }
        };
        let dx = shift(rect.x0, rect.x1, visible.x0, visible.x1);
        let dy = shift(rect.y0, rect.y1, visible.y0, visible.y1);
        self.camera.pan(-Vec2::new(dx, dy) * self.camera.zoom());
        self.start_editing(next);
    }

    /// Enter on a single selected text starts editing it with everything selected.
    fn edit_selected_text(&mut self) -> bool {
        let [id] = self.tools.selection().ids() else {
            return false;
        };
        let id = *id;
        if self.document.get(id).is_none() || self.document.is_locked(id) {
            return false;
        }
        let zoom = self.camera.zoom();
        let engine = self.text.engine();
        let card = self.tools.selected_card(&self.document);
        if let Some((_, i)) = self.tools.selected_node(&self.document) {
            let node = match self.document.get(id).map(|o| &o.data.kind) {
                Some(ObjectKind::MindMap(map)) => map.nodes[i].id.clone(),
                _ => return false,
            };
            let engine = self.text.engine();
            let Some(mut edit) = TextEdit::node(engine, &self.document, id, &node) else {
                return false;
            };
            edit.select_all(engine);
            self.start_editing(edit);
            return true;
        }
        let edit = match (self.tools.table_cells(&self.document), card) {
            (Some((_, range)), _) => {
                TextEdit::table_cell(engine, &self.document, id, (range.row0, range.col0))
            }
            (None, Some((_, i))) => match self.document.get(id).map(|o| &o.data.kind) {
                Some(ObjectKind::Kanban(k)) => {
                    let card = k.cards[i].id.clone();
                    TextEdit::card(engine, &self.document, id, &card)
                }
                _ => None,
            },
            (None, None) => TextEdit::existing(engine, &self.document, id, None, zoom),
        };
        let Some(mut edit) = edit else {
            return self.start_crop(id);
        };
        edit.select_all(self.text.engine());
        self.start_editing(edit);
        true
    }

    /// A left press on the canvas that belongs to text editing, rather than the tools.
    /// Returns true if it was handled.
    fn text_press(&mut self, pointer: Pointer, screen: Point) -> bool {
        let now = Instant::now();
        let double = self.last_press.is_some_and(|(at, pos)| {
            now.duration_since(at) <= DOUBLE_CLICK_TIME
                && (pos - screen).hypot() <= DOUBLE_CLICK_DISTANCE
        });
        self.last_press = Some((now, screen));
        // While voting, a click on an object gives it a vote; with Shift it takes one back.
        if self.document.voting().is_some_and(|v| v.open) && self.tools.active() == ToolKind::Select
        {
            if let Some(id) = self
                .tools
                .object_at(&self.document, pointer)
                .filter(|&id| self.document.get(id).is_some())
            {
                self.apply(Command::Vote {
                    id,
                    author: self.me.clone(),
                    add: !pointer.shift,
                });
            }
            return true;
        }
        if self.editing.is_none()
            && let Some(thread) = self.comment_pin_at(screen)
        {
            self.thread_panel = Some(ThreadPanel {
                thread: Some(thread),
                anchor: None,
            });
            return true;
        }
        if self.tools.active() == ToolKind::Comment {
            self.tools.set_active(ToolKind::Select);
            let anchor = match self.tools.object_at(&self.document, pointer) {
                Some(id) if self.document.get(id).is_some() => pb_core::ThreadAnchor::Object(id),
                _ => pb_core::ThreadAnchor::Point(pointer.world),
            };
            self.finish_editing();
            self.thread_panel = Some(ThreadPanel {
                thread: None,
                anchor: Some(anchor),
            });
            return true;
        }
        // A click on the board puts the thread away.
        self.thread_panel = None;
        if let Some(source) = self.picking_link.take() {
            let target = self.tools.object_at(&self.document, pointer);
            if let Some(target) = target.filter(|&t| t != source) {
                self.apply(Command::SetLink {
                    ids: vec![source],
                    link: Some(Link::Object(target)),
                });
                self.tools.select_only([source]);
            }
            return true;
        }
        if self.editing.is_none()
            && self.tools.active() == ToolKind::Select
            && let Some(link) = self.link_at(screen)
        {
            self.follow(&link);
            return true;
        }
        if let Some(crop) = &mut self.cropping {
            if crop.press(pointer.world, pointer.zoom) {
                return true;
            }
            // A click beside the picture keeps the crop and then does what it normally does.
            self.finish_crop(true);
        }
        let pad = 4.0 / pointer.zoom;
        if let Some(edit) = &mut self.editing {
            let engine = self.text.engine();
            if edit.contains(engine, pointer.world, pad) {
                edit.press(engine, pointer.world, pointer.shift, double);
                return true;
            }
            // A click elsewhere ends editing and then does what it normally does.
            self.finish_editing();
        }
        if self.tools.active() == ToolKind::Text {
            // Like the shape tools, the text tool is one-shot.
            self.add_text_at(pointer.world);
            return true;
        }
        if self.tools.active() == ToolKind::Sticky {
            self.add_sticky_at(pointer.world);
            return true;
        }
        if self.tools.active() == ToolKind::MindMap {
            self.add_mindmap_at(pointer.world);
            return true;
        }
        if self.tools.active() == ToolKind::Doc {
            self.add_doc_at(pointer.world);
            return true;
        }
        if self.tools.active() == ToolKind::Select
            && let Some((id, col)) = self.tools.add_card_at(&self.document, pointer)
        {
            let at = match self.document.get(id).map(|o| &o.data.kind) {
                Some(ObjectKind::Kanban(k)) => k.cards_in(col).len(),
                _ => 0,
            };
            let engine = self.text.engine();
            if let Some(edit) = TextEdit::new_card(engine, &self.document, id, col, at) {
                self.start_editing(edit);
                return true;
            }
        }
        if double && self.tools.active() == ToolKind::Select {
            let target = self
                .tools
                .object_at(&self.document, pointer)
                .filter(|&id| !self.document.is_locked(id) && self.document.get(id).is_some());
            if let Some(id) = target {
                let engine = self.text.engine();
                if let Some(mut edit) = TextEdit::existing(
                    engine,
                    &self.document,
                    id,
                    Some(pointer.world),
                    pointer.zoom,
                ) {
                    edit.press(engine, pointer.world, false, false);
                    self.start_editing(edit);
                    return true;
                }
                // Double click on an image crops it, like in Miro.
                if self.start_crop(id) {
                    return true;
                }
            }
        }
        false
    }

    /// Sends the selected ink to handwriting recognition; the result comes back as
    /// [`UserEvent::Recognized`].
    fn recognize(&mut self, purpose: Recognition) {
        if self.recognition.is_some() {
            return;
        }
        let ids: Vec<_> = self
            .tools
            .selection()
            .ids()
            .iter()
            .copied()
            .filter(|&id| !self.document.is_locked(id))
            .collect();
        let strokes: Vec<_> = ids
            .iter()
            .flat_map(|&id| self.document.leaves(id))
            .collect();
        let Some(first) = strokes.first() else {
            return;
        };
        let color = first.data.style.stroke;
        let mut bounds: Option<pb_geom::kurbo::Rect> = None;
        let mut points = Vec::new();
        for object in &strokes {
            let ObjectKind::Ink(ink) = &object.data.kind else {
                return;
            };
            let frame = object.data.frame();
            let aabb = frame.aabb();
            bounds = Some(bounds.map_or(aabb, |b| b.union(aabb)));
            let stroke = ink
                .points_in(object.data.bounds)
                .into_iter()
                .map(|(p, _)| {
                    let p = frame.to_world(p);
                    (p.x, p.y)
                })
                .collect();
            points.push(stroke);
        }
        let Some(bounds) = bounds else {
            return;
        };
        let request = self.next_request;
        self.next_request += 1;
        self.recognition = Some(PendingRecognition {
            request,
            purpose,
            ids,
            bounds,
            color,
        });
        let proxy = self.proxy.clone();
        handwriting::recognize(points, move |result| {
            let _ = proxy.send_event(UserEvent::Recognized { request, result });
        });
    }

    fn finish_recognition(&mut self, request: u64, result: Result<String, String>) {
        let Some(pending) = self.recognition.take_if(|p| p.request == request) else {
            return;
        };
        let text = match result {
            Ok(text) if !text.trim().is_empty() => text.trim().to_owned(),
            Ok(_) => {
                dialog::show_error(
                    tr("Nothing recognized"),
                    tr("No text was found in the strokes."),
                );
                return;
            }
            Err(err) => {
                dialog::show_error(tr("Handwriting recognition failed"), &err);
                return;
            }
        };
        let b = pending.bounds;
        let size = (b.height() * HANDWRITING_TO_FONT_SIZE).max(4.0);
        match pending.purpose {
            Recognition::ToText => {
                // The strokes may have been deleted while recognition ran.
                let ids: Vec<_> = pending
                    .ids
                    .into_iter()
                    .filter(|&id| self.document.contains(id))
                    .collect();
                if ids.is_empty() {
                    return;
                }
                let data = self.text_object(&text, size, pending.color, |w, _| {
                    Point::new(b.center().x - w / 2.0, b.center().y)
                });
                // Both edits are one undo step.
                self.document.begin_undo_group();
                self.apply(Command::Delete(ids));
                self.apply(Command::Create(vec![data.into()]));
                self.document.end_undo_group();
            }
            Recognition::Calculate => match math::evaluate(&text) {
                Ok(value) => {
                    let answer = math::format_number(value);
                    let label = if math::ends_with_equals(&text) {
                        answer
                    } else {
                        format!("= {answer}")
                    };
                    let left = b.x1 + size * ANSWER_GAP;
                    let data = self.text_object(&label, size, pending.color, |_, _| {
                        Point::new(left, b.center().y)
                    });
                    self.apply(Command::Create(vec![data.into()]));
                }
                Err(err) => {
                    let message = trf("Recognized \"{}\": {}.", &[&text, &err]);
                    dialog::show_error(tr("Could not calculate"), &message);
                }
            },
        }
        self.request_redraw();
    }

    /// A text object sized to fit `text`. `place` gets the text's width and height and
    /// returns its left edge and vertical center.
    fn text_object(
        &mut self,
        text: &str,
        size: f64,
        color: Rgba,
        place: impl FnOnce(f64, f64) -> Point,
    ) -> ObjectData {
        let layout = self.text.engine().layout(text, size, None);
        let at = place(layout.width, layout.height);
        ObjectData {
            kind: ObjectKind::Text(Text {
                spans: Default::default(),
                text: text.into(),
                size,
            }),
            bounds: pb_geom::kurbo::Rect::new(
                at.x,
                at.y - layout.height / 2.0,
                at.x + layout.width,
                at.y + layout.height / 2.0,
            ),
            rotation: 0.0,
            style: Style {
                fill: Rgba(0),
                stroke: color,
                stroke_width: 0.0,
                opacity: 1.0,
                ..Style::default()
            },
            tags: Default::default(),
            link: None,
        }
    }

    fn end_style_drag(&mut self) {
        if self.style_drag {
            self.document.end_undo_group();
            self.style_drag = false;
        }
    }

    fn open_board(&mut self) {
        self.finish_editing();
        self.end_style_drag();
        if let Some(document) = self.session.open(&self.document) {
            self.replace_document(document);
        }
    }

    fn new_board(&mut self) {
        self.finish_editing();
        self.end_style_drag();
        if let Some(document) = self.session.new_board(&self.document) {
            self.replace_document(document);
        }
    }

    /// The pointer for tools, if it is over the canvas.
    fn pointer(&self) -> Option<Pointer> {
        let cursor = self.navigation.cursor()?;
        Some(Pointer {
            world: self.camera.screen_to_world(cursor),
            zoom: self.camera.zoom(),
            visible: self.camera.visible_world_rect(self.viewport()),
            pressure: self.pen_pressure.unwrap_or(1.0),
            shift: self.navigation.modifiers().shift_key(),
            ctrl: self.navigation.modifiers().control_key(),
            alt: self.navigation.modifiers().alt_key(),
        })
    }

    fn apply(&mut self, command: Command) {
        match self.document.apply(command) {
            // New objects get selected, like in Miro, and so do the parts of an ungrouped group.
            // Ink is not: the pen stays active for the next stroke.
            Ok(applied) if !applied.created.is_empty() => {
                if !self.tools.active().draws_ink() {
                    self.tools.select_only(applied.created);
                }
            }
            Ok(applied) if !applied.ungrouped.is_empty() => {
                self.tools.select_only(applied.ungrouped)
            }
            Ok(_) => {}
            Err(err) => log::error!("command failed: {err}"),
        }
        self.tools.sync(&self.document);
    }

    fn undo(&mut self) {
        self.finish_editing();
        self.end_style_drag();
        self.tools.cancel();
        if let Err(err) = self.document.undo() {
            log::error!("undo failed: {err}");
        }
        self.tools.sync(&self.document);
    }

    fn redo(&mut self) {
        self.finish_editing();
        self.tools.cancel();
        if let Err(err) = self.document.redo() {
            log::error!("redo failed: {err}");
        }
        self.tools.sync(&self.document);
    }

    /// Returns true if the view changed.
    fn handle_left_button(&mut self, state: ElementState) -> bool {
        let Some(pointer) = self.pointer() else {
            return false;
        };
        match state {
            ElementState::Pressed => {
                if let Some((id, side)) = self.tools.quick_dot_at(&self.document, pointer) {
                    self.quick_create(id, side);
                    return true;
                }
                // Drawing puts the pen's options away.
                if self.tools.active().draws_ink() {
                    self.ink_panel = false;
                }
                self.tools.pointer_down(&self.document, pointer)
            }
            ElementState::Released => match self.tools.pointer_up(&self.document, pointer) {
                Some(command) => {
                    let command = self.fit_resized_text(command);
                    self.apply(command);
                    true
                }
                // The gesture may still have changed the selection.
                None => true,
            },
        }
    }

    /// Text objects made wider or narrower get the height their text takes at the new
    /// width, from the top down, so lines that wrap do not spill out of the box.
    fn fit_resized_text(&mut self, command: Command) -> Command {
        let Command::SetGeometry(mut changes) = command else {
            return command;
        };
        for (id, frame) in &mut changes {
            let Some(object) = self.document.get(*id) else {
                continue;
            };
            let ObjectKind::Text(text) = &object.data.kind else {
                continue;
            };
            let rect = frame.rect.abs();
            if (rect.width() - object.data.bounds.width()).abs() < 0.5 {
                continue;
            }
            let layout = self.text.engine().layout_aligned(
                &text.text,
                &text.spans,
                text.size,
                // The same slack the text is drawn with.
                Some(rect.width() + 0.5),
                pb_text::TextAlign::Start,
            );
            frame.rect = pb_geom::kurbo::Rect::new(
                rect.x0,
                rect.y0,
                rect.x1,
                rect.y0 + layout.height.max(text.size),
            );
        }
        Command::SetGeometry(changes)
    }

    /// A "+" dot: adds a copy of `id` on `side`, connects the two, and starts typing on the
    /// copy. One undo step.
    fn quick_create(&mut self, id: ObjectId, side: pb_tools::quick::Side) {
        let zoom = self.camera.zoom();
        let Some(copy) = pb_tools::quick::copy_beside(&self.document, id, side, zoom) else {
            return;
        };
        let area = copy.frame().aabb();
        self.document.begin_undo_group();
        let created = self.document.apply(Command::Create(vec![copy.into()]));
        let new = match created {
            Ok(applied) => applied.created.first().copied(),
            Err(err) => {
                log::error!("quick create failed: {err}");
                None
            }
        };
        let route = self.tools.connector_route();
        if let Some(line) =
            new.and_then(|new| pb_tools::quick::link(&self.document, id, new, side, route))
            && let Err(err) = self.document.apply(Command::Create(vec![line.into()]))
        {
            log::error!("quick create failed: {err}");
        }
        self.document.end_undo_group();
        self.tools.sync(&self.document);
        let Some(new) = new else {
            return;
        };
        // Keep the copy in view.
        let viewport = self.viewport();
        if !self.camera.visible_world_rect(viewport).contains_rect(area) {
            self.camera.look_at(area.center(), zoom, viewport);
        }
        match TextEdit::existing(self.text.engine(), &self.document, new, None, zoom) {
            Some(edit) => self.start_editing(edit),
            None => self.tools.select_only([new]),
        }
    }

    fn cursor_icon(&mut self) -> CursorIcon {
        let over_text = match (&mut self.editing, self.navigation.cursor()) {
            (Some(edit), Some(cursor)) => {
                let world = self.camera.screen_to_world(cursor);
                edit.contains(self.text.engine(), world, 0.0)
            }
            _ => false,
        };
        if over_text || self.tools.active() == ToolKind::Text {
            return CursorIcon::Text;
        }
        if self.tools.active() == ToolKind::Sticky {
            return CursorIcon::Crosshair;
        }
        let hint = match (&self.cropping, self.navigation.cursor()) {
            (Some(crop), Some(cursor)) => {
                crop.cursor(self.camera.screen_to_world(cursor), self.camera.zoom())
            }
            _ => self.tools.cursor(),
        };
        match self.navigation.cursor_icon() {
            CursorIcon::Default => match hint {
                CursorHint::Default => CursorIcon::Default,
                CursorHint::Crosshair => CursorIcon::Crosshair,
                CursorHint::Move => CursorIcon::Move,
                CursorHint::Copy => CursorIcon::Copy,
                CursorHint::Rotate => CursorIcon::Grab,
                CursorHint::ResizeNs => CursorIcon::NsResize,
                CursorHint::ResizeEw => CursorIcon::EwResize,
                CursorHint::ResizeNwse => CursorIcon::NwseResize,
                CursorHint::ResizeNesw => CursorIcon::NeswResize,
            },
            icon => icon,
        }
    }

    /// A key while cropping: Enter keeps the crop and Escape drops it. Other keys keep it
    /// and do what they normally do, except modifiers (Shift keeps proportions) and Space
    /// (panning), which leave cropping on.
    fn crop_key(&mut self, event: &KeyEvent) -> bool {
        let pressed = event.state == ElementState::Pressed;
        match &event.logical_key {
            Key::Named(NamedKey::Enter) if pressed => self.finish_crop(true),
            Key::Named(NamedKey::Escape) if pressed => self.finish_crop(false),
            Key::Named(
                NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::Super
                | NamedKey::Space,
            ) => return self.handle_key(event),
            _ if !pressed => return self.handle_key(event),
            _ => {
                self.finish_crop(true);
                return self.handle_key(event);
            }
        }
        true
    }

    /// Applies a command built from the current selection, if there is one.
    fn apply_with(&mut self, build: impl FnOnce(&Tools, &Document) -> Option<Command>) {
        if let Some(command) = build(&self.tools, &self.document) {
            self.apply(command);
        }
    }

    /// Alt shortcuts for aligning and distributing (Miro's layout).
    fn handle_arrange_key(&mut self, code: KeyCode, shift: bool) -> bool {
        let align =
            |to| move |tools: &Tools, doc: &Document| pb_tools::align(doc, tools.selection(), to);
        let distribute = |axis| {
            move |tools: &Tools, doc: &Document| pb_tools::distribute(doc, tools.selection(), axis)
        };
        match code {
            KeyCode::KeyG => {
                self.apply_with(|tools, doc| pb_tools::arrange_grid(doc, tools.selection()));
            }
            KeyCode::KeyH if shift => self.apply_with(distribute(Axis::Horizontal)),
            KeyCode::KeyV if shift => self.apply_with(distribute(Axis::Vertical)),
            KeyCode::KeyA => self.apply_with(align(Align::Left)),
            KeyCode::KeyD => self.apply_with(align(Align::Right)),
            KeyCode::KeyW => self.apply_with(align(Align::Top)),
            KeyCode::KeyS => self.apply_with(align(Align::Bottom)),
            KeyCode::KeyH => self.apply_with(align(Align::CenterX)),
            KeyCode::KeyV => self.apply_with(align(Align::CenterY)),
            _ => return false,
        }
        true
    }

    fn reorder(&mut self, to: ZOrder) {
        let ids = self.tools.selection().ids().to_vec();
        if !ids.is_empty() {
            self.apply(Command::Reorder { ids, to });
        }
    }

    fn nudge(&mut self, dx: f64, dy: f64) {
        // Shift moves in bigger steps, like in Miro.
        let step = if self.navigation.modifiers().shift_key() {
            10.0
        } else {
            1.0
        };
        if let Some(command) = self
            .tools
            .nudge_selected(&self.document, Vec2::new(dx, dy) * step)
        {
            self.apply(command);
        }
    }

    /// Shows another board (opened or new) from a clean slate.
    fn replace_document(&mut self, document: Document) {
        self.document = document;
        // The new board's revisions start over: drop what the minimap kept of the old one.
        self.minimap_marks.borrow_mut().take();
        self.version_view = None;
        self.focus_saved = None;
        self.recording = None;
        self.playback = None;
        self.images.clear();
        self.tools.cancel();
        self.tools.clear_selection();
        self.tools.sync(&self.document);
        let viewport = self.viewport();
        self.camera.look_at(Point::ZERO, 1.0, viewport);
    }

    fn on_gpu_ready(&mut self, mut gpu: Gpu) {
        log::info!("drawing with {}", gpu.describe());
        // In the browser the GPU arrives later, after the canvas got its real size (it
        // starts at 1×1): resizes before then had no surface to go to.
        if let Some(window) = &self.window {
            let size = window.inner_size();
            gpu.resize(size.width, size.height);
        }
        pb_io::svg::warm_up();
        if let Some(ui) = &mut self.ui {
            ui.attach_gpu(&gpu);
        }
        self.gpu = Some(gpu);
        self.request_redraw();
    }

    fn handle_key(&mut self, event: &KeyEvent) -> bool {
        let PhysicalKey::Code(code) = event.physical_key else {
            return false;
        };
        if code == KeyCode::Space {
            self.navigation
                .set_space(event.state == ElementState::Pressed);
            return false;
        }
        if event.state != ElementState::Pressed {
            return false;
        }
        let modifiers = self.navigation.modifiers();
        let ctrl = modifiers.control_key();
        if modifiers.alt_key() && !ctrl {
            return self.handle_arrange_key(code, modifiers.shift_key());
        }
        let viewport = self.viewport();
        // Physical keys, so shortcuts work the same on any keyboard layout.
        match code {
            KeyCode::KeyS if ctrl && modifiers.shift_key() => {
                self.session.save_as(&self.document);
            }
            KeyCode::KeyS if ctrl => self.session.save(&self.document),
            KeyCode::KeyO if ctrl => self.open_board(),
            KeyCode::KeyE if ctrl && modifiers.shift_key() => self.open_export(),
            KeyCode::KeyP if ctrl => self.print(),
            // The UI's size: Ctrl+Alt+= / - / 0 (Ctrl+= and friends zoom the board).
            KeyCode::Equal if ctrl && modifiers.alt_key() => {
                self.perform(Action::SetUiScale(self.ui_scale + 0.25));
            }
            KeyCode::Minus if ctrl && modifiers.alt_key() => {
                self.perform(Action::SetUiScale(self.ui_scale - 0.25));
            }
            KeyCode::Digit0 if ctrl && modifiers.alt_key() => {
                self.perform(Action::SetUiScale(1.0));
            }
            KeyCode::KeyN if ctrl => self.new_board(),
            KeyCode::KeyZ if ctrl && modifiers.shift_key() => self.redo(),
            KeyCode::KeyZ if ctrl => self.undo(),
            KeyCode::KeyY if ctrl => self.redo(),
            KeyCode::KeyV if !ctrl => self.tools.set_active(ToolKind::Select),
            KeyCode::KeyR if !ctrl => self.tools.set_active(ToolKind::Shape(ShapeKind::Rectangle)),
            KeyCode::KeyO if !ctrl => self.tools.set_active(ToolKind::Shape(ShapeKind::Ellipse)),
            KeyCode::KeyS if !ctrl => self.tools.cycle_shape(),
            KeyCode::KeyT if !ctrl => self.tools.set_active(ToolKind::Text),
            KeyCode::KeyN if !ctrl => self.tools.set_active(ToolKind::Sticky),
            KeyCode::KeyL if !ctrl => self.tools.set_active(ToolKind::Connector),
            KeyCode::KeyF if ctrl => self.open_search(),
            KeyCode::F1 => self.show_shortcuts = !self.show_shortcuts,
            KeyCode::F5 => self.start_presentation(),
            KeyCode::Slash if modifiers.shift_key() && !ctrl => {
                self.show_shortcuts = !self.show_shortcuts;
            }
            KeyCode::KeyF if !ctrl && modifiers.shift_key() => self.toggle_focus(),
            KeyCode::KeyF if !ctrl => self.tools.set_active(ToolKind::Frame),
            KeyCode::KeyK if !ctrl => self.tools.set_active(ToolKind::Kanban),
            KeyCode::KeyD if !ctrl => self.tools.set_active(ToolKind::Doc),
            KeyCode::KeyC if !ctrl => self.tools.set_active(ToolKind::Comment),
            KeyCode::Enter | KeyCode::NumpadEnter if !ctrl => return self.edit_selected_text(),
            KeyCode::KeyP if !ctrl => self.tools.set_active(ToolKind::Pen),
            KeyCode::KeyM if !ctrl => self.tools.set_active(ToolKind::Marker),
            KeyCode::KeyE if !ctrl => self.tools.set_active(ToolKind::Eraser),
            KeyCode::KeyA if ctrl => self.tools.select_all(&self.document),
            KeyCode::KeyC if ctrl => {
                self.copy_selection();
            }
            KeyCode::KeyX if ctrl => {
                if self.copy_selection()
                    && let Some(command) = self.tools.delete_selected(&self.document)
                {
                    self.apply(command);
                }
            }
            // A held Ctrl+V or Ctrl+D would add copies some 30 times a second: one per press.
            KeyCode::KeyV | KeyCode::KeyD if ctrl && event.repeat => {}
            KeyCode::KeyV if ctrl => self.paste(),
            KeyCode::KeyG if ctrl && modifiers.shift_key() => {
                self.apply_with(Tools::ungroup_selected);
            }
            KeyCode::KeyG if ctrl => self.apply_with(Tools::group_selected),
            KeyCode::KeyL if ctrl && modifiers.shift_key() => {
                self.apply_with(Tools::toggle_lock_selected);
            }
            KeyCode::KeyH if ctrl && modifiers.shift_key() => self.toggle_hidden(),
            KeyCode::KeyD if ctrl => {
                if let Some(command) = pb_tools::duplicate(&self.document, self.tools.selection()) {
                    self.apply(command);
                }
            }
            KeyCode::BracketRight if ctrl => self.reorder(if modifiers.shift_key() {
                ZOrder::Front
            } else {
                ZOrder::Forward
            }),
            KeyCode::BracketLeft if ctrl => self.reorder(if modifiers.shift_key() {
                ZOrder::Back
            } else {
                ZOrder::Backward
            }),
            KeyCode::Tab if self.tools.selected_node(&self.document).is_some() => {
                self.add_node(true);
            }
            KeyCode::Enter | KeyCode::NumpadEnter
                if !ctrl && self.tools.selected_node(&self.document).is_some() =>
            {
                self.add_node(false);
            }
            KeyCode::F2 => return self.edit_selected_text(),
            KeyCode::Delete | KeyCode::Backspace
                if self.tools.selected_node(&self.document).is_some() =>
            {
                if let Some(command) = self.tools.remove_selected_node(&self.document) {
                    self.apply(command);
                }
            }
            KeyCode::Delete | KeyCode::Backspace
                if self.tools.selected_card(&self.document).is_some() =>
            {
                if let Some(command) = self.tools.remove_selected_card(&self.document) {
                    self.apply(command);
                }
            }
            KeyCode::Delete | KeyCode::Backspace
                if self.tools.table_cells(&self.document).is_some() =>
            {
                self.clear_cells();
            }
            KeyCode::Delete | KeyCode::Backspace => self.delete_selected(),
            KeyCode::ArrowLeft => self.nudge(-1.0, 0.0),
            KeyCode::ArrowRight => self.nudge(1.0, 0.0),
            KeyCode::ArrowUp => self.nudge(0.0, -1.0),
            KeyCode::ArrowDown => self.nudge(0.0, 1.0),
            // Esc backs out one level: picking a link target, gesture, tool, cells, selection.
            KeyCode::Escape => {
                let closed = self.context_menu.take().is_some()
                    || self.thread_panel.take().is_some()
                    || self.diagram.take().is_some()
                    || std::mem::replace(&mut self.show_templates, false)
                    || std::mem::replace(&mut self.show_export, false)
                    || std::mem::replace(&mut self.show_shortcuts, false);
                if !closed
                    && self.picking_link.take().is_none()
                    && !self.tools.cancel()
                    && !self.tools.clear_cells()
                {
                    if self.tools.active() != ToolKind::Select {
                        self.tools.set_active(ToolKind::Select);
                    } else if !self.tools.clear_selection() && self.focus_saved.is_some() {
                        self.stop_focus();
                    }
                }
            }
            KeyCode::Equal | KeyCode::NumpadAdd if ctrl => {
                self.navigation.zoom_in(&mut self.camera, viewport);
            }
            KeyCode::Minus | KeyCode::NumpadSubtract if ctrl => {
                self.navigation.zoom_out(&mut self.camera, viewport);
            }
            KeyCode::Digit0 | KeyCode::Numpad0 if ctrl => {
                self.navigation.zoom_reset(&mut self.camera, viewport);
            }
            KeyCode::Home => self.camera.look_at(Point::ZERO, 1.0, viewport),
            KeyCode::KeyG if modifiers.shift_key() && !ctrl && !event.repeat => {
                self.tools.set_snap_to_grid(!self.tools.snap_to_grid());
            }
            KeyCode::KeyG if !ctrl && !event.repeat => self.grid = self.grid.next(),
            _ => return false,
        }
        true
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("PaintBoard")
            .with_window_icon(window_icon())
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        #[cfg(target_arch = "wasm32")]
        let attributes = {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes.with_append(true)
        };
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create window"),
        );
        self.window = Some(window.clone());
        self.ui = Some(Ui::new(&window));
        // The language and theme picked before, or the system's at first.
        let system_lang = pb_platform::locale().and_then(|l| crate::i18n::Lang::from_code(&l));
        crate::i18n::set_lang(
            crate::settings::language()
                .or(system_lang)
                .unwrap_or(crate::i18n::Lang::En),
        );
        let system_dark = window.theme() == Some(winit::window::Theme::Dark);
        self.set_dark(crate::settings::dark_theme().unwrap_or(system_dark));
        let (scale, contrast) = crate::settings::view();
        self.set_view(scale, contrast);
        self.camera.look_at(Point::ZERO, 1.0, self.viewport());

        let size = window.inner_size();
        let gpu = Gpu::new(window, size.width, size.height);

        #[cfg(not(target_arch = "wasm32"))]
        match pollster::block_on(gpu) {
            Ok(gpu) => self.on_gpu_ready(gpu),
            Err(err) => {
                log::error!("failed to initialize GPU: {err}");
                event_loop.exit();
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            let proxy = self.proxy.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match gpu.await {
                    Ok(gpu) => {
                        let _ = proxy.send_event(UserEvent::GpuReady(Box::new(gpu)));
                    }
                    Err(err) => log::error!("failed to initialize GPU (WebGPU required): {err}"),
                }
            });
        }
    }

    /// Runs after each batch of events: autosaves when due and wakes up for the next one.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.dropped.is_empty() {
            let paths = std::mem::take(&mut self.dropped);
            // Windows sends no cursor moves during a drag: this is the last known position.
            let at = self
                .navigation
                .cursor()
                .map(|c| self.camera.screen_to_world(c));
            self.insert_files(&paths, at);
        }
        let now = Instant::now();
        self.session.tick(&self.document, now);
        let ui_repaint = self.ui.as_ref().and_then(Ui::repaint_at);
        if ui_repaint.is_some_and(|at| at <= now) {
            self.request_redraw();
        }
        let blink = self.editing.as_ref().and_then(|edit| {
            if edit.needs_blink_redraw(now) {
                self.request_redraw();
            }
            edit.next_blink(now)
        });
        // A running timer shows each second; a finished one blinks.
        let timer = self.timer.as_ref().and_then(|t| t.next_tick(now));
        // Recording samples the view, and shows each second.
        let viewport = self.viewport();
        let center = self.camera.visible_world_rect(viewport).center();
        let zoom = self.camera.zoom();
        let pointer = self
            .navigation
            .cursor()
            .map(|c| self.camera.screen_to_world(c));
        let mut redraw = false;
        let sampling = self.recording.as_mut().map(|recording| {
            recording.sample(now, center, zoom, pointer);
            redraw = recording.second_passed(now);
            recording.next_key()
        });
        if redraw {
            self.request_redraw();
        }
        if timer.is_some_and(|at| at <= now) {
            self.request_redraw();
        }
        let wake = [
            self.session.deadline(),
            ui_repaint.filter(|&at| at > now),
            blink,
            timer.filter(|&at| at > now),
            sampling,
        ]
        .into_iter()
        .flatten()
        .min();
        event_loop.set_control_flow(match wake {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.session.flush(&self.document);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::GpuReady(gpu) => self.on_gpu_ready(*gpu),
            UserEvent::Recognized { request, result } => self.finish_recognition(request, result),
            UserEvent::Previewed {
                id,
                url,
                replace_title,
                result,
            } => self.finish_preview(id, &url, replace_title, result),
            // Waking up is all it takes: `about_to_wait` ticks the session.
            UserEvent::Saved => {}
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        self.handle_window_event(event_loop, event, true);
    }
}

impl App {
    /// Windows delivers pens and touch screens as touch events only: winit takes their
    /// pointer messages, so Windows makes no mouse messages from them. The first pen or
    /// finger drives the canvas like the left mouse button, with the pen's pressure. egui
    /// has had the touch event itself, so these mouse events are not passed to it again.
    ///
    /// TODO: more fingers for pinch zoom and two-finger panning on touch screens.
    fn touch_as_mouse(&mut self, event_loop: &ActiveEventLoop, touch: Touch) {
        if self.touch.is_some_and(|id| id != touch.id) {
            return;
        }
        let device_id = touch.device_id;
        let moved = WindowEvent::CursorMoved {
            device_id,
            position: touch.location,
        };
        let button = |state| WindowEvent::MouseInput {
            device_id,
            state,
            button: MouseButton::Left,
        };
        let pressure = touch.force.map(|f| f.normalized());
        match touch.phase {
            TouchPhase::Started => {
                self.touch = Some(touch.id);
                self.pen_pressure = pressure;
                self.handle_window_event(event_loop, moved, false);
                self.handle_window_event(event_loop, button(ElementState::Pressed), false);
            }
            TouchPhase::Moved => {
                // A pen also reports moves while it hovers above the screen.
                if self.touch.is_some() {
                    self.pen_pressure = pressure;
                }
                self.handle_window_event(event_loop, moved, false);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.handle_window_event(event_loop, moved, false);
                self.handle_window_event(event_loop, button(ElementState::Released), false);
                self.touch = None;
                self.pen_pressure = None;
            }
        }
    }

    /// A window event; `feed_ui` passes it to egui first.
    fn handle_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: WindowEvent,
        feed_ui: bool,
    ) {
        let scale = self.scale();
        let mut ui_consumed = false;
        // Typing on the canvas is not for egui, which would take Tab to move its focus.
        // Nor is Tab anywhere else unless a text field has focus: on the board it adds
        // sticky notes, cells and topics.
        let ui_typing = self.ui.as_ref().is_some_and(Ui::wants_keyboard);
        let board_tab = !ui_typing
            && matches!(
                &event,
                WindowEvent::KeyboardInput { event, .. }
                    if event.logical_key == Key::Named(NamedKey::Tab)
            );
        let canvas_typing = board_tab
            || self.editing.is_some()
                && matches!(
                    event,
                    WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_)
                );
        if feed_ui
            && !canvas_typing
            && let (Some(ui), Some(window)) = (&mut self.ui, &self.window)
        {
            let response = ui.on_window_event(window, &event);
            // egui asks for a repaint after every frame drawn; following that would redraw
            // nonstop, keeping the GPU busy while nothing changes.
            let drawn = matches!(event, WindowEvent::RedrawRequested);
            if response.repaint && !drawn {
                window.request_redraw();
            }
            ui_consumed = response.consumed;
        }
        let over_ui = self.ui.as_ref().is_some_and(Ui::wants_pointer) && !self.canvas_drag;
        let pointer_or_keys = matches!(
            event,
            WindowEvent::CursorMoved { .. }
                | WindowEvent::CursorEntered { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::KeyboardInput { .. }
                | WindowEvent::ModifiersChanged(_)
                | WindowEvent::Touch(_)
        );
        let changed = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                false
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                false
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                true
            }
            WindowEvent::ScaleFactorChanged { .. } => true,
            WindowEvent::ModifiersChanged(modifiers) => {
                self.navigation.set_modifiers(modifiers.state());
                // Shift changes the shape being drawn without moving the mouse.
                self.pointer()
                    .is_some_and(|p| self.tools.pointer_move(&self.document, p))
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let typing = self.ui.as_ref().is_some_and(Ui::wants_keyboard);
                if typing {
                    false
                } else if self.playback.is_some() {
                    self.playback_key(&event)
                } else if self.presentation.is_some() {
                    self.presentation_key(&event)
                } else if self.version_view.is_some() {
                    // Looking only: Esc goes back to the board.
                    let escape = event.state == ElementState::Pressed
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape);
                    if escape {
                        self.version_view = None;
                    }
                    escape
                } else if self.cropping.is_some() {
                    self.crop_key(&event)
                } else if let Some(edit) = &mut self.editing {
                    let modifiers = self.navigation.modifiers();
                    match edit.key(self.text.engine(), &event, modifiers) {
                        KeyOutcome::Handled => {}
                        KeyOutcome::Finish => {
                            // Escape from a cell or card leaves it selected.
                            let cell = edit.edited_cell();
                            let card = edit.edited_card();
                            let node = edit.edited_node();
                            self.finish_editing();
                            if let Some((id, cell)) = cell {
                                self.tools.select_cell(id, cell);
                            }
                            if let Some((id, card)) = card {
                                self.tools.select_card(id, card);
                                self.tools.sync(&self.document);
                            }
                            if let Some((id, node)) = node {
                                self.tools.select_node(id, node);
                                self.tools.sync(&self.document);
                            }
                        }
                        KeyOutcome::FinishAndPass => {
                            self.finish_editing();
                            self.handle_key(&event);
                        }
                        KeyOutcome::NextSticky { below } => self.next_sticky(below),
                        KeyOutcome::NextCell { back } => self.next_cell(back),
                        KeyOutcome::NextCard => self.next_card(),
                        KeyOutcome::NextNode { child } => self.next_node(child),
                    }
                    true
                } else {
                    self.handle_key(&event)
                }
            }
            WindowEvent::Ime(ime) => match &mut self.editing {
                Some(edit) => {
                    edit.ime(self.text.engine(), &ime);
                    true
                }
                None => false,
            },
            WindowEvent::CursorMoved { position, .. } => {
                let pos = position.to_logical::<f64>(scale);
                let panned = self
                    .navigation
                    .cursor_moved(&mut self.camera, Point::new(pos.x, pos.y));
                if let Some(p) = &mut self.presentation
                    && p.laser
                    && !over_ui
                {
                    let world = self.camera.screen_to_world(Point::new(pos.x, pos.y));
                    p.trail.push((world, Instant::now()));
                }
                let pointer = self.pointer();
                let text_drag = match (&mut self.editing, pointer) {
                    (Some(edit), Some(p)) => edit.drag(self.text.engine(), p.world),
                    _ => false,
                };
                let text_drag = text_drag
                    || match (&mut self.cropping, pointer) {
                        (Some(crop), Some(p)) if crop.is_dragging() => {
                            crop.drag_to(p.world, p.shift);
                            true
                        }
                        _ => false,
                    };
                let tool_changed = if text_drag {
                    true
                } else if over_ui {
                    self.tools.clear_hover()
                } else {
                    self.pointer()
                        .is_some_and(|p| self.tools.pointer_move(&self.document, p))
                };
                panned || tool_changed
            }
            WindowEvent::CursorLeft { .. } => {
                self.navigation.cursor_left();
                false
            }
            WindowEvent::MouseInput { state, button, .. } => {
                // Presses on the UI stay there; releases always reach the canvas so that a
                // drag started on it ends even over a toolbar. While presenting the board
                // can only be panned (middle button) and zoomed.
                let ui_press = state == ElementState::Pressed && (ui_consumed || over_ui);
                let presenting = self.playback.is_some()
                    || (self.presentation.is_some() || self.version_view.is_some())
                        && button != MouseButton::Middle;
                if ui_press || presenting {
                    false
                } else {
                    let consumed = self.navigation.mouse_button(button, state);
                    let left = button == MouseButton::Left;
                    // A press on the board closes the right-click menu; a right click
                    // opens it.
                    let menu_was_open =
                        state == ElementState::Pressed && self.context_menu.take().is_some();
                    let right_click = !consumed
                        && button == MouseButton::Right
                        && state == ElementState::Released;
                    if right_click {
                        self.open_context_menu();
                    }
                    if left {
                        self.canvas_drag = state == ElementState::Pressed && !consumed;
                    }
                    if !consumed && left && state == ElementState::Released {
                        if let Some(edit) = &mut self.editing {
                            edit.release();
                        }
                        if let Some(crop) = &mut self.cropping {
                            crop.release();
                        }
                    }
                    let screen = self.navigation.cursor();
                    let text = !consumed
                        && left
                        && state == ElementState::Pressed
                        && match (self.pointer(), screen) {
                            (Some(p), Some(s)) => self.text_press(p, s),
                            _ => false,
                        };
                    menu_was_open
                        || right_click
                        || text
                        || (!consumed && left && self.handle_left_button(state))
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // A talktrack playing moves the view itself.
                if ui_consumed || over_ui || self.playback.is_some() {
                    false
                } else {
                    self.navigation.wheel(&mut self.camera, delta, scale);
                    true
                }
            }
            WindowEvent::Touch(touch) => {
                self.touch_as_mouse(event_loop, touch);
                false
            }
            WindowEvent::DroppedFile(path) => {
                self.dropped.push(path);
                false
            }
            WindowEvent::PinchGesture { delta, .. } => {
                self.navigation.pinch(&mut self.camera, delta);
                true
            }
            _ => false,
        };
        // Over the UI, egui sets the cursor. Only input changes it: on the web, winit sends
        // some events (Resized) while it holds the canvas the cursor is set on.
        if pointer_or_keys && !over_ui {
            let icon = self.cursor_icon();
            if let Some(window) = &self.window {
                window.set_cursor(icon);
            }
        }
        if changed {
            self.request_redraw();
        }
    }
}

/// A countdown.
struct Timer {
    duration: pb_platform::Duration,
    /// When it last started running; `None` while paused.
    started: Option<Instant>,
    /// Time run before the last start.
    run: pb_platform::Duration,
}

impl Timer {
    /// A countdown of `duration`, running from now.
    fn new(duration: pb_platform::Duration) -> Self {
        Self {
            duration,
            started: Some(Instant::now()),
            run: pb_platform::Duration::ZERO,
        }
    }

    fn elapsed(&self, now: Instant) -> pb_platform::Duration {
        self.run
            + self
                .started
                .map_or(pb_platform::Duration::ZERO, |s| now.duration_since(s))
    }

    /// Pauses it, or runs it on.
    fn toggle(&mut self, now: Instant) {
        match self.started.take() {
            Some(start) => self.run += now.duration_since(start),
            None => self.started = Some(now),
        }
    }

    /// What it shows at `now`.
    fn view(&self, now: Instant) -> crate::ui::TimerView {
        let elapsed = self.elapsed(now);
        let left = self.duration.saturating_sub(elapsed);
        // Rounded up, so it reads 0:00 only when time is up.
        let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        let done = left.is_zero();
        let over = elapsed.saturating_sub(self.duration);
        crate::ui::TimerView {
            text: format!("{}:{:02}", secs / 60, secs % 60),
            running: self.started.is_some(),
            done,
            // Blinks twice a second once time is up.
            flash: done && over.subsec_millis() < 500,
        }
    }

    /// When it next needs drawing: at the next whole second while running, every half
    /// second once time is up; `None` while paused.
    fn next_tick(&self, now: Instant) -> Option<Instant> {
        self.started?;
        let elapsed = self.elapsed(now);
        let step = if elapsed >= self.duration {
            pb_platform::Duration::from_millis(500)
        } else {
            pb_platform::Duration::from_secs(1)
        };
        let into = pb_platform::Duration::from_nanos((elapsed.as_nanos() % step.as_nanos()) as u64);
        Some(now + (step - into))
    }
}

/// Margin around a presented frame, in logical pixels.
const PRESENT_MARGIN: f64 = 24.0;
/// How long the camera takes to glide to a new place.
const GLIDE: pb_platform::Duration = pb_platform::Duration::from_millis(350);

/// Presenting the board frame by frame.
struct Presentation {
    /// The frame shown, an index into `frames`.
    index: usize,
    /// The frames, in order, when the presentation started.
    frames: Vec<ObjectId>,
    /// Where the board was before, to go back to.
    saved: Camera,
    /// The pointer is a laser dot.
    laser: bool,
    /// Where the laser has been lately (world), oldest first.
    trail: Vec<(Point, Instant)>,
}

/// How long the laser's trail lasts.
const LASER_TRAIL: pb_platform::Duration = pb_platform::Duration::from_millis(700);

/// The camera gliding from one view to another: centers (world) and zooms.
struct Glide {
    from: (Point, f64),
    to: (Point, f64),
    start: Instant,
}

impl Glide {
    /// Where the glide is at `now`: center, zoom, and whether it is over.
    fn at(&self, now: Instant) -> (Point, f64, bool) {
        let t = (now.duration_since(self.start).as_secs_f64() / GLIDE.as_secs_f64()).min(1.0);
        // Ease in and out.
        let s = t * t * (3.0 - 2.0 * t);
        let center = self.from.0.lerp(self.to.0, s);
        let zoom = self.from.1 * (self.to.1 / self.from.1).powf(s);
        (center, zoom, t >= 1.0)
    }
}

/// The comment thread in the panel.
struct ThreadPanel {
    /// `None` until the first comment starts it.
    thread: Option<Arc<str>>,
    /// Where a new thread goes.
    anchor: Option<pb_core::ThreadAnchor>,
}

/// How long ago `at_ms` was at `now_ms`, for people: "just now", "5 min ago", "3 h ago",
/// "2 d ago", or the date.
fn time_ago(now_ms: i64, at_ms: i64) -> String {
    let minutes = (now_ms - at_ms).max(0) / 60_000;
    match minutes {
        0 => tr("just now").to_owned(),
        1..60 => trf("{} min ago", &[&minutes]),
        60..1440 => trf("{} h ago", &[&(minutes / 60)]),
        1440..10080 => trf("{} d ago", &[&(minutes / 1440)]),
        _ => {
            // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
            let z = at_ms.div_euclid(86_400_000) + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let day = doy - (153 * mp + 2) / 5 + 1;
            let month = if mp < 10 { mp + 3 } else { mp - 9 };
            let year = yoe + era * 400 + i64::from(month <= 2);
            format!("{year}-{month:02}-{day:02}")
        }
    }
}

/// What a new document starts with.
const DOC_TEMPLATE: &str = "# Title

Write **Markdown** here: *italic*, `code`, [links](https://example.com).

- lists
- and more
";

/// PDF files put on the board get at most this many pages.
const MAX_PDF_PAGES: usize = 30;

/// Whether `path` is a PDF file, whose pages are added as pictures.
fn is_pdf_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// The app's icon for its windows, from `assets/icon.png`.
fn window_icon() -> Option<winit::window::Icon> {
    const PNG: &[u8] = include_bytes!("../assets/icon.png");
    let pixels = pb_io::image::decode(PNG, 256).ok()?;
    winit::window::Icon::from_rgba(pixels.rgba, pixels.width, pixels.height).ok()
}

/// Deleting this many objects at once asks first...
const DELETE_ASK: usize = 20;
/// ...and so does deleting everything on a board of at least this many.
const DELETE_ASK_ALL: usize = 5;

/// The most objects the minimap draws one by one; boards with more are drawn by cell.
const MINIMAP_MAX_MARKS: usize = 1500;

/// Cells along the longer side of the minimap's grid for large boards.
const MINIMAP_CELLS: u32 = 100;

/// `name` made fit for a file name: characters Windows does not allow become `_`.
fn file_name(name: &str) -> String {
    name.chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect()
}

fn has_extension(path: &std::path::Path, extension: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
}

/// Whether `path` is a CSV or TSV file, added as a table.
fn is_table_file(path: &std::path::Path) -> bool {
    has_extension(path, "csv") || has_extension(path, "tsv")
}

/// Most rows and columns a table made from a file or paste gets; the rest is cut.
const MAX_TABLE_ROWS: usize = 500;
const MAX_TABLE_COLS: usize = 50;

/// Space around cell text, and the widest a column of a pasted table starts out, in world
/// units at 100%.
const TABLE_CELL_PAD: f64 = 8.0;
const TABLE_MAX_COLUMN: f64 = 280.0;

/// Whether `path` is a Markdown or plain text file, added as a document.
fn is_document_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["md", "markdown", "txt"].contains(&e.to_ascii_lowercase().as_str()))
}

/// Font size and wrap width of diagram text made from Mermaid, in world units at 100%.
const DIAGRAM_FONT: f64 = 16.0;
const DIAGRAM_WRAP: f64 = 200.0;

/// Searching the board.
#[derive(Default)]
struct Search {
    query: String,
    hits: Vec<pb_core::search::Hit>,
    /// The result shown now.
    current: usize,
    /// The board's revision the results are for.
    revision: u64,
}

/// Selection feedback, following objects mid-gesture.
fn overlay(document: &Document, tools: &Tools, overrides: &[Object], zoom: f64) -> Overlay {
    let overrides: HashMap<_, _> = overrides.iter().map(|o| (o.id, &o.data)).collect();
    let frame = |o: &Object| overrides.get(&o.id).copied().unwrap_or(&o.data).frame();
    let ids = tools.selection().ids();
    let selection = selection_frame(document, ids, frame);
    let selected = match ids {
        [id] if document.group(*id).is_some() => {
            document.leaves(*id).into_iter().map(frame).collect()
        }
        [_] => Vec::new(),
        _ => ids
            .iter()
            .filter_map(|&id| unit_frame(document, id, frame))
            .collect(),
    };
    let show_handles = tools.shows_handles(document);
    // Connectors are outlined along their line rather than with a frame, broken where
    // their label is (as it is mid-gesture or mid-edit).
    let connector_path = |id: ObjectId| {
        let object = document.get(id)?;
        let ObjectKind::Connector(c) = &overrides.get(&id).copied().unwrap_or(&object.data).kind
        else {
            return None;
        };
        let path = document.connector_path(id)?;
        let gap = c.label.rect(path).map(|r| {
            let pad = pb_core::Label::PADDING;
            r.inflate(pad, pad)
        });
        Some((path.line.clone(), gap))
    };
    let mut lines = Vec::new();
    let mut gaps = Vec::new();
    let mut hover = tools.hover().and_then(|id| unit_frame(document, id, frame));
    if let Some((line, gap)) = tools.hover().and_then(connector_path) {
        lines.push(line);
        gaps.extend(gap);
        hover = None;
    }
    let end_handles = tools.connector_handles(document);
    let mut selection = selection;
    if let Some((line, gap)) = end_handles
        .and(ids.first().copied())
        .and_then(connector_path)
    {
        lines.push(line);
        gaps.extend(gap);
        selection = None;
    }
    let (connection_points, connection_active) = tools.connection_hints(document);
    Overlay {
        selected,
        selection,
        show_handles,
        locked: tools.selection().has_locked(document),
        rotate_knob: selection
            .filter(|_| show_handles && tools.can_rotate(document))
            .map(|f| rotation_knob(f, zoom)),
        hover,
        cells: tools.cells_frame(document),
        marquee: tools.marquee(),
        guides: tools.guides(document),
        eraser: tools.eraser(),
        text_edit: None,
        quick_dots: if show_handles {
            tools.quick_dots(document, zoom)
        } else {
            Vec::new()
        },
        quick_dot_radius: pb_tools::quick::DOT_RADIUS,
        link_badges: Vec::new(),
        comment_pins: Vec::new(),
        spotlight: None,
        laser: Vec::new(),
        search_hits: Vec::new(),
        marked: Vec::new(),
        search_current: None,
        lines,
        gaps,
        end_handles: end_handles.map(Vec::from).unwrap_or_default(),
        bend_handles: tools.bend_handles(document),
        connection_points,
        connection_active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timers_count_down_pause_and_run_out() {
        use pb_platform::Duration;
        let t0 = Instant::now();
        let secs = |s: u64| t0 + Duration::from_secs(s);
        let mut timer = Timer {
            duration: Duration::from_secs(90),
            started: Some(t0),
            run: Duration::ZERO,
        };
        assert_eq!(timer.view(t0).text, "1:30");
        assert_eq!(timer.view(t0 + Duration::from_millis(500)).text, "1:30");
        assert_eq!(timer.view(secs(31)).text, "0:59");
        // Paused for a minute, it does not move.
        timer.toggle(secs(31));
        assert!(!timer.view(secs(91)).running);
        assert_eq!(timer.view(secs(91)).text, "0:59");
        assert_eq!(timer.next_tick(secs(91)), None);
        timer.toggle(secs(91));
        assert_eq!(timer.view(secs(100)).text, "0:50");
        // Next tick on the next whole second of running time.
        assert_eq!(
            timer.next_tick(secs(100) + Duration::from_millis(300)),
            Some(secs(101))
        );
        let up = timer.view(secs(150) + Duration::from_millis(100));
        assert!(up.done && up.flash);
        assert!(!timer.view(secs(150) + Duration::from_millis(600)).flash);
    }

    #[test]
    fn times_read_like_people_say_them() {
        let now = 1_760_000_000_000; // 2025-10-09
        assert_eq!(time_ago(now, now - 10_000), "just now");
        assert_eq!(time_ago(now, now - 5 * 60_000), "5 min ago");
        assert_eq!(time_ago(now, now - 3 * 3_600_000), "3 h ago");
        assert_eq!(time_ago(now, now - 2 * 86_400_000), "2 d ago");
        assert_eq!(time_ago(now, 0), "1970-01-01");
        assert_eq!(time_ago(now, 951_782_400_000), "2000-02-29");
        // Clocks a little apart do not make the future.
        assert_eq!(time_ago(now, now + 5_000), "just now");
    }
}
