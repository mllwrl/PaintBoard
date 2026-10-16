use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pb_core::{
    Command, Document, Object, ObjectData, ObjectId, ObjectKind, Rgba, ShapeKind, Sticky, Style,
    Text, ZOrder,
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
use crate::navigation::Navigation;
use crate::pictures::{self, Picture};
use crate::session::Session;
use crate::ui::{Action, Minimap, Model, STICKY_COLORS, SelectionInfo, ShapeText, TextFormat, Ui};

pub enum UserEvent {
    /// GPU initialization finished (asynchronous on the web).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    GpuReady(Box<Gpu>),
    /// Handwriting recognition finished on its background thread.
    Recognized {
        request: u64,
        result: Result<String, String>,
    },
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
    /// Time and screen position of the last left press, to detect double clicks.
    last_press: Option<(Instant, Point)>,
    recognition: Option<PendingRecognition>,
    next_request: u64,
    shown_title: String,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let (session, document) = Session::restore();
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
                }),
                editing::Target::New => previews.push(data),
            }
        }
        let mut overlay = overlay(&self.document, &self.tools, &overrides, self.camera.zoom());
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
        build_scene(
            &mut self.scene,
            &View {
                camera: &self.camera,
                viewport: Size::new(size.width, size.height),
                scale,
                grid: self.grid,
                theme: &self.theme,
                objects: self.document.objects(),
                assets: self.document.assets(),
                overrides: &overrides,
                hidden: &hidden,
                previews: &previews,
                overlay: &overlay,
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
        let model = Model {
            board: &board,
            frames,
            tool: self.tools.active(),
            last_shape: self.tools.last_shape(),
            ink: self
                .tools
                .ink_style(self.tools.active() == ToolKind::Marker),
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
        };
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
        let objects = self.document.objects();
        // Frames under everything, like on the board.
        let shown = objects
            .iter()
            .filter(|o| o.data.kind.is_frame())
            .chain(objects.iter().filter(|o| !o.data.kind.is_frame()));
        let mut content = None;
        let mut marks = Vec::new();
        for o in shown {
            let data = &o.data;
            let bounds = data.frame().aabb();
            content = Some(content.map_or(bounds, |c: pb_geom::kurbo::Rect| c.union(bounds)));
            let color = match data.kind {
                ObjectKind::Shape(_) | ObjectKind::Sticky(_)
                    if data.style.fill.to_bytes()[3] > 0 =>
                {
                    data.style.fill
                }
                ObjectKind::Frame(_) => Rgba::rgb(0xFF, 0xFF, 0xFF),
                ObjectKind::Image(_) => Rgba::rgb(0xB8, 0xB8, 0xB2),
                // Lines and strokes would show as boxes.
                ObjectKind::Connector(_) | ObjectKind::Ink(_) => continue,
                _ => Rgba::rgb(0xC8, 0xC8, 0xC2),
            };
            marks.push((bounds, color));
        }
        let content = content?;
        let view = self.camera.visible_world_rect(self.viewport());
        let area = content.union(view);
        let pad = area.width().max(area.height()) * 0.04;
        Some(Minimap {
            area: area.inflate(pad, pad),
            objects: marks,
            view,
        })
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
                    ObjectKind::Shape(_) | ObjectKind::Sticky(_) | ObjectKind::Frame(_)
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
            stroke_label: if leaves
                .iter()
                .any(|o| matches!(o.data.kind, ObjectKind::Shape(_)))
            {
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
            can_crop: matches!(ids, [id] if Crop::new(&self.document, *id).is_some()),
            several: ids.len() > 1,
            locked: selection.has_locked(&self.document),
        })
    }

    /// Carries out a UI action.
    fn perform(&mut self, action: Action) {
        let viewport = self.viewport();
        match action {
            Action::Tool(tool) => {
                self.finish_editing();
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
            Action::Duplicate => {
                self.apply_with(|tools, doc| pb_tools::duplicate(doc, tools.selection()));
            }
            Action::Delete => self.apply_with(Tools::delete_selected),
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

    /// Asks for image files and places them in the middle of the view.
    fn insert_image_files(&mut self) {
        let paths = dialog::pick_open_many("Images", pb_io::image::EXTENSIONS);
        self.insert_files(&paths, None);
    }

    /// Places image files at `at` (world), or in the middle of the view. Other files are
    /// skipped; unreadable images are reported.
    fn insert_files(&mut self, paths: &[PathBuf], at: Option<Point>) {
        let mut errors = Vec::new();
        let pictures: Vec<_> = paths
            .iter()
            .filter(|p| pictures::is_image_file(p))
            .filter_map(|p| Picture::from_file(p).map_err(|e| errors.push(e)).ok())
            .collect();
        self.insert_pictures(pictures, at);
        if !errors.is_empty() {
            dialog::show_error("Some images could not be added", &errors.join("\n"));
        }
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
        if let Some(image) = clipboard::image() {
            match Picture::from_clipboard(image) {
                Ok(picture) => self.insert_pictures(vec![picture], at),
                Err(err) => dialog::show_error("Could not paste the image", &err.to_string()),
            }
            return;
        }
        let files = clipboard::files();
        if files.iter().any(|f| pictures::is_image_file(f)) {
            self.insert_files(&files, at);
            return;
        }
        for bytes in self.clipboard.assets().to_vec() {
            self.document.add_asset(bytes);
        }
        if let Some(command) = self.clipboard.paste(at) {
            self.apply(command);
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
        let Some(mut edit) = TextEdit::existing(self.text.engine(), &self.document, id, None, zoom)
        else {
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
            self.tools.set_active(ToolKind::Select);
            let size = NEW_TEXT_SIZE / pointer.zoom;
            let color = Style::default().stroke;
            let edit = TextEdit::new_text(self.text.engine(), pointer.world, size, color);
            self.start_editing(edit);
            return true;
        }
        if self.tools.active() == ToolKind::Sticky {
            self.tools.set_active(ToolKind::Select);
            let side = Sticky::SIZE / pointer.zoom;
            let bounds = pb_geom::kurbo::Rect::from_center_size(pointer.world, (side, side));
            let style = Style {
                fill: self.sticky_color,
                stroke_width: 0.0,
                ..Style::default()
            };
            let edit = TextEdit::new_sticky(self.text.engine(), bounds, style);
            self.start_editing(edit);
            return true;
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
                dialog::show_error("Nothing recognized", "No text was found in the strokes.");
                return;
            }
            Err(err) => {
                dialog::show_error("Handwriting recognition failed", &err);
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
                    let message = format!("Recognized \"{text}\": {err}.");
                    dialog::show_error("Could not calculate", &message);
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
                self.tools.pointer_down(&self.document, pointer)
            }
            ElementState::Released => match self.tools.pointer_up(&self.document, pointer) {
                Some(command) => {
                    self.apply(command);
                    true
                }
                // The gesture may still have changed the selection.
                None => true,
            },
        }
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
        self.images.clear();
        self.tools.cancel();
        self.tools.clear_selection();
        self.tools.sync(&self.document);
        let viewport = self.viewport();
        self.camera.look_at(Point::ZERO, 1.0, viewport);
    }

    fn on_gpu_ready(&mut self, gpu: Gpu) {
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
            KeyCode::KeyF if !ctrl => self.tools.set_active(ToolKind::Frame),
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
            KeyCode::KeyV if ctrl => self.paste(),
            KeyCode::KeyG if ctrl && modifiers.shift_key() => {
                self.apply_with(Tools::ungroup_selected);
            }
            KeyCode::KeyG if ctrl => self.apply_with(Tools::group_selected),
            KeyCode::KeyL if ctrl && modifiers.shift_key() => {
                self.apply_with(Tools::toggle_lock_selected);
            }
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
            KeyCode::Delete | KeyCode::Backspace => {
                if let Some(command) = self.tools.delete_selected(&self.document) {
                    self.apply(command);
                }
            }
            KeyCode::ArrowLeft => self.nudge(-1.0, 0.0),
            KeyCode::ArrowRight => self.nudge(1.0, 0.0),
            KeyCode::ArrowUp => self.nudge(0.0, -1.0),
            KeyCode::ArrowDown => self.nudge(0.0, 1.0),
            // Esc backs out one level: gesture, then tool, then selection.
            KeyCode::Escape => {
                if !self.tools.cancel() {
                    if self.tools.active() != ToolKind::Select {
                        self.tools.set_active(ToolKind::Select);
                    } else {
                        self.tools.clear_selection();
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
        let wake = [
            self.session.deadline(),
            ui_repaint.filter(|&at| at > now),
            blink,
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
        let canvas_typing = self.editing.is_some()
            && matches!(
                event,
                WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_)
            );
        if feed_ui
            && !canvas_typing
            && let (Some(ui), Some(window)) = (&mut self.ui, &self.window)
        {
            let response = ui.on_window_event(window, &event);
            if response.repaint {
                window.request_redraw();
            }
            ui_consumed = response.consumed;
        }
        let over_ui = self.ui.as_ref().is_some_and(Ui::wants_pointer) && !self.canvas_drag;
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
                } else if self.cropping.is_some() {
                    self.crop_key(&event)
                } else if let Some(edit) = &mut self.editing {
                    let modifiers = self.navigation.modifiers();
                    match edit.key(self.text.engine(), &event, modifiers) {
                        KeyOutcome::Handled => {}
                        KeyOutcome::Finish => self.finish_editing(),
                        KeyOutcome::FinishAndPass => {
                            self.finish_editing();
                            self.handle_key(&event);
                        }
                        KeyOutcome::NextSticky { below } => self.next_sticky(below),
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
                // drag started on it ends even over a toolbar.
                if state == ElementState::Pressed && (ui_consumed || over_ui) {
                    false
                } else {
                    let consumed = self.navigation.mouse_button(button, state);
                    let left = button == MouseButton::Left;
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
                    text || (!consumed && left && self.handle_left_button(state))
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if ui_consumed || over_ui {
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
        // Over the UI, egui sets the cursor.
        if !over_ui {
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
        lines,
        gaps,
        end_handles: end_handles.map(Vec::from).unwrap_or_default(),
        connection_points,
        connection_active,
    }
}
