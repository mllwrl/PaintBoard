//! Toolbars and the properties panel, drawn with egui on top of the canvas.
//!
//! The UI only reads a [`Model`] of the app and reports what the user did as [`Action`]s; the
//! app carries them out, the same way as keyboard shortcuts.

use std::sync::Arc;

use egui::{
    Align2, Area, Button, Color32, Context, FontData, FontDefinitions, FontFamily, Frame, Id,
    Order, Painter, Pos2, Rect, Response, RichText, Sense, Shadow, Shape, Slider, Stroke, Vec2,
};
use pb_core::{
    Cap, Connector, ConnectorChange, HAlign, ObjectId, Rgba, Route, ShapeKind, Style, StyleChange,
    TextMarks, VAlign, ZOrder,
};
use pb_geom::kurbo::{self, PathEl};
use pb_platform::Instant;
use pb_render::{FrameTarget, Gpu, GridStyle};
use pb_text::Mark;
use pb_tools::{InkStyle, ToolKind};
use winit::window::Window;

/// Something the user asked for through the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Tool(ToolKind),
    InkStyle {
        marker: bool,
        style: InkStyle,
    },
    /// Color of new sticky notes.
    StickyColor(Rgba),
    /// Route of new connectors.
    ConnectorRoute(Route),
    /// Changes the selected connectors.
    SetConnector(ConnectorChange),
    Undo,
    Redo,
    NewBoard,
    Open,
    Save,
    SaveAs,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    NextGrid,
    ToggleSnap,
    SetStyle(StyleChange),
    /// A slider drag starts: the style changes until [`Action::EndStyleDrag`] are one undo step.
    BeginStyleDrag,
    EndStyleDrag,
    Group,
    Ungroup,
    ToggleLock,
    /// Hides the selection, or shows it again if all of it is hidden.
    ToggleHidden,
    /// Shows hidden objects faded, or leaves them out again.
    ShowHidden,
    /// Protects the board from changes, or lifts that.
    ToggleReadOnly,
    /// Opens or closes the layers panel.
    ToggleLayers,
    /// Adds a layer on top and makes it the one new objects go to.
    AddLayer,
    /// Makes a layer the one new objects go to.
    ActivateLayer(Arc<str>),
    /// Stores a layer as given: shown or hidden, locked or not, renamed.
    UpdateLayer(pb_core::Layer),
    /// Moves a layer one step up (true) or down.
    MoveLayer(Arc<str>, bool),
    /// Removes a layer with everything on it.
    RemoveLayer(Arc<str>),
    /// Moves the selection to a layer.
    MoveToLayer(Arc<str>),
    Duplicate,
    Delete,
    Reorder(ZOrder),
    /// Ask for image files and put them on the board.
    InsertImage,
    /// Start cropping the selected image.
    Crop,
    /// Show and select a frame.
    GoToFrame(ObjectId),
    /// Turns a mark on or off in the text being edited.
    ToggleMark(Mark),
    /// Colors the selected part of the text being edited; `None` for the object's color.
    MarkColor(Option<Rgba>),
    /// Shows or hides the minimap.
    ToggleMinimap,
    /// Puts the selected connector back on its automatic path.
    ResetBends,
    /// Adds a row (`rows`) or column to the selected table: after the selected cells
    /// (`after`) or before them, or at the end when no cells are selected.
    AddTableLine {
        rows: bool,
        after: bool,
    },
    /// Removes the rows (`true`) or columns of the selected cells, or the last one.
    RemoveTableLines(bool),
    /// Merges the selected table cells (`true`), or splits the merged ones among them.
    MergeCells(bool),
    Copy,
    Cut,
    /// Pastes where the right-click menu was opened, or under the pointer.
    Paste,
    SelectAll,
    /// Starts editing the selected object's text.
    EditText,
    /// Starts a sticky note, or a text, where the right-click menu was opened.
    AddStickyHere,
    AddTextHere,
    CloseContextMenu,
    /// Opens or closes the keyboard shortcuts window.
    ToggleShortcuts,
    /// Starts voting with this many votes per person.
    StartVoting(u32),
    /// Ends voting and shows the results.
    EndVoting,
    /// Removes the votes.
    ClearVotes,
    /// Opens or closes the voting results.
    ToggleResults,
    /// Starts a countdown of this many minutes.
    StartTimer(u64),
    /// Pauses the countdown, or runs it on.
    PauseTimer,
    /// Starts the countdown over.
    ResetTimer,
    CloseTimer,
    /// Enters focus mode on the selection or the view, or leaves it (Shift+F).
    ToggleFocus,
    /// Presents the board frame by frame (F5).
    Present,
    /// Moves through the presentation by this many frames (`i64::MIN`/`MAX`: first/last).
    PresentStep(i64),
    StopPresenting,
    /// Turns the laser pointer on or off while presenting.
    ToggleLaser,
    /// Opens or closes the template gallery.
    ToggleTemplates,
    /// Adds template `i` of `templates::TEMPLATES` in the middle of the view.
    InsertTemplate(usize),
    /// Opens the window for making a diagram from text.
    OpenDiagram,
    /// Makes a diagram from this Mermaid text.
    InsertDiagram(String),
    CloseDiagram,
    /// Adds this comment to the thread shown, starting it if new.
    PostComment(String),
    DeleteComment(std::sync::Arc<str>),
    /// Marks the thread shown resolved (`true`) or open.
    ResolveThread(bool),
    DeleteThread,
    CloseThread,
    /// Shows a thread from the Comments menu.
    OpenThread(std::sync::Arc<str>),
    /// Shows or hides resolved threads.
    ToggleShowResolved,
    /// Opens the search bar (Ctrl+F).
    OpenSearch,
    /// Searches the board for this text.
    Search(String),
    /// Shows the next search result, or the previous one.
    SearchStep(bool),
    CloseSearch,
    /// Links the selected object to a typed or pasted address or anchor.
    SetLink(String),
    /// Links the selected object to another object.
    LinkTo(ObjectId),
    /// Removes the selected object's link.
    RemoveLink,
    /// Links the selected object to the next object clicked on the board.
    PickLinkTarget,
    /// Copies an anchor to the selected object, which other objects can link to.
    CopyAnchor,
    /// Opens the selected object's link.
    FollowLink,
    /// Fetches the selected web page card's page again to fill it in.
    FetchPreview,
    /// Lays the selection out in a grid (`None`) or in clusters.
    Arrange(Option<pb_tools::ClusterBy>),
    /// Puts your reaction on the selected object, or takes it off.
    React(pb_core::Reaction),
    /// Puts a tag on the selection, or takes it off if everything selected has it.
    ToggleTag(std::sync::Arc<str>),
    /// Adds a tag with this name to the board and puts it on the selection.
    NewTag(String),
    /// Gives a tag the next color.
    RecolorTag(std::sync::Arc<str>),
    /// Removes a tag from the board.
    RemoveTag(std::sync::Arc<str>),
    /// Turns filtering by a tag on or off.
    FilterTag(std::sync::Arc<str>),
    /// Turns filtering by a fill color on or off.
    FilterColor(Rgba),
    /// Shows everything again.
    ClearFilter,
    /// Starts a new child (`true`) or sibling of the selected mind map node.
    AddNode(bool),
    /// Collapses or expands the selected mind map node.
    ToggleNode,
    /// Removes the selected mind map node and everything under it.
    DeleteNode,
    /// Adds a column at the right of the selected kanban board.
    AddKanbanColumn,
    /// Removes the right column of the selected kanban board.
    RemoveKanbanColumn,
    /// Removes the selected kanban card.
    DeleteCard,
    /// Centers the view on a world point (from the minimap).
    LookAt(kurbo::Point),
    /// Replace the selected handwriting with typed text.
    ToText,
    /// Solve the selected handwritten expression.
    Calculate,
}

/// What the UI shows.
pub struct Model<'a> {
    pub board: &'a str,
    /// The board's frames with their titles, back to front.
    pub frames: Vec<(ObjectId, String)>,
    pub tool: ToolKind,
    pub last_shape: ShapeKind,
    /// The structure tool (frame, table, …) the shared button offers.
    pub last_structure: ToolKind,
    /// Style of the pen or marker, whichever is active.
    pub ink: InkStyle,
    pub sticky_color: Rgba,
    pub connector_route: Route,
    pub can_undo: bool,
    pub can_redo: bool,
    pub zoom: f64,
    pub grid: GridStyle,
    pub snap: bool,
    /// Hidden while a gesture is in progress.
    pub selection: Option<SelectionInfo>,
    /// Styles of the text being edited, if it takes them.
    pub text_format: Option<TextFormat>,
    /// The board in small; `None` when hidden.
    pub minimap: Option<Minimap>,
    /// The board's tags with how many objects carry each, by name.
    pub tags: Vec<(pb_core::Tag, usize)>,
    /// Fill colors on the board, to filter by.
    pub fill_colors: Vec<Rgba>,
    pub filter: &'a pb_core::Filter,
    /// The search bar, when it is open.
    pub search: Option<SearchInfo>,
    /// The right-click menu, when it is open.
    pub context_menu: Option<ContextMenuInfo>,
    pub show_shortcuts: bool,
    /// How many objects are hidden, and whether they are shown faded.
    pub hidden: (usize, bool),
    /// The board is read-only: only looking and commenting.
    pub read_only: bool,
    /// The layers panel, while open.
    pub layers: Option<LayersView>,
    /// The diagram-from-text window, when it is open, with the last error.
    pub diagram: Option<Option<String>>,
    /// The template gallery is open.
    pub show_templates: bool,
    /// Focus mode is on: the board menu, minimap and search step aside.
    pub focus: bool,
    /// Presenting: the frame shown, how many there are, and its title.
    pub presentation: Option<(usize, usize, String)>,
    /// The laser pointer is on (while presenting).
    pub laser: bool,
    /// The countdown, while shown.
    pub timer: Option<TimerView>,
    /// The voting session, if there is one.
    pub voting: Option<VotingView>,
    /// The voting results window: objects with their votes, most first.
    pub results: Option<Vec<(ObjectId, u32, String)>>,
    /// The board has frames to present.
    pub can_present: bool,
    /// The comment thread shown.
    pub thread: Option<ThreadView>,
    /// The board's threads: id, a line about it, and whether it is resolved.
    pub threads: Vec<(std::sync::Arc<str>, String, bool)>,
    pub show_resolved: bool,
}

/// A comment thread in its panel.
#[derive(Clone, Debug)]
pub struct ThreadView {
    /// The spot it marks, in logical pixels.
    pub pos: (f32, f32),
    /// Not started yet: no comments.
    pub is_new: bool,
    pub resolved: bool,
    pub comments: Vec<CommentView>,
}

#[derive(Clone, Debug)]
pub struct CommentView {
    pub id: std::sync::Arc<str>,
    pub author: String,
    /// How long ago, for people.
    pub when: String,
    pub text: String,
    /// Written by the person using the app.
    pub mine: bool,
}

/// What the right-click menu offers.
#[derive(Clone, Copy, Debug)]
pub struct ContextMenuInfo {
    /// Where it opens, in logical pixels.
    pub pos: (f32, f32),
    /// It was opened on the selection, not on empty board.
    pub selection: bool,
    pub several: bool,
    pub has_group: bool,
    pub locked: bool,
    /// A single object whose text can be edited is selected.
    pub has_text: bool,
    /// A single object (not a group) is selected.
    pub single_object: bool,
    /// Focus mode is on.
    pub focus: bool,
    /// Everything selected is hidden.
    pub hidden: bool,
    /// The board is read-only.
    pub read_only: bool,
}

/// A voting session, for the person using the app.
#[derive(Clone, Copy, Debug)]
pub struct VotingView {
    /// People can still vote.
    pub open: bool,
    pub limit: u32,
    /// Votes the person has left.
    pub left: u32,
}

/// What the countdown shows.
#[derive(Clone, Debug)]
pub struct TimerView {
    /// Time left, like `4:32`.
    pub text: String,
    pub running: bool,
    /// Time is up.
    pub done: bool,
    /// Drawn highlighted now, to blink when time is up.
    pub flash: bool,
}

/// What the search bar shows.
#[derive(Clone, Copy, Debug)]
pub struct SearchInfo {
    pub count: usize,
    /// The result shown now.
    pub current: usize,
    /// Something is typed, so "no results" means something.
    pub has_query: bool,
    /// The field should take the keyboard.
    pub focus: bool,
}

/// What the minimap shows, in world coordinates.
pub struct Minimap {
    /// The part of the board it covers: everything on it and the view.
    pub area: kurbo::Rect,
    /// Object bounds with their colors, back to front.
    pub objects: Vec<(kurbo::Rect, Rgba)>,
    /// The part of the board in the window.
    pub view: kurbo::Rect,
}

/// Width of the minimap, and the limits of its height, in logical pixels.
const MINIMAP_WIDTH: f32 = 200.0;
const MINIMAP_HEIGHT: std::ops::RangeInclusive<f32> = 80.0..=160.0;

/// The text being edited, for the formatting bar.
pub struct TextFormat {
    /// The edit box on screen, in logical pixels.
    pub screen: kurbo::Rect,
    /// Marks of the selection, or of what is typed next.
    pub marks: TextMarks,
}

/// Text colors offered while editing; the default color comes first.
const TEXT_COLORS: [Option<Rgba>; 7] = [
    None,
    Some(Rgba::rgb(0xF2, 0x55, 0x55)),
    Some(Rgba::rgb(0xF5, 0x8A, 0x1F)),
    Some(Rgba::rgb(0x2E, 0x9E, 0x4F)),
    Some(Rgba::rgb(0x3B, 0x6E, 0xF5)),
    Some(Rgba::rgb(0x8E, 0x4D, 0xD6)),
    Some(Rgba::rgb(0x80, 0x80, 0x80)),
];

pub struct SelectionInfo {
    /// Selection bounds on screen, in logical pixels.
    pub screen: kurbo::Rect,
    /// Style of the first selected object.
    pub style: Style,
    pub has_group: bool,
    /// A shape is selected, so there is a fill to change.
    pub has_fill: bool,
    /// Something with an outline width (shape or ink) is selected.
    pub has_width: bool,
    /// The first selected connector, for its route and arrowheads.
    pub connector: Option<Connector>,
    /// Something other than images is selected, so there is an outline or text color.
    pub has_stroke: bool,
    /// What the outline color is called for this selection.
    pub stroke_label: &'static str,
    /// Only ink is selected and handwriting can be recognized: `Some(busy)`, where `busy`
    /// says a recognition is already running.
    pub handwriting: Option<bool>,
    /// Text style of the first selected shape or table, if there is one.
    pub shape_text: Option<ShapeText>,
    /// A single table is selected.
    pub table: Option<TableInfo>,
    /// A single kanban board is selected: its number of columns, and whether a card of it
    /// is selected.
    pub kanban: Option<(usize, bool)>,
    /// A node of a mind map is selected: whether it has children, and is collapsed.
    pub mind_node: Option<(bool, bool)>,
    /// Tags every selected object has, and tags only some have.
    pub tags_on_all: Vec<std::sync::Arc<str>>,
    pub tags_on_some: Vec<std::sync::Arc<str>>,
    /// The board's tags, to offer.
    pub board_tags: Vec<pb_core::Tag>,
    /// A single unlocked object (not a group) is selected, which can link somewhere.
    pub link: Option<LinkInfo>,
    /// A single object is selected: the reactions you put on it.
    pub my_reactions: Option<Vec<pb_core::Reaction>>,
    /// A single web page card is selected.
    pub is_embed: bool,
    /// A single unlocked image is selected.
    pub can_crop: bool,
    pub several: bool,
    pub locked: bool,
    /// Everything selected is hidden (they are being shown faded).
    pub hidden: bool,
    /// The board is read-only.
    pub read_only: bool,
    /// The board's layers, top first, when there are several, and the selection's layer
    /// if it is all on one.
    pub layers: Vec<(Arc<str>, String)>,
    pub layer: Option<Arc<str>>,
}

/// The layers panel: layers top first, each with how many objects are on it, and the
/// layer new objects go to.
pub struct LayersView {
    pub layers: Vec<(pb_core::Layer, usize)>,
    pub active: Arc<str>,
}

/// The link of the selected object.
pub struct LinkInfo {
    /// What it links to now, described.
    pub current: Option<String>,
    /// Frames it can link to, with their titles.
    pub frames: Vec<(ObjectId, String)>,
    /// A target is being picked on the board.
    pub picking: bool,
}

/// How a shape's text looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeText {
    pub color: Rgba,
    /// The size it is drawn at, fitted or set.
    pub size: f64,
    /// The size is fitted to the shape rather than set.
    pub auto: bool,
    pub align: HAlign,
    pub valign: VAlign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Popup {
    Fill,
    Stroke,
    Text,
    Opacity,
    Tags,
    Link,
    React,
    Arrange,
}

/// Font sizes offered for shape text, in logical pixels at 100% zoom.
const FONT_SIZES: std::ops::RangeInclusive<f64> = 6.0..=144.0;

/// Colors offered for fill and outline. The last entry is "none".
const PALETTE: [Rgba; 16] = [
    Rgba::rgb(0xFF, 0xFF, 0xFF),
    Rgba::rgb(0xE6, 0xE6, 0xE6),
    Rgba::rgb(0x80, 0x80, 0x80),
    Rgba::rgb(0x1F, 0x29, 0x37),
    Rgba::rgb(0xFF, 0xF1, 0x76),
    Rgba::rgb(0xFF, 0xB7, 0x4D),
    Rgba::rgb(0xF2, 0x55, 0x55),
    Rgba::rgb(0xF4, 0x8F, 0xB1),
    Rgba::rgb(0xB3, 0x9D, 0xDB),
    Rgba::rgb(0x5C, 0x6B, 0xF0),
    Rgba::rgb(0x81, 0xD4, 0xFA),
    Rgba::rgb(0x4D, 0xD0, 0xC4),
    Rgba::rgb(0x66, 0xBB, 0x6A),
    Rgba::rgb(0xC5, 0xE1, 0xA5),
    Rgba::rgb(0x8D, 0x6E, 0x63),
    Rgba(0),
];

/// Sticky note colors, light enough for dark text. The first is the default.
pub const STICKY_COLORS: [Rgba; 8] = [
    Rgba::rgb(0xFF, 0xF1, 0x76),
    Rgba::rgb(0xFF, 0xD0, 0x80),
    Rgba::rgb(0xFF, 0xAB, 0xAB),
    Rgba::rgb(0xF8, 0xBB, 0xD9),
    Rgba::rgb(0xD1, 0xC4, 0xE9),
    Rgba::rgb(0xB3, 0xE0, 0xFF),
    Rgba::rgb(0xB9, 0xF0, 0xC8),
    Rgba::rgb(0xF0, 0xF0, 0xF0),
];

const ACCENT: Color32 = Color32::from_rgb(0x3B, 0x6E, 0xF5);
const INK: Color32 = Color32::from_rgb(0x1F, 0x29, 0x37);
const BUTTON: f32 = 36.0;
const MARGIN: f32 = 12.0;
/// Gap between the selection and its toolbar; leaves room for the rotation knob.
const SELECTION_GAP: f32 = 44.0;

pub struct Ui {
    ctx: Context,
    state: egui_winit::State,
    renderer: Option<egui_wgpu::Renderer>,
    /// Texture changes not yet uploaded (kept when a frame could not be drawn).
    textures: egui::TexturesDelta,
    paint_jobs: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
    popup: Option<Popup>,
    /// Name typed for a new tag.
    new_tag: String,
    /// Address typed for a link.
    link_text: String,
    /// Text typed into the search bar.
    search_text: String,
    /// Mermaid text in the diagram window, kept between openings.
    diagram_text: String,
    /// A comment being written.
    comment_text: String,
    /// The layer being renamed, with the name typed so far.
    layer_name: Option<(Arc<str>, String)>,
    repaint_at: Option<Instant>,
}

impl Ui {
    pub fn new(window: &Window) -> Self {
        let ctx = Context::default();
        ctx.set_fonts(fonts());
        ctx.set_visuals(egui::Visuals::light());
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        Self {
            ctx,
            state,
            renderer: None,
            textures: egui::TexturesDelta::default(),
            paint_jobs: Vec::new(),
            pixels_per_point: 1.0,
            popup: None,
            new_tag: String::new(),
            link_text: String::new(),
            search_text: String::new(),
            diagram_text: DIAGRAM_EXAMPLE.to_owned(),
            comment_text: String::new(),
            layer_name: None,
            repaint_at: None,
        }
    }

    pub fn attach_gpu(&mut self, gpu: &Gpu) {
        let max_side = gpu.device().limits().max_texture_dimension_2d as usize;
        self.state.set_max_texture_side(max_side);
        self.renderer = Some(egui_wgpu::Renderer::new(
            gpu.device(),
            gpu.surface_format(),
            egui_wgpu::RendererOptions::default(),
        ));
    }

    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &winit::event::WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    /// The pointer is over the UI or dragging one of its widgets.
    pub fn wants_pointer(&self) -> bool {
        self.ctx.is_pointer_over_egui() || self.ctx.egui_is_using_pointer()
    }

    /// A text field has focus, so keys are not shortcuts.
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// When the UI wants to be drawn again (animations, tooltips).
    pub fn repaint_at(&self) -> Option<Instant> {
        self.repaint_at
    }

    /// Lays out the UI for this frame and returns what the user did.
    pub fn run(&mut self, window: &Window, model: &Model) -> Vec<Action> {
        let input = self.state.take_egui_input(window);
        let mut actions = Vec::new();
        let popup = &mut self.popup;
        let new_tag = &mut self.new_tag;
        let link_text = &mut self.link_text;
        let search_text = &mut self.search_text;
        let diagram_text = &mut self.diagram_text;
        let comment_text = &mut self.comment_text;
        let layer_name = &mut self.layer_name;
        let output = self.ctx.run_pass(input, |ctx| {
            if let Some(timer) = &model.timer {
                timer_pill(ctx, timer, &mut actions);
            }
            if let Some(voting) = model.voting.filter(|v| v.open) {
                voting_banner(ctx, voting, &mut actions);
            }
            if let Some(results) = &model.results {
                results_window(ctx, results, &mut actions);
            }
            if let Some((index, count, title)) = &model.presentation {
                // Nothing but the board and the controls.
                *popup = None;
                presentation_bar(ctx, *index, *count, title, model.laser, &mut actions);
                return;
            }
            if model.focus {
                focus_bar(ctx, &mut actions);
            } else {
                board_menu(ctx, model, &mut actions);
            }
            if model.read_only {
                read_only_toolbar(ctx, model, &mut actions);
            } else {
                toolbar(ctx, model, &mut actions);
            }
            let view = view_controls(ctx, model, &mut actions);
            if let Some(map) = model.minimap.as_ref().filter(|_| !model.focus) {
                minimap(ctx, map, view, &mut actions);
            }
            match &model.layers {
                Some(layers) => layers_panel(ctx, layers, layer_name, &mut actions),
                None => *layer_name = None,
            }
            if let Some(selection) = &model.selection {
                let fields = Fields { new_tag, link_text };
                selection_toolbar(ctx, selection, popup, fields, &mut actions);
            } else {
                *popup = None;
            }
            if let Some(format) = &model.text_format {
                format_bar(ctx, format, &mut actions);
            }
            match model.search {
                Some(info) => search_bar(ctx, info, search_text, &mut actions),
                None => search_text.clear(),
            }
            if let Some(info) = model.context_menu {
                context_menu(ctx, info, &mut actions);
            }
            if model.show_shortcuts {
                shortcuts_window(ctx, &mut actions);
            }
            if let Some(error) = &model.diagram {
                diagram_window(ctx, error.as_deref(), diagram_text, &mut actions);
            }
            if model.show_templates {
                templates_window(ctx, &mut actions);
            }
            match &model.thread {
                Some(thread) => thread_panel(ctx, thread, comment_text, &mut actions),
                None => comment_text.clear(),
            }
        });
        self.state
            .handle_platform_output(window, output.platform_output);
        self.textures.append(output.textures_delta);
        self.pixels_per_point = output.pixels_per_point;
        self.paint_jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.repaint_at = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .and_then(|v| Instant::now().checked_add(v.repaint_delay));
        actions
    }

    /// Draws the UI laid out by the last [`Ui::run`] into the frame.
    pub fn paint(&mut self, target: FrameTarget<'_>) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let mut textures = std::mem::take(&mut self.textures);
        for (id, deltas) in &textures.set {
            for delta in deltas {
                renderer.update_texture(target.device, target.queue, *id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: target.size,
            pixels_per_point: self.pixels_per_point,
        };
        // Callback command buffers would only come from custom paint callbacks; there are none.
        let _ = renderer.update_buffers(
            target.device,
            target.queue,
            target.encoder,
            &self.paint_jobs,
            &screen,
        );
        let pass = target
            .encoder
            .begin_render_pass(&egui_wgpu::wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(egui_wgpu::wgpu::RenderPassColorAttachment {
                    view: target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: egui_wgpu::wgpu::Operations {
                        load: egui_wgpu::wgpu::LoadOp::Load,
                        store: egui_wgpu::wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        renderer.render(&mut pass.forget_lifetime(), &self.paint_jobs, &screen);
        for id in &textures.free {
            renderer.free_texture(id);
        }
        // Applied above; egui checks that deltas are not dropped by accident.
        textures.clear();
    }
}

/// Inter (SIL Open Font License) instead of egui's built-in fonts, whose licenses we have
/// not cleared.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    fonts.font_data.insert(
        "inter".to_owned(),
        Arc::new(FontData::from_static(pb_text::FONT)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.insert(family, vec!["inter".to_owned()]);
    }
    fonts
}

/// White rounded panel with a soft shadow, like Miro's floating toolbars.
fn panel() -> Frame {
    Frame::new()
        .fill(Color32::WHITE)
        .corner_radius(10)
        .inner_margin(6)
        .stroke(Stroke::new(1.0, Color32::from_gray(225)))
        .shadow(Shadow {
            offset: [0, 2],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(30),
        })
}

fn board_menu(ctx: &Context, model: &Model, actions: &mut Vec<Action>) {
    Area::new(Id::unique("board"))
        .fixed_pos(Pos2::new(MARGIN, MARGIN))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.strong(model.board);
                    if model.read_only {
                        ui.weak("read-only");
                    }
                    ui.separator();
                    ui.menu_button("File", |ui| {
                        let items = [
                            ("New board", "Ctrl+N", Action::NewBoard),
                            ("Open…", "Ctrl+O", Action::Open),
                            ("Save", "Ctrl+S", Action::Save),
                            ("Save as…", "Ctrl+Shift+S", Action::SaveAs),
                            ("Insert image…", "", Action::InsertImage),
                            ("Diagram from text…", "", Action::OpenDiagram),
                        ];
                        for (label, shortcut, action) in items {
                            // Inserting changes the board.
                            let on = !model.read_only
                                || !matches!(action, Action::InsertImage | Action::OpenDiagram);
                            let button = Button::new(label).shortcut_text(shortcut);
                            if ui.add_enabled(on, button).clicked() {
                                actions.push(action);
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui
                            .add(Button::selectable(model.read_only, "Read-only"))
                            .on_hover_text("Protect the board: only looking and commenting")
                            .clicked()
                        {
                            actions.push(Action::ToggleReadOnly);
                            ui.close();
                        }
                    });
                    ui.menu_button("Frames", |ui| {
                        if model.frames.is_empty() {
                            ui.weak("No frames yet: draw one with F");
                        }
                        for (id, title) in &model.frames {
                            let title = if title.trim().is_empty() {
                                "Untitled frame"
                            } else {
                                title
                            };
                            if ui.button(title).clicked() {
                                actions.push(Action::GoToFrame(*id));
                                ui.close();
                            }
                        }
                    });
                    let filter = model.filter;
                    let label = if filter.is_empty() {
                        RichText::new("Filter")
                    } else {
                        RichText::new("Filter ●").color(Color32::from_rgb(0x3B, 0x82, 0xF6))
                    };
                    ui.menu_button(label, |ui| filter_menu(ui, model, actions));
                    if ui
                        .add_enabled(model.can_present, Button::new("Present"))
                        .on_hover_text("Show the frames one by one (F5)")
                        .on_disabled_hover_text("Add frames to present them")
                        .clicked()
                    {
                        actions.push(Action::Present);
                    }
                    ui.menu_button("Vote", |ui| voting_menu(ui, model.voting, &mut *actions));
                    ui.menu_button("Timer", |ui| {
                        for minutes in [1, 3, 5, 10, 15, 30] {
                            if ui.button(format!("{minutes} min")).clicked() {
                                actions.push(Action::StartTimer(minutes));
                                ui.close();
                            }
                        }
                    });
                    if ui
                        .add_enabled(!model.read_only, Button::new("Templates"))
                        .on_hover_text("Start from a ready-made layout")
                        .clicked()
                    {
                        actions.push(Action::ToggleTemplates);
                    }
                    let open = model.threads.iter().filter(|(_, _, r)| !r).count();
                    let label = if open > 0 {
                        format!("Comments ({open})")
                    } else {
                        "Comments".to_owned()
                    };
                    ui.menu_button(label, |ui| comments_menu(ui, model, actions));
                    if ui
                        .button("Find")
                        .on_hover_text("Find on the board (Ctrl+F)")
                        .clicked()
                    {
                        actions.push(Action::OpenSearch);
                    }
                    if ui
                        .button("?")
                        .on_hover_text("Keyboard shortcuts (F1)")
                        .clicked()
                    {
                        actions.push(Action::ToggleShortcuts);
                    }
                });
            });
        });
}

/// The toolbar of a read-only board: selecting and commenting.
fn read_only_toolbar(ctx: &Context, model: &Model, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    Area::new(Id::unique("tools"))
        .fixed_pos(Pos2::new(MARGIN, screen.center().y))
        .pivot(Align2::LEFT_CENTER)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                let tools = [
                    (
                        ToolKind::Select,
                        "Select (V)",
                        paint_cursor as fn(&Painter, Rect, Color32),
                    ),
                    (ToolKind::Comment, "Comment (C)", paint_comment),
                ];
                for (tool, tooltip, paint) in tools {
                    if icon_button(ui, model.tool == tool, tooltip, paint).clicked() {
                        actions.push(Action::Tool(tool));
                    }
                }
            });
        });
}

fn toolbar(ctx: &Context, model: &Model, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    let shapes_button = Area::new(Id::unique("tools"))
        .fixed_pos(Pos2::new(MARGIN, screen.center().y))
        .pivot(Align2::LEFT_CENTER)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel()
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                    let select = model.tool == ToolKind::Select;
                    if icon_button(ui, select, "Select (V)", paint_cursor).clicked() {
                        actions.push(Action::Tool(ToolKind::Select));
                    }
                    let shape_active = matches!(model.tool, ToolKind::Shape(_));
                    let shape = match model.tool {
                        ToolKind::Shape(shape) => shape,
                        _ => model.last_shape,
                    };
                    let shapes = icon_button(ui, shape_active, "Shapes (S, R, O)", |p, r, c| {
                        paint_shape(p, r, c, shape)
                    });
                    if shapes.clicked() && !shape_active {
                        actions.push(Action::Tool(ToolKind::Shape(shape)));
                    }
                    let ink_tools = [
                        (
                            ToolKind::Pen,
                            "Pen (P)",
                            paint_pen as fn(&Painter, Rect, Color32),
                        ),
                        (ToolKind::Marker, "Marker (M)", paint_marker),
                        (ToolKind::Eraser, "Eraser (E)", paint_eraser),
                        (ToolKind::Text, "Text (T)", paint_text),
                        (ToolKind::Sticky, "Sticky note (N)", paint_sticky),
                        (ToolKind::Connector, "Connector (L)", paint_connector),
                        (ToolKind::Comment, "Comment (C)", paint_comment),
                    ];
                    let mut ink_button = shapes.rect;
                    for (tool, tooltip, paint) in ink_tools {
                        let button = icon_button(ui, model.tool == tool, tooltip, paint);
                        if button.clicked() {
                            actions.push(Action::Tool(tool));
                        }
                        if model.tool == tool {
                            ink_button = button.rect;
                        }
                    }
                    // Frames, tables, kanban boards, mind maps and documents share a button.
                    let structure_active = STRUCTURES.iter().any(|(t, ..)| *t == model.tool);
                    let shown = if structure_active {
                        model.tool
                    } else {
                        model.last_structure
                    };
                    let (_, tip, paint) = STRUCTURES
                        .iter()
                        .find(|(t, ..)| *t == shown)
                        .copied()
                        .unwrap_or(STRUCTURES[0]);
                    let structures = icon_button(ui, structure_active, tip, paint);
                    if structures.clicked() && !structure_active {
                        actions.push(Action::Tool(shown));
                    }
                    let image = icon_button(ui, false, "Image (or drop, paste)", paint_image);
                    if image.clicked() {
                        actions.push(Action::InsertImage);
                    }
                    ui.separator();
                    let undo = ui.add_enabled_ui(model.can_undo, |ui| {
                        icon_button(ui, false, "Undo (Ctrl+Z)", |p, r, c| {
                            paint_turn(p, r, c, false)
                        })
                    });
                    if undo.inner.clicked() {
                        actions.push(Action::Undo);
                    }
                    let redo = ui.add_enabled_ui(model.can_redo, |ui| {
                        icon_button(ui, false, "Redo (Ctrl+Y)", |p, r, c| {
                            paint_turn(p, r, c, true)
                        })
                    });
                    if redo.inner.clicked() {
                        actions.push(Action::Redo);
                    }
                    (shapes.rect, ink_button, structures.rect)
                })
                .inner
        })
        .inner;
    let (shapes_button, ink_button, structures_button) = shapes_button;
    if STRUCTURES.iter().any(|(t, ..)| *t == model.tool) {
        Area::new(Id::unique("structures"))
            .fixed_pos(Pos2::new(
                structures_button.right() + 18.0,
                structures_button.top() - 8.0,
            ))
            .constrain(true)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                panel().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                        for (tool, tip, paint) in STRUCTURES {
                            if icon_button(ui, model.tool == tool, tip, paint).clicked() {
                                actions.push(Action::Tool(tool));
                            }
                        }
                    });
                });
            });
    }

    if model.tool.draws_ink() {
        ink_panel(ctx, model, ink_button, actions);
    }
    if model.tool == ToolKind::Sticky {
        sticky_panel(ctx, model, ink_button, actions);
    }
    if model.tool == ToolKind::Connector {
        connector_panel(ctx, model, ink_button, actions);
    }

    // Like Miro, the shape tool shows its shapes next to the toolbar, in sets.
    if let ToolKind::Shape(active) = model.tool {
        Area::new(Id::unique("shapes"))
            .fixed_pos(Pos2::new(
                shapes_button.right() + 18.0,
                shapes_button.top() - 8.0,
            ))
            .constrain(true)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                panel().show(ui, |ui| {
                    for (set, kinds) in SHAPE_SETS {
                        ui.weak(*set);
                        egui::Grid::new(*set)
                            .spacing(Vec2::splat(2.0))
                            .show(ui, |ui| {
                                for (i, (kind, name)) in kinds.iter().enumerate() {
                                    let kind = *kind;
                                    let button =
                                        icon_button(ui, kind == active, name, |p, r, c| {
                                            paint_shape(p, r, c, kind)
                                        });
                                    if button.clicked() {
                                        actions.push(Action::Tool(ToolKind::Shape(kind)));
                                    }
                                    if i % SHAPES_PER_ROW == SHAPES_PER_ROW - 1 {
                                        ui.end_row();
                                    }
                                }
                            });
                        ui.add_space(4.0);
                    }
                });
            });
    }
}

/// Shapes offered by the shape tool, in sets, with what each is called there.
const SHAPE_SETS: &[(&str, &[(ShapeKind, &str)])] = &[
    (
        "Basic",
        &[
            (ShapeKind::Rectangle, "Rectangle"),
            (ShapeKind::RoundedRectangle, "Rounded rectangle"),
            (ShapeKind::Ellipse, "Ellipse"),
            (ShapeKind::Diamond, "Diamond"),
            (ShapeKind::Triangle, "Triangle"),
            (ShapeKind::Star, "Star"),
            (ShapeKind::Arrow, "Arrow"),
            (ShapeKind::DoubleArrow, "Double arrow"),
            (ShapeKind::Cloud, "Cloud"),
            (ShapeKind::Hexagon, "Hexagon"),
            (ShapeKind::Parallelogram, "Parallelogram"),
            (ShapeKind::Trapezoid, "Trapezoid"),
        ],
    ),
    (
        "Flowchart",
        &[
            (ShapeKind::Terminator, "Start / end"),
            (ShapeKind::Rectangle, "Process"),
            (ShapeKind::Diamond, "Decision"),
            (ShapeKind::Parallelogram, "Data (input / output)"),
            (ShapeKind::Document, "Document"),
            (ShapeKind::PredefinedProcess, "Predefined process"),
            (ShapeKind::ManualInput, "Manual input"),
            (ShapeKind::Trapezoid, "Manual operation"),
            (ShapeKind::Hexagon, "Preparation"),
            (ShapeKind::Cylinder, "Database"),
            (ShapeKind::Delay, "Delay"),
            (ShapeKind::OffPage, "Off-page connector"),
        ],
    ),
    (
        "UML",
        &[
            (ShapeKind::UmlClass, "Class"),
            (ShapeKind::Actor, "Actor"),
            (ShapeKind::Ellipse, "Use case"),
            (ShapeKind::RoundedRectangle, "State"),
            (ShapeKind::Note, "Note"),
            (ShapeKind::Package, "Package"),
        ],
    ),
    (
        "Entity relationship",
        &[
            (ShapeKind::Rectangle, "Entity"),
            (ShapeKind::Diamond, "Relationship"),
            (ShapeKind::Ellipse, "Attribute"),
            (ShapeKind::UmlClass, "Table"),
        ],
    ),
    (
        "BPMN",
        &[
            (ShapeKind::RoundedRectangle, "Task"),
            (ShapeKind::Ellipse, "Event"),
            (ShapeKind::Diamond, "Gateway"),
            (ShapeKind::Note, "Data object"),
            (ShapeKind::Cylinder, "Data store"),
            (ShapeKind::Pool, "Pool"),
        ],
    ),
    (
        "Network",
        &[
            (ShapeKind::Server, "Server"),
            (ShapeKind::Monitor, "Computer"),
            (ShapeKind::Cylinder, "Database"),
            (ShapeKind::Cloud, "Cloud"),
            (ShapeKind::Actor, "User"),
        ],
    ),
];
const SHAPES_PER_ROW: usize = 6;

/// Draws an icon in a rectangle with a color.
type PaintIcon = fn(&Painter, Rect, Color32);

/// Tools that share the structures button, with their tooltips and icons.
const STRUCTURES: [(ToolKind, &str, PaintIcon); 5] = [
    (ToolKind::Frame, "Frame (F)", paint_frame),
    (ToolKind::Table, "Table", paint_table),
    (ToolKind::Kanban, "Kanban (K)", paint_kanban),
    (ToolKind::MindMap, "Mind map", paint_mindmap),
    (ToolKind::Doc, "Markdown document (D)", paint_doc),
];

/// Width ranges of the pen and the marker, in logical pixels.
const PEN_WIDTHS: std::ops::RangeInclusive<f64> = 1.0..=24.0;
const MARKER_WIDTHS: std::ops::RangeInclusive<f64> = 6.0..=60.0;

/// Color and width of the active pen or marker, next to its button.
fn ink_panel(ctx: &Context, model: &Model, button: Rect, actions: &mut Vec<Action>) {
    let marker = model.tool == ToolKind::Marker;
    let style = model.ink;
    Area::new(Id::unique("ink"))
        .fixed_pos(Pos2::new(button.right() + 18.0, button.top() - 8.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                if let Some(color) = palette(ui, style.color, false) {
                    actions.push(Action::InkStyle {
                        marker,
                        style: InkStyle { color, ..style },
                    });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let range = if marker { MARKER_WIDTHS } else { PEN_WIDTHS };
                    // A dot as thick as the line, up to the size of the preview box.
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::hover());
                    let radius = (style.width as f32 / 2.0).clamp(1.0, BUTTON / 2.0 - 2.0);
                    let preview = to_color32(style.color);
                    ui.painter().circle_filled(rect.center(), radius, preview);
                    let mut width = style.width;
                    let slider = Slider::new(&mut width, range).suffix(" px").step_by(0.5);
                    if ui.add(slider).changed() {
                        actions.push(Action::InkStyle {
                            marker,
                            style: InkStyle { width, ..style },
                        });
                    }
                });
            });
        });
}

/// Colors for new sticky notes, next to the tool's button.
fn sticky_panel(ctx: &Context, model: &Model, button: Rect, actions: &mut Vec<Action>) {
    Area::new(Id::unique("sticky"))
        .fixed_pos(Pos2::new(button.right() + 18.0, button.top() - 8.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                if let Some(color) = color_grid(ui, &STICKY_COLORS, model.sticky_color, 4) {
                    actions.push(Action::StickyColor(color));
                }
            });
        });
}

/// Route of new connectors, next to the tool's button.
fn connector_panel(ctx: &Context, model: &Model, button: Rect, actions: &mut Vec<Action>) {
    Area::new(Id::unique("connector"))
        .fixed_pos(Pos2::new(button.right() + 18.0, button.top() - 8.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    for route in Route::ALL {
                        let on = model.connector_route == route;
                        if ui.selectable_label(on, route.name()).clicked() {
                            actions.push(Action::ConnectorRoute(route));
                        }
                    }
                });
            });
        });
}

/// Route and arrowhead buttons for the selected connectors.
fn connector_controls(ui: &mut egui::Ui, c: &Connector, actions: &mut Vec<Action>) {
    for route in Route::ALL {
        if ui
            .selectable_label(c.route == route, route.name())
            .clicked()
        {
            actions.push(Action::SetConnector(ConnectorChange {
                route: Some(route),
                ..ConnectorChange::default()
            }));
        }
    }
    ui.separator();
    let toggled = |cap: Cap| {
        if cap == Cap::Arrow {
            Cap::None
        } else {
            Cap::Arrow
        }
    };
    let start = ui
        .selectable_label(c.start_cap == Cap::Arrow, "Start arrow")
        .on_hover_text("Arrowhead at the start");
    if start.clicked() {
        actions.push(Action::SetConnector(ConnectorChange {
            start_cap: Some(toggled(c.start_cap)),
            ..ConnectorChange::default()
        }));
    }
    let end = ui
        .selectable_label(c.end_cap == Cap::Arrow, "End arrow")
        .on_hover_text("Arrowhead at the end");
    if end.clicked() {
        actions.push(Action::SetConnector(ConnectorChange {
            end_cap: Some(toggled(c.end_cap)),
            ..ConnectorChange::default()
        }));
    }
    if !c.bends.is_empty()
        && ui
            .button("Reset path")
            .on_hover_text("Drop the bends set by hand")
            .clicked()
    {
        actions.push(Action::ResetBends);
    }
    ui.separator();
}

/// Grid, snapping, minimap and zoom, in the bottom-right corner. Returns where they are.
fn view_controls(ctx: &Context, model: &Model, actions: &mut Vec<Action>) -> Rect {
    let screen = ctx.content_rect();
    Area::new(Id::unique("view"))
        .fixed_pos(screen.max - Vec2::splat(MARGIN))
        .pivot(Align2::RIGHT_BOTTOM)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .button(model.grid.name())
                        .on_hover_text("Grid style (G)")
                        .clicked()
                    {
                        actions.push(Action::NextGrid);
                    }
                    if ui
                        .selectable_label(model.snap, "Snap to grid")
                        .on_hover_text("Shift+G")
                        .clicked()
                    {
                        actions.push(Action::ToggleSnap);
                    }
                    if ui
                        .selectable_label(model.minimap.is_some(), "Map")
                        .on_hover_text("Minimap")
                        .clicked()
                    {
                        actions.push(Action::ToggleMinimap);
                    }
                    if ui
                        .selectable_label(model.layers.is_some(), "Layers")
                        .on_hover_text("Layers of the board")
                        .clicked()
                    {
                        actions.push(Action::ToggleLayers);
                    }
                    let (hidden, shown) = model.hidden;
                    if (hidden > 0 || shown)
                        && ui
                            .selectable_label(shown, format!("Hidden ({hidden})"))
                            .on_hover_text("Show hidden objects faded, to pick and show them")
                            .clicked()
                    {
                        actions.push(Action::ShowHidden);
                    }
                    ui.separator();
                    if ui.button("−").on_hover_text("Zoom out (Ctrl+−)").clicked() {
                        actions.push(Action::ZoomOut);
                    }
                    let zoom = format!("{:.0}%", model.zoom * 100.0);
                    if ui
                        .button(zoom)
                        .on_hover_text("Reset zoom (Ctrl+0)")
                        .clicked()
                    {
                        actions.push(Action::ZoomReset);
                    }
                    if ui.button("+").on_hover_text("Zoom in (Ctrl+=)").clicked() {
                        actions.push(Action::ZoomIn);
                    }
                });
            });
        })
        .response
        .rect
}

/// The board in small above the view controls (at `below`); a press or drag on it moves
/// the view there.
fn minimap(ctx: &Context, map: &Minimap, below: Rect, actions: &mut Vec<Action>) {
    let area = map.area;
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return;
    }
    let height = (MINIMAP_WIDTH * (area.height() / area.width()) as f32)
        .clamp(*MINIMAP_HEIGHT.start(), *MINIMAP_HEIGHT.end());
    Area::new(Id::unique("minimap"))
        .fixed_pos(Pos2::new(below.right(), below.top() - 8.0))
        .pivot(Align2::RIGHT_BOTTOM)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                let size = Vec2::new(MINIMAP_WIDTH, height);
                let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
                // World to minimap: uniform scale, centered.
                let scale =
                    (f64::from(size.x) / area.width()).min(f64::from(size.y) / area.height());
                let offset = Vec2::new(
                    (size.x - (area.width() * scale) as f32) / 2.0,
                    (size.y - (area.height() * scale) as f32) / 2.0,
                );
                let to_map = |p: kurbo::Point| {
                    rect.min
                        + offset
                        + Vec2::new(
                            ((p.x - area.x0) * scale) as f32,
                            ((p.y - area.y0) * scale) as f32,
                        )
                };
                let map_rect = |r: kurbo::Rect| {
                    let r = Rect::from_two_pos(
                        to_map(r.origin()),
                        to_map(kurbo::Point::new(r.x1, r.y1)),
                    );
                    // Tiny objects stay visible.
                    Rect::from_center_size(r.center(), r.size().max(Vec2::splat(1.5)))
                };
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 4, Color32::from_gray(246));
                for (r, color) in &map.objects {
                    let r = map_rect(*r);
                    painter.rect_filled(r, 0, to_color32(*color));
                    // An edge, so light objects show on the light background.
                    painter.rect_stroke(
                        r,
                        0,
                        Stroke::new(0.75, Color32::from_gray(160)),
                        egui::StrokeKind::Inside,
                    );
                }
                painter.rect_stroke(
                    map_rect(map.view),
                    2,
                    Stroke::new(1.5, ACCENT),
                    egui::StrokeKind::Middle,
                );
                if (response.is_pointer_button_down_on() || response.clicked())
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let local = p - rect.min - offset;
                    let world = kurbo::Point::new(
                        area.x0 + f64::from(local.x) / scale,
                        area.y0 + f64::from(local.y) / scale,
                    );
                    actions.push(Action::LookAt(world));
                }
            });
        });
}

/// Properties and actions for the selection, floating above it.
/// Text typed into the selection popups, kept between frames.
struct Fields<'a> {
    new_tag: &'a mut String,
    link_text: &'a mut String,
}

fn selection_toolbar(
    ctx: &Context,
    selection: &SelectionInfo,
    popup: &mut Option<Popup>,
    fields: Fields<'_>,
    actions: &mut Vec<Action>,
) {
    let Fields { new_tag, link_text } = fields;
    let r = selection.screen;
    let center = r.center().x as f32;
    // Above the selection, or below it when there is no room at the top.
    let (anchor, pivot) = if (r.y0 as f32) - SELECTION_GAP > 90.0 {
        (
            Pos2::new(center, r.y0 as f32 - SELECTION_GAP),
            Align2::CENTER_BOTTOM,
        )
    } else {
        (
            Pos2::new(center, r.y1 as f32 + SELECTION_GAP / 2.0),
            Align2::CENTER_TOP,
        )
    };
    let bar = Area::new(Id::unique("selection"))
        .fixed_pos(anchor)
        .pivot(pivot)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    if selection.read_only {
                        if ui.button("Copy").on_hover_text("Ctrl+C").clicked() {
                            actions.push(Action::Copy);
                        }
                        return;
                    }
                    if selection.locked {
                        if ui.button("Unlock").on_hover_text("Ctrl+Shift+L").clicked() {
                            actions.push(Action::ToggleLock);
                        }
                        return;
                    }
                    let style = selection.style;
                    if selection.has_fill {
                        let fill = swatch_button(ui, style.fill, false, "Fill color");
                        toggle(popup, Popup::Fill, fill.clicked());
                    }
                    if selection.has_stroke {
                        let label = selection.stroke_label;
                        let stroke = swatch_button(ui, style.stroke, true, label);
                        toggle(popup, Popup::Stroke, stroke.clicked());
                    }
                    if let Some(text) = selection.shape_text {
                        let label = RichText::new("Aa").color(to_color32(text.color)).strong();
                        let button = ui.button(label).on_hover_text("Text color, size and alignment");
                        toggle(popup, Popup::Text, button.clicked());
                    }
                    if let Some(busy) = selection.handwriting {
                        ui.add_enabled_ui(!busy, |ui| {
                            let label = if busy { "Recognizing…" } else { "To text" };
                            if ui
                                .button(label)
                                .on_hover_text("Turn the handwriting into typed text")
                                .clicked()
                            {
                                actions.push(Action::ToText);
                            }
                            if ui
                                .button("Calculate")
                                .on_hover_text("Solve a written expression such as 12 + 7 =")
                                .clicked()
                            {
                                actions.push(Action::Calculate);
                            }
                        });
                        ui.separator();
                    }
                    if let Some(c) = &selection.connector {
                        connector_controls(ui, c, actions);
                    }
                    if let Some(table) = selection.table {
                        let add = |rows, after| Action::AddTableLine { rows, after };
                        let buttons = match table.cells {
                            None => vec![
                                ("+ Row", "Add a row at the bottom", add(true, true), true),
                                ("+ Column", "Add a column at the right", add(false, true), true),
                                ("− Row", "Remove the bottom row", Action::RemoveTableLines(true), table.rows > 1),
                                ("− Column", "Remove the right column", Action::RemoveTableLines(false), table.cols > 1),
                            ],
                            Some(cells) => vec![
                                ("Row ↑", "Insert a row above", add(true, false), true),
                                ("Row ↓", "Insert a row below", add(true, true), true),
                                ("Column ←", "Insert a column to the left", add(false, false), true),
                                ("Column →", "Insert a column to the right", add(false, true), true),
                                (
                                    "Delete rows",
                                    "Delete the selected rows",
                                    Action::RemoveTableLines(true),
                                    cells.rows().count() < table.rows,
                                ),
                                (
                                    "Delete columns",
                                    "Delete the selected columns",
                                    Action::RemoveTableLines(false),
                                    cells.cols().count() < table.cols,
                                ),
                                if table.can_split {
                                    ("Unmerge", "Split the merged cells", Action::MergeCells(false), true)
                                } else {
                                    ("Merge", "Join the selected cells into one", Action::MergeCells(true), table.can_merge)
                                },
                            ],
                        };
                        for (label, tip, action, enabled) in buttons {
                            if ui
                                .add_enabled(enabled, Button::new(label))
                                .on_hover_text(tip)
                                .clicked()
                            {
                                actions.push(action);
                            }
                        }
                        ui.separator();
                    }
                    if let Some((children, collapsed)) = selection.mind_node {
                        if ui.button("+ Child").on_hover_text("Add a subtopic (Tab)").clicked() {
                            actions.push(Action::AddNode(true));
                        }
                        if ui.button("+ Sibling").on_hover_text("Add a topic after this one (Enter)").clicked() {
                            actions.push(Action::AddNode(false));
                        }
                        if children {
                            let label = if collapsed { "Expand" } else { "Collapse" };
                            if ui.button(label).on_hover_text("Show or hide the subtopics").clicked() {
                                actions.push(Action::ToggleNode);
                            }
                        }
                        if ui.button("Delete topic").on_hover_text("Delete it with its subtopics (Delete)").clicked() {
                            actions.push(Action::DeleteNode);
                        }
                        ui.separator();
                    }
                    if let Some((columns, card)) = selection.kanban {
                        let mut buttons = vec![
                            ("+ Column", "Add a column at the right", Action::AddKanbanColumn, true),
                            ("− Column", "Remove the right column", Action::RemoveKanbanColumn, columns > 1),
                        ];
                        if card {
                            buttons.push(("Delete card", "Delete the selected card (Delete)", Action::DeleteCard, true));
                        }
                        for (label, tip, action, enabled) in buttons {
                            if ui
                                .add_enabled(enabled, Button::new(label))
                                .on_hover_text(tip)
                                .clicked()
                            {
                                actions.push(action);
                            }
                        }
                        ui.separator();
                    }
                    if selection.is_embed
                        && ui
                            .button("Update preview")
                            .on_hover_text("Fetch the page's title and description again")
                            .clicked()
                    {
                        actions.push(Action::FetchPreview);
                    }
                    if selection.can_crop
                        && ui
                            .button("Crop")
                            .on_hover_text("Double click or Enter; Enter or a click outside to finish, Esc to cancel")
                            .clicked()
                    {
                        actions.push(Action::Crop);
                    }
                    if selection.link.is_some() {
                        let link = ui.button("Link").on_hover_text("Link to a page or another object");
                        toggle(popup, Popup::Link, link.clicked());
                    }
                    if selection.my_reactions.is_some() {
                        let react = ui.button("React").on_hover_text("Add a reaction");
                        toggle(popup, Popup::React, react.clicked());
                    }
                    let tags = ui.button("Tags").on_hover_text("Tags on the selection");
                    toggle(popup, Popup::Tags, tags.clicked());
                    let opacity = format!("{:.0}%", style.opacity * 100.0);
                    let opacity = ui.button(opacity).on_hover_text("Opacity");
                    toggle(popup, Popup::Opacity, opacity.clicked());
                    ui.separator();
                    if selection.several {
                        let arrange = ui.button("Arrange").on_hover_text("Lay out in a grid or in groups");
                        toggle(popup, Popup::Arrange, arrange.clicked());
                    }
                    if selection.several && ui.button("Group").on_hover_text("Ctrl+G").clicked() {
                        actions.push(Action::Group);
                    }
                    if selection.has_group
                        && ui.button("Ungroup").on_hover_text("Ctrl+Shift+G").clicked()
                    {
                        actions.push(Action::Ungroup);
                    }
                    if ui.button("Lock").on_hover_text("Ctrl+Shift+L").clicked() {
                        actions.push(Action::ToggleLock);
                    }
                    if !selection.layers.is_empty() {
                        ui.menu_button("Layer", |ui| {
                            for (id, name) in &selection.layers {
                                let here = selection.layer.as_ref() == Some(id);
                                if ui.add(Button::selectable(here, name)).clicked() {
                                    actions.push(Action::MoveToLayer(id.clone()));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Move to another layer");
                    }
                    let hide = if selection.hidden { "Show" } else { "Hide" };
                    if ui.button(hide).on_hover_text("Ctrl+Shift+H").clicked() {
                        actions.push(Action::ToggleHidden);
                    }
                    ui.separator();
                    if ui.button("Front").on_hover_text("Ctrl+Shift+]").clicked() {
                        actions.push(Action::Reorder(ZOrder::Front));
                    }
                    if ui.button("Back").on_hover_text("Ctrl+Shift+[").clicked() {
                        actions.push(Action::Reorder(ZOrder::Back));
                    }
                    ui.separator();
                    if ui.button("Duplicate").on_hover_text("Ctrl+D").clicked() {
                        actions.push(Action::Duplicate);
                    }
                    if ui.button("Delete").on_hover_text("Delete").clicked() {
                        actions.push(Action::Delete);
                    }
                });
            });
        })
        .response;

    if selection.locked {
        *popup = None;
    }
    // A press anywhere else (the canvas, other toolbars) closes the popup.
    let pressed_outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .pointer_interact_pos()
            .is_some_and(|p| !bar.rect.contains(p));
    let Some(open) = *popup else {
        return;
    };
    let popup_area = Area::new(Id::unique("selection popup"))
        .fixed_pos(Pos2::new(bar.rect.left(), bar.rect.bottom() + 6.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| match open {
                Popup::Fill => {
                    if let Some(color) = palette(ui, selection.style.fill, true) {
                        actions.push(Action::SetStyle(StyleChange {
                            fill: Some(color),
                            ..StyleChange::default()
                        }));
                    }
                }
                Popup::Stroke => {
                    if let Some(color) = palette(ui, selection.style.stroke, true) {
                        actions.push(Action::SetStyle(StyleChange {
                            stroke: Some(color),
                            ..StyleChange::default()
                        }));
                    }
                    if !selection.has_width {
                        return;
                    }
                    ui.add_space(4.0);
                    let width = selection.style.stroke_width;
                    style_slider(ui, width, 0.0..=20.0, " px", actions, |v| StyleChange {
                        stroke_width: Some(v),
                        ..StyleChange::default()
                    });
                }
                Popup::Text => {
                    let Some(text) = selection.shape_text else {
                        return;
                    };
                    if let Some(color) = palette(ui, text.color, false) {
                        actions.push(Action::SetStyle(StyleChange {
                            text_color: Some(color),
                            ..StyleChange::default()
                        }));
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let auto = ui
                            .selectable_label(text.auto, "Auto")
                            .on_hover_text("As large as fits the shape");
                        if auto.clicked() && !text.auto {
                            actions.push(Action::SetStyle(StyleChange {
                                font_size: Some(None),
                                ..StyleChange::default()
                            }));
                        }
                        let mut size = text.size;
                        let slider = Slider::new(&mut size, FONT_SIZES)
                            .logarithmic(true)
                            .fixed_decimals(0);
                        let response = ui
                            .add(slider)
                            .on_hover_text("Font size; text that does not fit the shape shrinks");
                        style_drag(&response, actions, || StyleChange {
                            font_size: Some(Some(size.round())),
                            ..StyleChange::default()
                        });
                    });
                    ui.horizontal(|ui| {
                        for (align, name) in [
                            (HAlign::Left, "Left"),
                            (HAlign::Center, "Center"),
                            (HAlign::Right, "Right"),
                        ] {
                            if ui.selectable_label(text.align == align, name).clicked() {
                                actions.push(Action::SetStyle(StyleChange {
                                    text_align: Some(align),
                                    ..StyleChange::default()
                                }));
                            }
                        }
                        ui.separator();
                        for (valign, name) in [
                            (VAlign::Top, "Top"),
                            (VAlign::Middle, "Middle"),
                            (VAlign::Bottom, "Bottom"),
                        ] {
                            if ui.selectable_label(text.valign == valign, name).clicked() {
                                actions.push(Action::SetStyle(StyleChange {
                                    text_valign: Some(valign),
                                    ..StyleChange::default()
                                }));
                            }
                        }
                    });
                }
                Popup::Link => {
                    let Some(link) = &selection.link else {
                        return;
                    };
                    if let Some(current) = &link.current {
                        ui.horizontal(|ui| {
                            ui.label(format!("→ {current}"));
                            if ui.button("Open").clicked() {
                                actions.push(Action::FollowLink);
                            }
                            if ui.button("Remove").clicked() {
                                actions.push(Action::RemoveLink);
                            }
                        });
                        ui.separator();
                    }
                    ui.horizontal(|ui| {
                        let field = ui.add(
                            egui::TextEdit::singleline(link_text)
                                .hint_text("Web address or copied object link")
                                .desired_width(220.0),
                        );
                        let valid = pb_core::Link::from_input(link_text).is_some();
                        let entered =
                            field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let set = ui.add_enabled(valid, Button::new("Set"));
                        if valid && (entered || set.clicked()) {
                            actions.push(Action::SetLink(std::mem::take(link_text)));
                        }
                    });
                    if !link.frames.is_empty() {
                        ui.weak("Link to a frame:");
                        for (id, title) in &link.frames {
                            let title = if title.trim().is_empty() {
                                "Untitled frame"
                            } else {
                                title
                            };
                            if ui.button(title).clicked() {
                                actions.push(Action::LinkTo(*id));
                            }
                        }
                    }
                    ui.horizontal(|ui| {
                        let label = if link.picking {
                            "Click an object…"
                        } else {
                            "Pick on board"
                        };
                        if ui
                            .add_enabled(!link.picking, Button::new(label))
                            .on_hover_text("Then click the object to link to; Esc cancels")
                            .clicked()
                        {
                            actions.push(Action::PickLinkTarget);
                            *popup = None;
                        }
                        if ui
                            .button("Copy link")
                            .on_hover_text(
                                "Copy a link to this object, to paste into another one's Link",
                            )
                            .clicked()
                        {
                            actions.push(Action::CopyAnchor);
                        }
                    });
                }
                Popup::Arrange => {
                    let choices = [
                        ("Grid", "In a grid, in reading order (Alt+G)", None),
                        (
                            "By color",
                            "In groups by fill color, with a heading each",
                            Some(pb_tools::ClusterBy::Color),
                        ),
                        (
                            "By tag",
                            "In groups by tag, with a heading each",
                            Some(pb_tools::ClusterBy::Tag),
                        ),
                    ];
                    ui.horizontal(|ui| {
                        for (label, tip, by) in choices {
                            if ui.button(label).on_hover_text(tip).clicked() {
                                actions.push(Action::Arrange(by));
                            }
                        }
                    });
                }
                Popup::React => {
                    let Some(mine) = &selection.my_reactions else {
                        return;
                    };
                    ui.horizontal(|ui| {
                        for reaction in pb_core::Reaction::ALL {
                            let on = mine.contains(&reaction);
                            let button = icon_button(ui, on, reaction.name(), |p, r, _| {
                                paint_reaction(p, r, reaction)
                            });
                            if button.clicked() {
                                actions.push(Action::React(reaction));
                            }
                        }
                    });
                }
                Popup::Tags => {
                    for tag in &selection.board_tags {
                        let all = selection.tags_on_all.contains(&tag.id);
                        let some = selection.tags_on_some.contains(&tag.id);
                        let mark = if all {
                            "✔"
                        } else if some {
                            "–"
                        } else {
                            "  "
                        };
                        ui.horizontal(|ui| {
                            tag_dot(ui, tag.color);
                            let label = format!("{mark}  {}", tag.name);
                            if ui.selectable_label(all, label).clicked() {
                                actions.push(Action::ToggleTag(tag.id.clone()));
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        let field = ui.add(
                            egui::TextEdit::singleline(new_tag)
                                .hint_text("New tag")
                                .desired_width(140.0),
                        );
                        let entered =
                            field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let add = ui.add_enabled(!new_tag.trim().is_empty(), Button::new("Add"));
                        if (entered || add.clicked()) && !new_tag.trim().is_empty() {
                            actions.push(Action::NewTag(new_tag.trim().to_owned()));
                            new_tag.clear();
                        }
                    });
                }
                Popup::Opacity => {
                    let percent = selection.style.opacity * 100.0;
                    style_slider(ui, percent, 0.0..=100.0, "%", actions, |v| StyleChange {
                        opacity: Some(v / 100.0),
                        ..StyleChange::default()
                    });
                }
            });
        })
        .response;
    if pressed_outside
        && ctx
            .pointer_interact_pos()
            .is_some_and(|p| !popup_area.rect.contains(p))
    {
        *popup = None;
    }
}

/// The right-click menu: what can be done with the selection, or on empty board.
fn context_menu(ctx: &Context, info: ContextMenuInfo, actions: &mut Vec<Action>) {
    let area = Area::new(Id::unique("context menu"))
        .fixed_pos(Pos2::new(info.pos.0, info.pos.1))
        .order(Order::Foreground)
        .constrain(true)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.set_min_width(180.0);
                let mut item =
                    |ui: &mut egui::Ui, label: &str, keys: &str, action: Action, on: bool| {
                        let button = Button::new(label).shortcut_text(keys).frame(false);
                        if ui.add_enabled(on, button).clicked() {
                            actions.push(action);
                            actions.push(Action::CloseContextMenu);
                        }
                    };
                if info.read_only {
                    if info.selection {
                        item(ui, "Copy", "Ctrl+C", Action::Copy, true);
                    } else {
                        item(ui, "Select all", "Ctrl+A", Action::SelectAll, true);
                    }
                    if info.single_object {
                        item(ui, "Copy link to object", "", Action::CopyAnchor, true);
                    }
                    return;
                }
                if !info.selection {
                    item(ui, "Paste here", "Ctrl+V", Action::Paste, true);
                    item(ui, "Add sticky note here", "", Action::AddStickyHere, true);
                    item(ui, "Add text here", "", Action::AddTextHere, true);
                    ui.separator();
                    item(ui, "Select all", "Ctrl+A", Action::SelectAll, true);
                    let focus = if info.focus {
                        "Exit focus"
                    } else {
                        "Focus on this view"
                    };
                    item(ui, focus, "Shift+F", Action::ToggleFocus, true);
                    return;
                }
                let free = !info.locked;
                if info.has_text {
                    item(ui, "Edit text", "Enter", Action::EditText, free);
                    ui.separator();
                }
                item(ui, "Cut", "Ctrl+X", Action::Cut, free);
                item(ui, "Copy", "Ctrl+C", Action::Copy, true);
                item(ui, "Paste", "Ctrl+V", Action::Paste, true);
                item(ui, "Duplicate", "Ctrl+D", Action::Duplicate, true);
                item(ui, "Delete", "Delete", Action::Delete, free);
                ui.separator();
                item(
                    ui,
                    "Bring to front",
                    "Ctrl+Shift+]",
                    Action::Reorder(ZOrder::Front),
                    free,
                );
                item(
                    ui,
                    "Send to back",
                    "Ctrl+Shift+[",
                    Action::Reorder(ZOrder::Back),
                    free,
                );
                ui.separator();
                if info.several {
                    item(ui, "Group", "Ctrl+G", Action::Group, free);
                }
                if info.has_group {
                    item(ui, "Ungroup", "Ctrl+Shift+G", Action::Ungroup, free);
                }
                let lock = if info.locked { "Unlock" } else { "Lock" };
                item(ui, lock, "Ctrl+Shift+L", Action::ToggleLock, true);
                let hide = if info.hidden { "Show" } else { "Hide" };
                item(ui, hide, "Ctrl+Shift+H", Action::ToggleHidden, true);
                if info.single_object {
                    item(ui, "Copy link to object", "", Action::CopyAnchor, true);
                }
                ui.separator();
                let focus = if info.focus {
                    "Exit focus"
                } else {
                    "Focus on selection"
                };
                item(ui, focus, "Shift+F", Action::ToggleFocus, true);
            });
        });
    let pressed_elsewhere = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .pointer_interact_pos()
            .is_some_and(|p| !area.response.rect.contains(p));
    if pressed_elsewhere {
        actions.push(Action::CloseContextMenu);
    }
}

/// The board's comment threads, and whether resolved ones show.
fn comments_menu(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    if model.threads.is_empty() {
        ui.weak("No comments yet: use the comment tool (C)");
    }
    for (id, line, resolved) in &model.threads {
        if *resolved && !model.show_resolved {
            continue;
        }
        let text = if *resolved {
            RichText::new(format!("✓ {line}")).weak()
        } else {
            RichText::new(line)
        };
        if ui.button(text).clicked() {
            actions.push(Action::OpenThread(id.clone()));
            ui.close();
        }
    }
    ui.separator();
    let mut show = model.show_resolved;
    if ui.checkbox(&mut show, "Show resolved").clicked() {
        actions.push(Action::ToggleShowResolved);
    }
}

/// A comment thread next to its pin: the comments, and a field for the next one.
fn thread_panel(ctx: &Context, thread: &ThreadView, text: &mut String, actions: &mut Vec<Action>) {
    let accent = Color32::from_rgb(0x25, 0x63, 0xEB);
    Area::new(Id::unique("thread"))
        .fixed_pos(Pos2::new(thread.pos.0 + 14.0, thread.pos.1 - 10.0))
        .constrain(true)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.set_width(300.0);
                ui.horizontal(|ui| {
                    ui.strong(if thread.is_new {
                        "New comment"
                    } else {
                        "Comments"
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
                            actions.push(Action::CloseThread);
                        }
                        if !thread.is_new {
                            if ui
                                .small_button("Delete")
                                .on_hover_text("Delete the thread")
                                .clicked()
                            {
                                actions.push(Action::DeleteThread);
                            }
                            let (label, tip) = if thread.resolved {
                                ("Reopen", "Mark it open again")
                            } else {
                                (
                                    "Resolve",
                                    "Mark it done; it hides unless resolved ones show",
                                )
                            };
                            if ui.small_button(label).on_hover_text(tip).clicked() {
                                actions.push(Action::ResolveThread(!thread.resolved));
                            }
                        }
                    });
                });
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for comment in &thread.comments {
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.strong(&comment.author);
                                ui.weak(&comment.when);
                                if comment.mine {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.small_button("Delete").clicked() {
                                                actions.push(Action::DeleteComment(
                                                    comment.id.clone(),
                                                ));
                                            }
                                        },
                                    );
                                }
                            });
                            ui.label(mention_job(ui, &comment.text, accent));
                        }
                    });
                ui.separator();
                let field = ui.add(
                    egui::TextEdit::multiline(text)
                        .hint_text("Write a comment… @name mentions someone")
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                if thread.is_new
                    && thread.comments.is_empty()
                    && !field.has_focus()
                    && text.is_empty()
                {
                    field.request_focus();
                }
                let send = field.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
                ui.horizontal(|ui| {
                    let label = if thread.is_new { "Comment" } else { "Reply" };
                    let button = ui
                        .add_enabled(!text.trim().is_empty(), Button::new(label))
                        .on_hover_text("Ctrl+Enter");
                    if (button.clicked() || send) && !text.trim().is_empty() {
                        actions.push(Action::PostComment(std::mem::take(text)));
                    }
                });
            });
        });
}

/// `text` with its `@mentions` in `accent`.
fn mention_job(ui: &egui::Ui, text: &str, accent: Color32) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let normal = egui::TextFormat::simple(font.clone(), ui.visuals().text_color());
    let mention = egui::TextFormat::simple(font, accent);
    let mut job = egui::text::LayoutJob::default();
    let mut at = 0;
    for range in pb_core::comment::mention_ranges(text) {
        job.append(&text[at..range.start], 0.0, normal.clone());
        job.append(&text[range.clone()], 0.0, mention.clone());
        at = range.end;
    }
    job.append(&text[at..], 0.0, normal);
    job.wrap.max_width = 290.0;
    job
}

/// How far below the top of the window the countdown sits, in logical pixels.
const TIMER_DROP: f32 = 52.0;

/// Starting, ending and clearing votes.
fn voting_menu(ui: &mut egui::Ui, voting: Option<VotingView>, actions: &mut Vec<Action>) {
    let start = |ui: &mut egui::Ui, actions: &mut Vec<Action>| {
        ui.weak("Votes per person:");
        ui.horizontal(|ui| {
            for limit in [3, 5, 10] {
                if ui.button(limit.to_string()).clicked() {
                    actions.push(Action::StartVoting(limit));
                    ui.close();
                }
            }
        });
    };
    match voting {
        None => start(ui, actions),
        Some(v) if v.open => {
            ui.label(format!("{} of {} votes left", v.left, v.limit));
            if ui.button("End voting").clicked() {
                actions.push(Action::EndVoting);
                ui.close();
            }
        }
        Some(_) => {
            if ui.button("Results").clicked() {
                actions.push(Action::ToggleResults);
                ui.close();
            }
            if ui.button("Clear votes").clicked() {
                actions.push(Action::ClearVotes);
                ui.close();
            }
            ui.separator();
            start(ui, actions);
        }
    }
}

/// The note at the bottom while voting.
fn voting_banner(ctx: &Context, voting: VotingView, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    // Above the view controls, which reach the middle of narrow windows.
    Area::new(Id::unique("voting"))
        .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - MARGIN - 52.0))
        .pivot(Align2::CENTER_BOTTOM)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Voting");
                    ui.label(format!(
                        "{} of {} votes left · click an object to vote, Shift+click to take one back",
                        voting.left, voting.limit
                    ));
                    if ui.button("End voting").clicked() {
                        actions.push(Action::EndVoting);
                    }
                });
            });
        });
}

/// The voting results: objects by votes, each going to its object.
fn results_window(ctx: &Context, results: &[(ObjectId, u32, String)], actions: &mut Vec<Action>) {
    let mut open = true;
    let shown = egui::Window::new("Voting results")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_TOP, egui::vec2(-MARGIN, 64.0))
        .show(ctx, |ui| {
            if results.is_empty() {
                ui.weak("No votes yet");
            }
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    egui::Grid::new("results")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            for (id, votes, text) in results {
                                ui.strong(votes.to_string());
                                if ui.link(text).clicked() {
                                    actions.push(Action::GoToFrame(*id));
                                }
                                ui.end_row();
                            }
                        });
                });
        });
    if let Some(shown) = shown {
        let layer = shown.response.layer_id;
        let on_top = ctx.memory(|m| m.areas().top_layer_id(Order::Foreground)) == Some(layer);
        if !on_top {
            ctx.move_to_top(layer);
            ctx.request_repaint();
        }
    }
    if !open {
        actions.push(Action::ToggleResults);
    }
}

/// The countdown at the top middle, with its controls.
fn timer_pill(ctx: &Context, timer: &TimerView, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    let alert = Color32::from_rgb(0xDC, 0x26, 0x26);
    // Under the board menu, which can reach the middle of narrow windows.
    Area::new(Id::unique("timer"))
        .fixed_pos(Pos2::new(screen.center().x, MARGIN + TIMER_DROP))
        .pivot(Align2::CENTER_TOP)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            let frame = if timer.flash {
                panel().fill(alert)
            } else {
                panel()
            };
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (text, color) = if timer.done {
                        let color = if timer.flash { Color32::WHITE } else { alert };
                        ("Time's up".to_owned(), color)
                    } else {
                        (timer.text.clone(), ui.visuals().strong_text_color())
                    };
                    ui.label(RichText::new(text).size(20.0).color(color).strong());
                    if !timer.done {
                        let label = if timer.running { "Pause" } else { "Resume" };
                        let tip = if timer.running {
                            "Stop the clock"
                        } else {
                            "Run the clock on"
                        };
                        if ui.button(label).on_hover_text(tip).clicked() {
                            actions.push(Action::PauseTimer);
                        }
                    }
                    if ui.button("↺").on_hover_text("Start over").clicked() {
                        actions.push(Action::ResetTimer);
                    }
                    if ui.button("×").on_hover_text("Close").clicked() {
                        actions.push(Action::CloseTimer);
                    }
                });
            });
        });
}

/// The board's layers, top first, by the right edge: each can be shown or hidden, locked,
/// renamed (double click), moved up or down and removed; clicking one makes new objects
/// go there.
fn layers_panel(
    ctx: &Context,
    view: &LayersView,
    renaming: &mut Option<(Arc<str>, String)>,
    actions: &mut Vec<Action>,
) {
    let screen = ctx.content_rect();
    let last = view.layers.len().saturating_sub(1);
    Area::new(Id::unique("layers"))
        .fixed_pos(Pos2::new(screen.right() - MARGIN, 70.0))
        .pivot(Align2::RIGHT_TOP)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.set_width(250.0);
                ui.horizontal(|ui| {
                    ui.strong("Layers");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("+ Add")
                            .on_hover_text("New layer on top")
                            .clicked()
                        {
                            actions.push(Action::AddLayer);
                        }
                    });
                });
                ui.separator();
                egui::Grid::new("layers")
                    .num_columns(5)
                    .spacing([6.0, 4.0])
                    .show(ui, |ui| {
                        for (i, (layer, count)) in view.layers.iter().enumerate() {
                            let mut visible = layer.visible;
                            if ui
                                .checkbox(&mut visible, "")
                                .on_hover_text("Visible")
                                .changed()
                            {
                                actions.push(Action::UpdateLayer(pb_core::Layer {
                                    visible,
                                    ..layer.clone()
                                }));
                            }
                            match renaming.as_mut().filter(|(id, _)| *id == layer.id) {
                                Some((_, name)) => {
                                    let field = ui
                                        .add(egui::TextEdit::singleline(name).desired_width(110.0));
                                    field.request_focus();
                                    if field.lost_focus() {
                                        let name = name.trim();
                                        if !name.is_empty() {
                                            actions.push(Action::UpdateLayer(pb_core::Layer {
                                                name: name.into(),
                                                ..layer.clone()
                                            }));
                                        }
                                        *renaming = None;
                                    }
                                }
                                None => {
                                    let active = layer.id == view.active;
                                    let label = ui
                                        .add(
                                            Button::selectable(
                                                active,
                                                format!("{}  ({count})", layer.name),
                                            )
                                            .min_size(Vec2::new(110.0, 0.0)),
                                        )
                                        .on_hover_text(
                                            "New objects go to the selected layer; \
                                             double click to rename",
                                        );
                                    if label.double_clicked() {
                                        *renaming =
                                            Some((layer.id.clone(), layer.name.to_string()));
                                    } else if label.clicked() {
                                        actions.push(Action::ActivateLayer(layer.id.clone()));
                                    }
                                }
                            }
                            if ui
                                .selectable_label(layer.locked, "Lock")
                                .on_hover_text("Nothing on a locked layer can be changed")
                                .clicked()
                            {
                                actions.push(Action::UpdateLayer(pb_core::Layer {
                                    locked: !layer.locked,
                                    ..layer.clone()
                                }));
                            }
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                if ui.add_enabled(i > 0, Button::new("↑")).clicked() {
                                    actions.push(Action::MoveLayer(layer.id.clone(), true));
                                }
                                if ui.add_enabled(i < last, Button::new("↓")).clicked() {
                                    actions.push(Action::MoveLayer(layer.id.clone(), false));
                                }
                            });
                            let remove = ui
                                .add_enabled(!layer.is_base(), Button::new("×"))
                                .on_hover_text("Remove the layer and everything on it");
                            if remove.clicked() {
                                actions.push(Action::RemoveLayer(layer.id.clone()));
                            }
                            ui.end_row();
                        }
                    });
            });
        });
}

/// Stands in for the board menu in focus mode.
fn focus_bar(ctx: &Context, actions: &mut Vec<Action>) {
    Area::new(Id::unique("focus"))
        .fixed_pos(Pos2::new(MARGIN, MARGIN))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.strong("Focus mode");
                    ui.weak("the rest of the board is out of reach");
                    ui.separator();
                    if ui.button("Exit").on_hover_text("Esc, Shift+F").clicked() {
                        actions.push(Action::ToggleFocus);
                    }
                });
            });
        });
}

/// The controls at the bottom while presenting.
fn presentation_bar(
    ctx: &Context,
    index: usize,
    count: usize,
    title: &str,
    laser: bool,
    actions: &mut Vec<Action>,
) {
    let screen = ctx.content_rect();
    Area::new(Id::unique("presentation"))
        .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - MARGIN))
        .pivot(Align2::CENTER_BOTTOM)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(index > 0, Button::new("◀"))
                        .on_hover_text("Previous (←)")
                        .clicked()
                    {
                        actions.push(Action::PresentStep(-1));
                    }
                    let title = if title.trim().is_empty() {
                        "Untitled frame"
                    } else {
                        title
                    };
                    ui.label(format!("{title}  ·  {} / {count}", index + 1));
                    if ui
                        .add_enabled(index + 1 < count, Button::new("▶"))
                        .on_hover_text("Next (→, Space)")
                        .clicked()
                    {
                        actions.push(Action::PresentStep(1));
                    }
                    ui.separator();
                    if ui
                        .selectable_label(laser, "Laser")
                        .on_hover_text("Point with a red dot (L)")
                        .clicked()
                    {
                        actions.push(Action::ToggleLaser);
                    }
                    if ui.button("Exit").on_hover_text("Esc").clicked() {
                        actions.push(Action::StopPresenting);
                    }
                });
            });
        });
}

/// The template gallery: a card per template.
fn templates_window(ctx: &Context, actions: &mut Vec<Action>) {
    let mut open = true;
    let shown = egui::Window::new("Templates")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(ctx.content_rect().height() * 0.7)
                .show(ui, |ui| {
                    egui::Grid::new("templates")
                        .spacing([10.0, 10.0])
                        .show(ui, |ui| {
                            for (i, template) in crate::templates::TEMPLATES.iter().enumerate() {
                                let mut job = egui::text::LayoutJob::default();
                                let body = egui::TextStyle::Body.resolve(ui.style());
                                let small = egui::TextStyle::Small.resolve(ui.style());
                                let ink = ui.visuals().strong_text_color();
                                let weak = ui.visuals().weak_text_color();
                                job.append(template.name, 0.0, egui::TextFormat::simple(body, ink));
                                job.append(
                                    &format!(
                                        "
{}",
                                        template.description
                                    ),
                                    0.0,
                                    egui::TextFormat::simple(small, weak),
                                );
                                job.wrap.max_width = 190.0;
                                job.halign = egui::Align::LEFT;
                                let card = Button::new(job).wrap();
                                let response = ui.add_sized([210.0, 70.0], card);
                                if response.clicked() {
                                    actions.push(Action::InsertTemplate(i));
                                }
                                if i % 3 == 2 {
                                    ui.end_row();
                                }
                            }
                        });
                });
        });
    if let Some(shown) = shown {
        let layer = shown.response.layer_id;
        let on_top = ctx.memory(|m| m.areas().top_layer_id(Order::Foreground)) == Some(layer);
        if !on_top {
            ctx.move_to_top(layer);
            ctx.request_repaint();
        }
    }
    if !open {
        actions.push(Action::ToggleTemplates);
    }
}

/// What the diagram window starts with.
const DIAGRAM_EXAMPLE: &str = "flowchart TD
    A([Start]) --> B[Write the plan]
    B --> C{Looks good?}
    C -->|yes| D[Build it]
    C -->|no| B
    D --> E[(Save)]
";

/// A Mermaid flowchart to turn into shapes and connectors.
fn diagram_window(
    ctx: &Context,
    error: Option<&str>,
    text: &mut String,
    actions: &mut Vec<Action>,
) {
    let mut open = true;
    let shown = egui::Window::new("Diagram from text")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label("A Mermaid flowchart: nodes like A[Box], B{Decision}, C((Circle)); edges like A --> B or A -->|label| B.");
            ui.add(
                egui::TextEdit::multiline(text)
                    .code_editor()
                    .desired_rows(12)
                    .desired_width(460.0),
            );
            if let Some(error) = error {
                ui.colored_label(Color32::from_rgb(0xD0, 0x30, 0x30), error);
            }
            ui.horizontal(|ui| {
                if ui.button("Insert").clicked() {
                    actions.push(Action::InsertDiagram(text.clone()));
                }
                if ui.button("Cancel").clicked() {
                    actions.push(Action::CloseDiagram);
                }
            });
        });
    if let Some(shown) = shown {
        let layer = shown.response.layer_id;
        let on_top = ctx.memory(|m| m.areas().top_layer_id(Order::Foreground)) == Some(layer);
        if !on_top {
            ctx.move_to_top(layer);
            ctx.request_repaint();
        }
    }
    if !open {
        actions.push(Action::CloseDiagram);
    }
}

/// Keyboard shortcuts, by topic.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Tools",
        &[
            ("V", "Select"),
            ("R / O / S", "Rectangle / ellipse / next shape"),
            ("T", "Text"),
            ("N", "Sticky note"),
            ("L", "Connector"),
            ("F", "Frame"),
            ("K", "Kanban board"),
            ("D", "Markdown document"),
            ("C", "Comment"),
            ("Tab / Enter", "Mind map: add a subtopic / topic"),
            ("F2", "Edit the selected topic"),
            ("P / M / E", "Pen / marker / eraser"),
        ],
    ),
    (
        "Editing",
        &[
            ("Enter, double click", "Edit text"),
            ("Tab / Shift+Tab", "Next sticky note, cell or card"),
            ("Ctrl+B / I / U", "Bold / italic / underline"),
            ("Ctrl+Shift+X", "Strikethrough"),
            ("Esc", "Stop editing"),
        ],
    ),
    (
        "Selection",
        &[
            ("Ctrl+A", "Select all"),
            ("Ctrl+C / X / V", "Copy / cut / paste"),
            ("Ctrl+D, Alt+drag", "Duplicate"),
            ("Delete", "Delete"),
            ("Arrows, Shift+arrows", "Nudge"),
            (
                "Ctrl+] / Ctrl+[",
                "Forward / backward (with Shift: front / back)",
            ),
            ("Ctrl+G / Ctrl+Shift+G", "Group / ungroup"),
            ("Ctrl+Shift+L", "Lock / unlock"),
            ("Ctrl+Shift+H", "Hide / show"),
            (
                "Shift / Ctrl + click",
                "Add to or remove from the selection",
            ),
            ("Alt+A / D / W / S", "Align left / right / top / bottom"),
            ("Alt+H / Alt+V", "Center horizontally / vertically"),
            ("Alt+Shift+H / V", "Distribute horizontally / vertically"),
            ("Alt+G", "Arrange in a grid"),
        ],
    ),
    (
        "Board",
        &[
            ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
            ("Ctrl+F", "Find"),
            ("Ctrl+N / O / S", "New / open / save"),
            ("Ctrl+Shift+S", "Save as"),
            ("Space+drag, middle drag", "Pan"),
            ("Ctrl+wheel, Ctrl+= / -", "Zoom"),
            ("Ctrl+0", "Zoom to 100%"),
            ("Home", "Back to the start"),
            ("G / Shift+G", "Grid style / snap to grid"),
            ("F1, ?", "This list"),
            ("Shift+F", "Focus on the selection or the view"),
            ("F5", "Present the frames"),
            ("L (presenting)", "Laser pointer"),
        ],
    ),
];

/// The list of keyboard shortcuts, above the other panels.
fn shortcuts_window(ctx: &Context, actions: &mut Vec<Action>) {
    let mut open = true;
    let max_height = ctx.content_rect().height() * 0.75;
    let shown = egui::Window::new("Keyboard shortcuts")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| {
                    for (topic, keys) in SHORTCUTS {
                        ui.strong(*topic);
                        egui::Grid::new(*topic)
                            .num_columns(2)
                            .spacing([16.0, 4.0])
                            .min_col_width(150.0)
                            .show(ui, |ui| {
                                for (key, what) in *keys {
                                    ui.label(RichText::new(*key).monospace());
                                    ui.label(*what);
                                    ui.end_row();
                                }
                            });
                        ui.add_space(8.0);
                    }
                });
        });
    // Toolbars are foreground areas too; keep this window over them.
    if let Some(shown) = shown {
        let layer = shown.response.layer_id;
        let on_top = ctx.memory(|m| m.areas().top_layer_id(Order::Foreground)) == Some(layer);
        if !on_top {
            ctx.move_to_top(layer);
            // The new order shows on the next frame.
            ctx.request_repaint();
        }
    }
    if !open {
        actions.push(Action::ToggleShortcuts);
    }
}

/// The search field at the top right, with the number of results and steps between them.
fn search_bar(ctx: &Context, info: SearchInfo, text: &mut String, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    Area::new(Id::unique("search bar"))
        .fixed_pos(Pos2::new(screen.right() - MARGIN, MARGIN))
        .pivot(Align2::RIGHT_TOP)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(text)
                            .hint_text("Find on the board")
                            .desired_width(220.0),
                    );
                    if info.focus {
                        field.request_focus();
                    }
                    if field.changed() {
                        actions.push(Action::Search(text.clone()));
                    }
                    let (enter, shift, escape, up, down) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.modifiers.shift,
                            i.key_pressed(egui::Key::Escape),
                            i.key_pressed(egui::Key::ArrowUp),
                            i.key_pressed(egui::Key::ArrowDown),
                        )
                    });
                    let typing = field.has_focus() || field.lost_focus();
                    if typing && enter {
                        actions.push(Action::SearchStep(!shift));
                        // Enter takes the focus away; keep typing.
                        field.request_focus();
                    }
                    if field.has_focus() && (up || down) {
                        actions.push(Action::SearchStep(down));
                    }
                    if typing && escape {
                        actions.push(Action::CloseSearch);
                    }
                    let status = match (info.count, info.has_query) {
                        (0, true) => "No results".to_owned(),
                        (0, false) => String::new(),
                        (n, _) => format!("{} of {n}", info.current + 1),
                    };
                    ui.label(status);
                    let some = info.count > 0;
                    if ui
                        .add_enabled(some, Button::new("↑"))
                        .on_hover_text("Previous (Shift+Enter)")
                        .clicked()
                    {
                        actions.push(Action::SearchStep(false));
                    }
                    if ui
                        .add_enabled(some, Button::new("↓"))
                        .on_hover_text("Next (Enter)")
                        .clicked()
                    {
                        actions.push(Action::SearchStep(true));
                    }
                    if ui.button("×").on_hover_text("Close (Esc)").clicked() {
                        actions.push(Action::CloseSearch);
                    }
                });
            });
        });
}

/// Tags and fill colors to show only some objects, and managing the board's tags.
fn filter_menu(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    let filter = model.filter;
    ui.strong("Tags");
    if model.tags.is_empty() {
        ui.weak("No tags yet: select objects and use Tags");
    }
    for (tag, count) in &model.tags {
        ui.horizontal(|ui| {
            if tag_dot(ui, tag.color)
                .on_hover_text("Change the color")
                .clicked()
            {
                actions.push(Action::RecolorTag(tag.id.clone()));
            }
            let on = filter.tags.contains(&tag.id);
            let label = format!("{} ({count})", tag.name);
            if ui
                .selectable_label(on, label)
                .on_hover_text("Show objects with this tag")
                .clicked()
            {
                actions.push(Action::FilterTag(tag.id.clone()));
            }
            if ui
                .small_button("×")
                .on_hover_text("Remove the tag from the board")
                .clicked()
            {
                actions.push(Action::RemoveTag(tag.id.clone()));
            }
        });
    }
    if !model.fill_colors.is_empty() {
        ui.separator();
        ui.strong("Colors");
        ui.horizontal_wrapped(|ui| {
            for &color in &model.fill_colors {
                let on = filter.colors.contains(&color);
                let tip = "Show objects filled with this color";
                if swatch_button(ui, color, on, tip).clicked() {
                    actions.push(Action::FilterColor(color));
                }
            }
        });
    }
    if !filter.is_empty() {
        ui.separator();
        if ui.button("Show everything").clicked() {
            actions.push(Action::ClearFilter);
            ui.close();
        }
    }
}

/// A small round button in a tag's color.
fn tag_dot(ui: &mut egui::Ui, color: Rgba) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    ui.painter()
        .circle_filled(rect.center(), 6.0, to_color32(color));
    response
}

/// Bold, italic, underline, strikethrough and colors for the text being edited, above it.
fn format_bar(ctx: &Context, format: &TextFormat, actions: &mut Vec<Action>) {
    let r = format.screen;
    let (anchor, pivot) = if (r.y0 as f32) - SELECTION_GAP > 90.0 {
        (
            Pos2::new(r.center().x as f32, r.y0 as f32 - 12.0),
            Align2::CENTER_BOTTOM,
        )
    } else {
        (
            Pos2::new(r.center().x as f32, r.y1 as f32 + 12.0),
            Align2::CENTER_TOP,
        )
    };
    Area::new(Id::unique("format"))
        .fixed_pos(anchor)
        .pivot(pivot)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let m = format.marks;
                    let marks = [
                        (
                            Mark::Bold,
                            m.bold,
                            RichText::new("B").strong(),
                            "Bold (Ctrl+B)",
                        ),
                        (
                            Mark::Italic,
                            m.italic,
                            RichText::new("I").italics(),
                            "Italic (Ctrl+I)",
                        ),
                        (
                            Mark::Underline,
                            m.underline,
                            RichText::new("U").underline(),
                            "Underline (Ctrl+U)",
                        ),
                        (
                            Mark::Strike,
                            m.strike,
                            RichText::new("S").strikethrough(),
                            "Strikethrough (Ctrl+Shift+X)",
                        ),
                    ];
                    for (mark, on, label, tip) in marks {
                        if ui.selectable_label(on, label).on_hover_text(tip).clicked() {
                            actions.push(Action::ToggleMark(mark));
                        }
                    }
                    ui.separator();
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for color in TEXT_COLORS {
                        let (rect, response) =
                            ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                        // The object's own color is shown as the default dark ink.
                        let swatch = color.unwrap_or(Rgba::rgb(0x1F, 0x29, 0x37));
                        paint_swatch(ui.painter(), rect, swatch);
                        if m.color == color {
                            ui.painter().rect_stroke(
                                rect.expand(2.0),
                                6,
                                Stroke::new(2.0, ACCENT),
                                egui::StrokeKind::Outside,
                            );
                        }
                        let tip = if color.is_none() {
                            "Text color of the object"
                        } else {
                            "Text color"
                        };
                        if response.on_hover_text(tip).clicked() {
                            actions.push(Action::MarkColor(color));
                        }
                    }
                });
            });
        });
}

fn toggle(popup: &mut Option<Popup>, which: Popup, clicked: bool) {
    if clicked {
        *popup = if *popup == Some(which) {
            None
        } else {
            Some(which)
        };
    }
}

/// A slider whose drag is a single undo step.
fn style_slider(
    ui: &mut egui::Ui,
    mut value: f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    actions: &mut Vec<Action>,
    change: impl Fn(f64) -> StyleChange,
) {
    let response = ui.add(Slider::new(&mut value, range).suffix(suffix));
    style_drag(&response, actions, || change(value));
}

/// Style actions for a slider's `response`, so that a drag is a single undo step.
fn style_drag(response: &Response, actions: &mut Vec<Action>, change: impl Fn() -> StyleChange) {
    if response.drag_started() {
        actions.push(Action::BeginStyleDrag);
    }
    if response.changed() {
        actions.push(Action::SetStyle(change()));
    }
    if response.drag_stopped() {
        actions.push(Action::EndStyleDrag);
    }
}

/// Grid of color swatches; returns the picked color. `with_none` offers "no color" too.
fn palette(ui: &mut egui::Ui, current: Rgba, with_none: bool) -> Option<Rgba> {
    let colors = if with_none {
        &PALETTE[..]
    } else {
        &PALETTE[..PALETTE.len() - 1]
    };
    color_grid(ui, colors, current, 4)
}

/// Swatches of `colors`, `columns` per row, with `current` marked; returns the picked one.
fn color_grid(ui: &mut egui::Ui, colors: &[Rgba], current: Rgba, columns: usize) -> Option<Rgba> {
    let mut picked = None;
    egui::Grid::new("palette")
        .spacing(Vec2::splat(4.0))
        .show(ui, |ui| {
            for (i, &color) in colors.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                paint_swatch(ui.painter(), rect, color);
                if color == current {
                    ui.painter().rect_stroke(
                        rect.expand(2.0),
                        6,
                        Stroke::new(2.0, ACCENT),
                        egui::StrokeKind::Outside,
                    );
                }
                let name = if color.to_bytes()[3] == 0 { "None" } else { "" };
                let response = if name.is_empty() {
                    response
                } else {
                    response.on_hover_text(name)
                };
                if response.clicked() {
                    picked = Some(color);
                }
                if i % columns == columns - 1 {
                    ui.end_row();
                }
            }
        });
    picked
}

fn swatch_button(ui: &mut egui::Ui, color: Rgba, ring: bool, tooltip: &str) -> Response {
    icon_button(ui, false, tooltip, |painter, rect, _| {
        if ring {
            let stroke = Stroke::new(4.0, to_color32(color));
            painter.circle_stroke(rect.center(), rect.width() / 2.0 - 2.0, stroke);
            painter.circle_stroke(
                rect.center(),
                rect.width() / 2.0,
                Stroke::new(1.0, Color32::from_gray(200)),
            );
        } else {
            paint_swatch(painter, rect, color);
        }
    })
}

fn paint_swatch(painter: &Painter, rect: Rect, color: Rgba) {
    if color.to_bytes()[3] == 0 {
        painter.rect_filled(rect, 4, Color32::WHITE);
        let slash = [rect.left_bottom(), rect.right_top()];
        painter.line_segment(slash, Stroke::new(1.5, Color32::from_rgb(0xF2, 0x55, 0x55)));
    } else {
        painter.rect_filled(rect, 4, to_color32(color));
    }
    painter.rect_stroke(
        rect,
        4,
        Stroke::new(1.0, Color32::from_gray(200)),
        egui::StrokeKind::Inside,
    );
}

/// A square toolbar button with a painted icon.
fn icon_button(
    ui: &mut egui::Ui,
    selected: bool,
    tooltip: &str,
    paint: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
    let painter = ui.painter();
    let enabled = ui.is_enabled();
    if selected {
        painter.rect_filled(rect, 8, ACCENT.gamma_multiply(0.15));
    } else if response.hovered() && enabled {
        painter.rect_filled(rect, 8, Color32::from_gray(238));
    }
    let color = if selected {
        ACCENT
    } else if enabled {
        INK
    } else {
        Color32::from_gray(190)
    };
    paint(painter, rect.shrink(9.0), color);
    response.on_hover_text(tooltip)
}

fn paint_cursor(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let points = vec![
        at(0.15, 0.0),
        at(0.85, 0.62),
        at(0.5, 0.65),
        at(0.3, 1.0),
        at(0.15, 0.0),
    ];
    painter.add(Shape::line(points, Stroke::new(1.6, color)));
}

/// The selected table: its size, and the selected cells.
#[derive(Clone, Copy, Debug)]
pub struct TableInfo {
    pub rows: usize,
    pub cols: usize,
    pub cells: Option<pb_core::CellRange>,
    pub can_merge: bool,
    pub can_split: bool,
}

/// A table: a box split into cells.
fn paint_table(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let stroke = Stroke::new(1.6, color);
    painter.rect_stroke(
        Rect::from_min_max(at(0.0, 0.1), at(1.0, 0.9)),
        1,
        stroke,
        egui::StrokeKind::Middle,
    );
    painter.line_segment([at(0.0, 0.37), at(1.0, 0.37)], stroke);
    painter.line_segment([at(0.0, 0.63), at(1.0, 0.63)], stroke);
    painter.line_segment([at(0.5, 0.1), at(0.5, 0.9)], stroke);
}

/// A kanban board: three columns with cards.
fn paint_kanban(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let stroke = Stroke::new(1.6, color);
    for (x, cards) in [(0.0, 3), (0.37, 2), (0.74, 1)] {
        for i in 0..cards {
            let y = 0.08 + i as f32 * 0.3;
            painter.rect_stroke(
                Rect::from_min_max(at(x, y), at(x + 0.26, y + 0.22)),
                1,
                stroke,
                egui::StrokeKind::Middle,
            );
        }
    }
}

/// A reaction's icon in its own color.
fn paint_reaction(painter: &Painter, rect: Rect, reaction: pb_core::Reaction) {
    let bounds = kurbo::Rect::new(
        rect.left().into(),
        rect.top().into(),
        rect.right().into(),
        rect.bottom().into(),
    );
    let color = to_color32(reaction.color());
    let (fill, stroke) = reaction.icon(bounds);
    let polylines = |path: kurbo::BezPath| {
        let mut lines: Vec<(Vec<Pos2>, bool)> = Vec::new();
        kurbo::flatten(path, 0.2, |el| match el {
            PathEl::MoveTo(p) => lines.push((vec![Pos2::new(p.x as f32, p.y as f32)], false)),
            PathEl::LineTo(p) => {
                if let Some((line, _)) = lines.last_mut() {
                    line.push(Pos2::new(p.x as f32, p.y as f32));
                }
            }
            PathEl::ClosePath => {
                if let Some((_, closed)) = lines.last_mut() {
                    *closed = true;
                }
            }
            _ => {}
        });
        lines
    };
    for (points, _) in polylines(fill) {
        painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    }
    let width = rect.width() / 8.0;
    for (points, closed) in polylines(stroke) {
        painter.add(if closed {
            Shape::closed_line(points, Stroke::new(width, color))
        } else {
            Shape::line(points, Stroke::new(width, color))
        });
    }
}

/// A comment: a speech bubble.
fn paint_comment(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let stroke = Stroke::new(1.6, color);
    painter.rect_stroke(
        Rect::from_min_max(at(0.05, 0.1), at(0.95, 0.72)),
        4,
        stroke,
        egui::StrokeKind::Middle,
    );
    painter.line_segment([at(0.25, 0.72), at(0.18, 0.95)], stroke);
    painter.line_segment([at(0.18, 0.95), at(0.45, 0.72)], stroke);
}

/// A document: a page with a heading and lines of text.
fn paint_doc(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let stroke = Stroke::new(1.6, color);
    painter.rect_stroke(
        Rect::from_min_max(at(0.15, 0.0), at(0.85, 1.0)),
        2,
        stroke,
        egui::StrokeKind::Middle,
    );
    painter.line_segment([at(0.28, 0.2), at(0.6, 0.2)], Stroke::new(2.4, color));
    for y in [0.42, 0.6, 0.78] {
        painter.line_segment([at(0.28, y), at(0.72, y)], stroke);
    }
}

/// A mind map: a central topic with branches to both sides.
fn paint_mindmap(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let stroke = Stroke::new(1.6, color);
    painter.rect_stroke(
        Rect::from_center_size(at(0.5, 0.5), Vec2::new(0.34, 0.22) * rect.width()),
        3,
        stroke,
        egui::StrokeKind::Middle,
    );
    for (x0, x1) in [(0.67, 0.95), (0.33, 0.05)] {
        for y in [0.2, 0.8] {
            painter.line_segment([at(x0, 0.5), at(x1, y)], stroke);
        }
    }
}

/// A frame: its area under a title line.
fn paint_frame(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let stroke = Stroke::new(1.6, color);
    painter.line_segment([at(0.0, 0.05), at(0.45, 0.05)], stroke);
    painter.rect_stroke(
        Rect::from_min_max(at(0.0, 0.28), at(1.0, 0.95)),
        1,
        stroke,
        egui::StrokeKind::Middle,
    );
}

fn paint_pen(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let body = vec![
        at(0.25, 0.95),
        at(0.05, 0.75),
        at(0.7, 0.1),
        at(0.9, 0.3),
        at(0.25, 0.95),
    ];
    painter.add(Shape::line(body, Stroke::new(1.6, color)));
    painter.line_segment([at(0.05, 0.75), at(0.0, 1.0)], Stroke::new(1.6, color));
    painter.line_segment([at(0.0, 1.0), at(0.25, 0.95)], Stroke::new(1.6, color));
}

fn paint_marker(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let highlight = Color32::from_rgba_unmultiplied(0xFF, 0xD5, 0x4F, 200);
    painter.line_segment([at(0.1, 0.9), at(0.9, 0.1)], Stroke::new(7.0, highlight));
    painter.line_segment([at(0.1, 0.9), at(0.9, 0.1)], Stroke::new(1.2, color));
}

fn paint_eraser(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let body = vec![
        at(0.05, 0.6),
        at(0.55, 0.1),
        at(0.95, 0.5),
        at(0.45, 1.0),
        at(0.05, 0.6),
    ];
    painter.add(Shape::line(body, Stroke::new(1.6, color)));
    painter.line_segment([at(0.3, 0.35), at(0.7, 0.75)], Stroke::new(1.6, color));
}

fn paint_text(painter: &Painter, rect: Rect, color: Color32) {
    let font = egui::FontId::proportional(rect.height() * 1.15);
    painter.text(rect.center(), Align2::CENTER_CENTER, "T", font, color);
}

fn paint_sticky(painter: &Painter, rect: Rect, color: Color32) {
    let note = rect.shrink(1.0);
    painter.rect_stroke(note, 2, Stroke::new(1.6, color), egui::StrokeKind::Middle);
    for (y, right) in [(0.38, 0.75), (0.6, 0.6)] {
        let y = note.top() + note.height() * y;
        let x = |t: f32| note.left() + note.width() * t;
        painter.line_segment(
            [Pos2::new(x(0.25), y), Pos2::new(x(right), y)],
            Stroke::new(1.4, color),
        );
    }
}

fn paint_image(painter: &Painter, rect: Rect, color: Color32) {
    let frame = rect.shrink(1.0);
    let stroke = Stroke::new(1.6, color);
    painter.rect_stroke(frame, 2, stroke, egui::StrokeKind::Middle);
    let at = |x: f32, y: f32| {
        Pos2::new(
            frame.left() + frame.width() * x,
            frame.top() + frame.height() * y,
        )
    };
    painter.circle_stroke(at(0.68, 0.32), frame.width() * 0.1, Stroke::new(1.4, color));
    let hills = vec![
        at(0.0, 0.85),
        at(0.35, 0.5),
        at(0.6, 0.75),
        at(0.75, 0.62),
        at(1.0, 0.85),
    ];
    painter.add(Shape::line(hills, stroke));
}

fn paint_connector(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| {
        Pos2::new(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    let stroke = Stroke::new(1.6, color);
    painter.circle_stroke(at(0.12, 0.88), rect.width() * 0.1, stroke);
    painter.add(Shape::line(
        vec![at(0.2, 0.8), at(0.2, 0.3), at(0.85, 0.3)],
        stroke,
    ));
    painter.add(Shape::convex_polygon(
        vec![at(1.0, 0.3), at(0.75, 0.15), at(0.75, 0.45)],
        color,
        Stroke::NONE,
    ));
}

fn paint_shape(painter: &Painter, rect: Rect, color: Color32, kind: ShapeKind) {
    // In the shape's own proportions (a terminator is wide), within the icon's square.
    let (w, h) = kind.default_size();
    let side = f64::from(rect.width().min(rect.height()));
    let fit = side / w.max(h);
    let center = kurbo::Point::new(rect.center().x.into(), rect.center().y.into());
    let bounds = kurbo::Rect::from_center_size(center, (w * fit, h * fit));
    let stroke = Stroke::new(1.6, color);
    // Each subpath on its own: the outline's closed, the details' open.
    let draw = |path: kurbo::BezPath| {
        let mut points: Vec<Pos2> = Vec::new();
        let flush = |points: &mut Vec<Pos2>, closed: bool| {
            if points.len() > 1 {
                let line = std::mem::take(points);
                painter.add(if closed {
                    Shape::closed_line(line, stroke)
                } else {
                    Shape::line(line, stroke)
                });
            }
            points.clear();
        };
        kurbo::flatten(path, 0.2, |el| match el {
            PathEl::MoveTo(p) => {
                flush(&mut points, false);
                points.push(Pos2::new(p.x as f32, p.y as f32));
            }
            PathEl::LineTo(p) => points.push(Pos2::new(p.x as f32, p.y as f32)),
            PathEl::ClosePath => flush(&mut points, true),
            _ => {}
        });
        flush(&mut points, false);
    };
    draw(kind.outline(bounds));
    if let Some(details) = kind.details(bounds) {
        draw(details);
    }
}

/// A curved arrow for undo (counter-clockwise) and redo.
fn paint_turn(painter: &Painter, rect: Rect, color: Color32, redo: bool) {
    let c = rect.center() + Vec2::new(0.0, rect.height() * 0.15);
    let r = rect.width() * 0.4;
    let mirror = if redo { -1.0 } else { 1.0 };
    let at = |angle: f32| c + Vec2::new(mirror * r * angle.cos(), r * angle.sin());
    let arc: Vec<_> = (0..=16)
        .map(|i| at(std::f32::consts::PI * (1.0 + i as f32 / 16.0)))
        .collect();
    let start = arc[0];
    painter.add(Shape::line(arc, Stroke::new(1.6, color)));
    let head = [
        start + Vec2::new(-mirror * 4.0, -2.0),
        start + Vec2::new(0.0, 4.5),
        start + Vec2::new(mirror * 4.0, -2.0),
    ];
    painter.add(Shape::line(head.to_vec(), Stroke::new(1.6, color)));
}

fn to_color32(color: Rgba) -> Color32 {
    let [r, g, b, a] = color.to_bytes();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}
