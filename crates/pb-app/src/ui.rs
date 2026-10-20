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

use crate::i18n::{tr, trf};

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
    /// Opens or closes the sharing window.
    ToggleShare,
    /// Follows (or stops following) someone's view on a shared board.
    Follow(pb_sync::ClientId),
    StopFollowing,
    /// Asks everyone to follow this view, or stops asking.
    ToggleLead,
    /// The name to go by on boards (pointers, comments, votes).
    SetName(String),
    /// Makes a shared board of this one on the server at this address.
    CreateShared(String),
    /// Opens the shared board at this link (in place of this one).
    JoinShared(String),
    StopSharing,
    /// Connects to the server again now.
    RetrySharing,
    /// Signs in on the server (as typed) with this email.
    SignIn(String, String),
    CancelSignIn,
    SignOut,
    RefreshBoards,
    /// Opens a board of the account, by id.
    OpenMyBoard(String),
    /// Lets someone in to the shared board by email: `edit`, `comment`, `view` or `none`.
    SetMember(String, String),
    /// A new link for this role on the shared board; the old one stops working.
    RenewLink(pb_core::Role),
    /// Deletes the shared board from the server, after asking.
    DeleteSharedBoard,
    /// Deletes the account and the boards it owns from the server, after asking.
    DeleteAccount,
    /// Opens or closes the plugins window.
    TogglePlugins,
    InstallPlugin,
    /// Offers to install the plugin in these files.
    InstallPluginFiles(Vec<std::path::PathBuf>),
    /// Runs a plugin from its own folder, reloading it as it changes (desktop).
    LoadDevPlugin,
    SetPluginEnabled(String, bool),
    RemovePlugin(String),
    /// Runs this command of this plugin.
    RunPluginCommand(String, String),
    /// Something happened in this plugin's panel.
    PluginPanel(String, serde_json::Value),
    ClosePluginPanel(String),
    /// Saves a plugin being developed as one `.pbplugin` file, to give to others.
    PackPlugin(String),
    /// Puts text on the clipboard (a link to share).
    CopyText(String),
    /// Opens this board (one kept in the browser, or a file PaintBoard was started with).
    OpenFile(std::path::PathBuf),
    /// Lets the browser download the board as a file (web only).
    DownloadBoard,
    /// Deletes a board kept in the browser, after asking.
    DeleteStored(std::path::PathBuf),
    /// Gives a board kept in the browser a new name.
    RenameStored(std::path::PathBuf, String),
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
    /// Starts recording a talktrack.
    StartRecording,
    /// Stops recording and keeps the talktrack.
    StopRecording,
    /// Stops recording and throws it away.
    CancelRecording,
    /// Plays the talktrack with this id.
    PlayTalktrack(Arc<str>),
    /// Pauses the talktrack playing, or plays on.
    TogglePlayback,
    /// Goes to this many seconds into the talktrack playing.
    SeekPlayback(f64),
    StopPlayback,
    RemoveTalktrack(Arc<str>),
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
    /// Hides or shows one of the panels around the board.
    TogglePanel(Panel),
    /// Hides all panels but a button to bring them back, or shows them again (Ctrl+\).
    ToggleClean,
    /// The window takes the whole screen or goes back (F11).
    ToggleFullscreen,
    /// Colors the board's background; `None` is the usual color.
    SetBackground(Option<Rgba>),
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
    /// Starts a countdown of this many milliseconds.
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
    /// Shows or hides the boards kept in the browser.
    ToggleBoards,
    /// Lets the browser download a board kept in it.
    DownloadStored(std::path::PathBuf),
    /// Adds template `i` of `templates::TEMPLATES` in the middle of the view.
    InsertTemplate(usize),
    /// Opens the window for making a diagram from text.
    OpenDiagram,
    /// Saves the board as it is now as a version with this name (blank: a numbered one).
    SaveVersion(String),
    /// Shows a saved version instead of the board.
    OpenVersion(Arc<str>),
    CloseVersion,
    /// Brings the board back to the version shown.
    RestoreVersion,
    /// Forgets a saved version.
    RemoveVersion(Arc<str>),
    /// Opens a PDF of the board, a page per frame, in the system's viewer to print.
    Print,
    /// Switches between the dark and the light theme.
    ToggleTheme,
    /// Switches the UI between English and Russian.
    ToggleLanguage,
    /// Sets the UI's size relative to normal.
    SetUiScale(f32),
    ToggleHighContrast,
    /// Opens the window for saving a picture of the board.
    OpenExport,
    CloseExport,
    /// Saves a picture as asked.
    Export(crate::export::Settings),
    /// Makes a diagram from this Mermaid text.
    InsertDiagram(String),
    CloseDiagram,
    /// Opens the window that draws graphs of functions.
    OpenGraph,
    /// Draws graphs of the formulas (separated by `;`) for x from `from` to `to`.
    InsertGraph {
        formulas: String,
        from: String,
        to: String,
    },
    CloseGraph,
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
    /// Opens the command palette, or closes it (Ctrl+K).
    TogglePalette,
    ClosePalette,
    /// Looks on the board for objects with this text, to offer in the palette.
    PaletteQuery(String),
    /// Brings this area of the board (world) into view, like a search result.
    ShowArea(kurbo::Rect),
    /// Shows everything on the board, at most at 100%.
    ZoomToFit,
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
    /// Turns the selected book (a PDF shown a page at a time) to a page, from 0.
    TurnPage(u32),
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
/// The panels around the board that can be put away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    /// The board menu at the top.
    Menu,
    /// The tools on the left.
    Tools,
    /// The view controls (and the minimap) at the bottom.
    View,
}

/// Which panels show. Clean, the board shows alone with one button to bring them back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panels {
    pub menu: bool,
    pub tools: bool,
    pub view: bool,
    pub clean: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            menu: true,
            tools: true,
            view: true,
            clean: false,
        }
    }
}

impl Panels {
    pub fn shows(&self, panel: Panel) -> bool {
        !self.clean
            && match panel {
                Panel::Menu => self.menu,
                Panel::Tools => self.tools,
                Panel::View => self.view,
            }
    }

    /// Hides or shows `panel`; out of clean mode, that panel alone comes back.
    pub fn toggle(&mut self, panel: Panel) {
        if self.clean {
            *self = Self {
                menu: false,
                tools: false,
                view: false,
                clean: false,
            };
        }
        let shown = match panel {
            Panel::Menu => &mut self.menu,
            Panel::Tools => &mut self.tools,
            Panel::View => &mut self.view,
        };
        *shown = !*shown;
    }

    /// As words for the settings file: the hidden panels and `clean`.
    pub fn to_words(self) -> String {
        [
            (!self.menu, "no-menu"),
            (!self.tools, "no-tools"),
            (!self.view, "no-view"),
            (self.clean, "clean"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, word)| *word)
        .collect::<Vec<_>>()
        .join(" ")
    }

    pub fn from_words(text: &str) -> Self {
        let has = |word| text.split_whitespace().any(|w| w == word);
        Self {
            menu: !has("no-menu"),
            tools: !has("no-tools"),
            view: !has("no-view"),
            clean: has("clean"),
        }
    }
}

/// A command a plugin offers.
pub struct PluginCommandView {
    pub plugin: String,
    pub plugin_name: String,
    pub id: String,
    pub title: String,
    /// Offered for the selection in the right-click menu too.
    pub selection: bool,
}

/// An installed plugin, for the plugins window.
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// What it may do, in words.
    pub permissions: Vec<String>,
    pub enabled: bool,
    pub running: bool,
    /// Run from its own folder for development.
    pub dev: bool,
    pub error: Option<String>,
    /// (level, text), oldest first.
    pub log: Vec<(String, String)>,
}

pub struct Model<'a> {
    pub plugin_commands: Vec<PluginCommandView>,
    /// The plugins window, when open.
    pub plugins: Option<Vec<PluginView>>,
    /// Panels plugins show: plugin id, plugin name, the panel as described.
    pub plugin_panels: Vec<(String, String, serde_json::Value)>,
    /// Messages from plugins: the plugin's name, the text.
    pub notices: Vec<(String, String)>,
    /// Which panels show.
    pub panels: Panels,
    /// The window fills the screen.
    pub fullscreen: bool,
    /// The board's own background color, if it has one.
    pub background: Option<Rgba>,
    pub board: &'a str,
    /// Running in the browser.
    pub web: bool,
    /// Boards kept in the browser (web only).
    pub stored_boards: Vec<std::path::PathBuf>,
    /// The open board's file, if it has one.
    pub open_board: Option<std::path::PathBuf>,
    /// The board's frames with their titles, back to front.
    pub frames: Vec<(ObjectId, String)>,
    pub tool: ToolKind,
    pub last_shape: ShapeKind,
    /// The structure tool (frame, table, …) the shared button offers.
    pub last_structure: ToolKind,
    /// Style of the pen or marker, whichever is active.
    pub ink: InkStyle,
    /// The pen's options show (they go away while drawing; its button brings them back).
    pub ink_panel: bool,
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
    /// The command palette, when it is open: objects whose text fits what is typed, each
    /// with a line of its text and its area on the board.
    pub palette: Option<Vec<(String, kurbo::Rect)>>,
    /// The right-click menu, when it is open.
    pub context_menu: Option<ContextMenuInfo>,
    pub show_shortcuts: bool,
    /// The sharing window, when open.
    pub share_window: Option<ShareView>,
    /// Following someone (or leading): what to say, the color, and whether leading.
    pub follow_banner: Option<(String, u32, bool)>,
    /// The board is shared: how it is going, for the top bar.
    pub share_badge: Option<(String, bool)>,
    /// Other people's pointers (window position, logical pixels), names and colors.
    pub cursors: Vec<(kurbo::Point, String, u32)>,
    /// Outlines of what others have selected (window rect, logical pixels) and their color.
    pub their_selections: Vec<(kurbo::Rect, u32)>,
    /// How many objects are hidden, and whether they are shown faded.
    pub hidden: (usize, bool),
    /// The board is read-only: only looking and commenting.
    pub read_only: bool,
    /// Opened with a link for viewing: nothing can be changed, not even the timer.
    pub view_only: bool,
    /// The layers panel, while open.
    pub layers: Option<LayersView>,
    /// Recording a talktrack: seconds so far, and whether there is sound.
    pub recording: Option<(u64, bool)>,
    /// A talktrack playing.
    pub playback: Option<PlaybackView>,
    /// The board's talktracks: id, title and length in milliseconds.
    pub talktracks: Vec<(Arc<str>, String, u32)>,
    /// The diagram-from-text window, when it is open, with the last error.
    pub diagram: Option<Option<String>>,
    /// The graph window, when it is open, with the last error.
    pub graph: Option<Option<String>>,
    /// The template gallery is open.
    pub show_templates: bool,
    /// The window of the boards kept in the browser (the web version's start screen).
    pub show_boards: bool,
    /// The dark theme is on.
    pub dark: bool,
    /// The UI's size relative to normal, and whether high contrast is on.
    pub ui_scale: f32,
    pub high_contrast: bool,
    /// The export window is open.
    pub export: Option<ExportInfo>,
    /// Saved versions, newest first.
    pub versions: Vec<VersionItem>,
    /// The saved version shown instead of the board.
    pub version_view: Option<VersionBanner>,
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

pub struct VersionItem {
    pub id: Arc<str>,
    pub name: String,
    /// How long ago it was saved, for people.
    pub when: String,
    pub author: String,
}

pub struct VersionBanner {
    pub name: String,
    pub when: String,
    pub author: String,
    /// How much changed since.
    pub summary: String,
}

#[derive(Clone, Copy, Debug)]
pub struct ExportInfo {
    /// Something is selected to export.
    pub has_selection: bool,
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
#[derive(Clone, Debug, PartialEq)]
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
    /// A single book (a PDF shown a page at a time) is selected: the page it shows (from
    /// 0) and how many it has.
    pub book: Option<(u32, u32)>,
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

/// A talktrack playing.
pub struct PlaybackView {
    pub title: String,
    /// Seconds in, and how many there are.
    pub position: f64,
    pub length: f64,
    pub paused: bool,
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
/// Background colors a board can have, besides the usual one.
const BOARD_COLORS: [Rgba; 8] = [
    Rgba::rgb(0xFF, 0xFF, 0xFF),
    Rgba::rgb(0xFB, 0xF6, 0xE9),
    Rgba::rgb(0xE9, 0xEC, 0xEF),
    Rgba::rgb(0xE3, 0xF0, 0xFB),
    Rgba::rgb(0xE6, 0xF4, 0xE4),
    Rgba::rgb(0xFF, 0xF6, 0xCC),
    Rgba::rgb(0xFC, 0xE8, 0xEC),
    Rgba::rgb(0xEE, 0xE8, 0xF8),
];

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

/// The dark theme is on (see [`Ui::set_dark`]); panels are drawn to match.
static DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn dark() -> bool {
    DARK.load(std::sync::atomic::Ordering::Relaxed)
}

/// High contrast is on (see [`Ui::set_high_contrast`]).
static CONTRAST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn high_contrast() -> bool {
    CONTRAST.load(std::sync::atomic::Ordering::Relaxed)
}

/// The UI's size relative to normal, as `f32` bits (see [`Ui::set_scale`]).
static SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000);

/// The UI's size relative to normal: UI points per logical pixel of the window.
fn ui_scale() -> f32 {
    f32::from_bits(SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

/// Sizes the UI can take, relative to normal.
pub const UI_SCALES: std::ops::RangeInclusive<f32> = 0.75..=2.0;

/// A place on the window (logical pixels, as the app measures it) in UI points.
fn to_points(r: kurbo::Rect) -> kurbo::Rect {
    r.scale_from_origin(1.0 / f64::from(ui_scale()))
}
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
    /// What is typed in the command palette, and what it remembers.
    palette: PaletteState,
    /// Mermaid text in the diagram window, kept between openings.
    diagram_text: String,
    /// The graph window's formulas and x range.
    graph_text: (String, String, String),
    /// A comment being written.
    comment_text: String,
    /// The layer being renamed, with the name typed so far.
    layer_name: Option<(Arc<str>, String)>,
    /// Choices in the export window, kept between openings.
    export: crate::export::Settings,
    /// The export window was open in the last frame.
    export_shown: bool,
    /// Name typed for a version to save.
    version_name: String,
    /// Time typed for the countdown, like `7` or `2:30`.
    timer_text: String,
    /// The board kept in the browser being renamed, and the name typed.
    renaming_board: Option<(std::path::PathBuf, String)>,
    /// What is typed in the sharing window.
    share_texts: ShareTexts,
    /// What is typed in plugins' panels, by plugin and field.
    panel_values: std::collections::HashMap<(String, String), String>,
    repaint_at: Option<Instant>,
}

impl Ui {
    pub fn new(window: &Window) -> Self {
        let ctx = Context::default();
        ctx.set_fonts(fonts());
        ctx.set_visuals(egui::Visuals::light());
        // Ctrl+= / Ctrl+- / Ctrl+0 zoom the board; the UI's size has its own setting.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
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
            palette: PaletteState::default(),
            diagram_text: DIAGRAM_EXAMPLE.to_owned(),
            graph_text: (
                "sin(x); x^2/4 - 1".to_owned(),
                "-5".to_owned(),
                "5".to_owned(),
            ),
            comment_text: String::new(),
            layer_name: None,
            export: crate::export::Settings::default(),
            export_shown: false,
            version_name: String::new(),
            timer_text: String::new(),
            renaming_board: None,
            share_texts: ShareTexts::default(),
            panel_values: std::collections::HashMap::new(),
            repaint_at: None,
        }
    }

    /// Switches the UI between the dark and the light look.
    pub fn set_dark(&mut self, dark: bool) {
        DARK.store(dark, std::sync::atomic::Ordering::Relaxed);
        self.apply_visuals();
    }

    /// Black or white text and clear outlines on panels and buttons.
    pub fn set_high_contrast(&mut self, on: bool) {
        CONTRAST.store(on, std::sync::atomic::Ordering::Relaxed);
        self.apply_visuals();
    }

    /// Makes the whole UI larger or smaller (the board is zoomed on its own).
    pub fn set_scale(&mut self, scale: f32) {
        let scale = scale.clamp(*UI_SCALES.start(), *UI_SCALES.end());
        SCALE.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
        self.ctx.set_zoom_factor(scale);
    }

    fn apply_visuals(&self) {
        let dark = dark();
        let mut visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        if high_contrast() {
            let (ink, edge) = if dark {
                (Color32::WHITE, Color32::from_gray(0xC8))
            } else {
                (Color32::BLACK, Color32::from_gray(0x40))
            };
            visuals.override_text_color = Some(ink);
            for widget in [
                &mut visuals.widgets.noninteractive,
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
                &mut visuals.widgets.open,
            ] {
                widget.fg_stroke = Stroke::new(1.5, ink);
                widget.bg_stroke = Stroke::new(1.0, edge);
            }
            visuals.selection.stroke = Stroke::new(1.5, ink);
            visuals.window_stroke = Stroke::new(1.5, edge);
        }
        self.ctx.set_visuals(visuals);
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

    /// Pastes `text` into the focused text field (where egui cannot read the clipboard
    /// itself: the browser).
    pub fn paste(&mut self, text: String) {
        self.state
            .egui_input_mut()
            .events
            .push(egui::Event::Paste(text));
        self.ctx.request_repaint();
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
        let palette = &mut self.palette;
        let diagram_text = &mut self.diagram_text;
        let graph_text = &mut self.graph_text;
        let comment_text = &mut self.comment_text;
        let layer_name = &mut self.layer_name;
        let export = &mut self.export;
        let export_shown = &mut self.export_shown;
        let version_name = &mut self.version_name;
        let timer_text = &mut self.timer_text;
        let renaming_board = &mut self.renaming_board;
        let share_texts = &mut self.share_texts;
        let panel_values = &mut self.panel_values;
        let output = self.ctx.run_pass(input, |ctx| {
            others(ctx, model);
            if let Some(banner) = &model.follow_banner {
                follow_banner(ctx, banner, &mut actions);
            }
            if let Some(timer) = &model.timer {
                timer_pill(ctx, timer, !model.view_only, &mut actions);
            }
            if let Some(voting) = model.voting.filter(|v| v.open) {
                voting_banner(ctx, voting, &mut actions);
            }
            if let Some(results) = &model.results {
                results_window(ctx, results, &mut actions);
            }
            if let Some((seconds, sound)) = model.recording {
                recording_pill(ctx, seconds, sound, &mut actions);
            }
            if let Some(playback) = &model.playback {
                // Nothing but the board and the controls, like presenting.
                *popup = None;
                playback_bar(ctx, playback, &mut actions);
                return;
            }
            if let Some((index, count, title)) = &model.presentation {
                // Nothing but the board and the controls.
                *popup = None;
                presentation_bar(ctx, *index, *count, title, model.laser, &mut actions);
                return;
            }
            if let Some(version) = &model.version_view {
                // Nothing but the board as it was and the way back.
                *popup = None;
                version_bar(ctx, version, model.read_only, &mut actions);
                return;
            }
            let panels = model.panels;
            if model.focus {
                focus_bar(ctx, &mut actions);
            } else if panels.shows(Panel::Menu) {
                let typed = Typed {
                    version_name,
                    timer_text,
                };
                board_menu(ctx, model, typed, &mut actions);
            }
            if panels.shows(Panel::Tools) {
                if model.read_only {
                    read_only_toolbar(ctx, model, &mut actions);
                } else {
                    toolbar(ctx, model, &mut actions);
                }
            }
            if panels.shows(Panel::View) {
                let view = view_controls(ctx, model, &mut actions);
                if let Some(map) = model.minimap.as_ref().filter(|_| !model.focus) {
                    minimap(ctx, map, view, &mut actions);
                }
            }
            panel_handles(ctx, panels, model.focus, &mut actions);
            match model.layers.as_ref().filter(|_| !panels.clean) {
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
                context_menu(ctx, info, &model.plugin_commands, &mut actions);
            }
            if model.show_shortcuts {
                shortcuts_window(ctx, &mut actions);
            }
            match &model.palette {
                Some(objects) => command_palette(ctx, model, objects, palette, &mut actions),
                None => {
                    palette.text.clear();
                    palette.picked = 0;
                }
            }
            if let Some(error) = &model.diagram {
                diagram_window(ctx, error.as_deref(), diagram_text, &mut actions);
            }
            if let Some(error) = &model.graph {
                graph_window(ctx, error.as_deref(), graph_text, &mut actions);
            }
            match model.export {
                Some(info) => {
                    if !*export_shown {
                        // Opened anew: what is selected, if anything, is what to export.
                        if info.has_selection {
                            export.scope = crate::export::Scope::Selection;
                        } else if export.scope == crate::export::Scope::Selection {
                            export.scope = crate::export::Scope::Board;
                        }
                    }
                    export_window(ctx, model, info, export, &mut actions);
                    *export_shown = true;
                }
                None => *export_shown = false,
            }
            if model.show_templates {
                templates_window(ctx, &mut actions);
            }
            if model.show_boards {
                boards_window(ctx, model, renaming_board, &mut actions);
            }
            if let Some(view) = &model.share_window {
                share_window(ctx, view, share_texts, &mut actions);
            }
            if let Some(plugins) = &model.plugins {
                plugins_window(ctx, plugins, model.web, &mut actions);
            }
            for (plugin, name, panel) in &model.plugin_panels {
                plugin_panel(ctx, plugin, name, panel, panel_values, &mut actions);
            }
            if !model.notices.is_empty() {
                notices(ctx, &model.notices);
            }
            match &model.thread {
                Some(thread) => thread_panel(ctx, thread, comment_text, &mut actions),
                None => comment_text.clear(),
            }
        });
        // egui has no clipboard of its own in the browser.
        #[cfg(target_arch = "wasm32")]
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                pb_platform::clipboard::set_text(text);
            }
        }
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
        .fill(if dark() {
            Color32::from_gray(0x2B)
        } else {
            Color32::WHITE
        })
        .corner_radius(10)
        .inner_margin(6)
        .stroke(match (dark(), high_contrast()) {
            (true, true) => Stroke::new(1.5, Color32::from_gray(0xC8)),
            (false, true) => Stroke::new(1.5, Color32::from_gray(0x40)),
            (true, false) => Stroke::new(1.0, Color32::from_gray(0x40)),
            (false, false) => Stroke::new(1.0, Color32::from_gray(225)),
        })
        .shadow(Shadow {
            offset: [0, 2],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(30),
        })
}

/// The browser keeps boards inside itself: they are listed in the boards window, and
/// downloaded to have them as files.
fn stored_boards_menu(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    ui.separator();
    if ui
        .button(tr("Download board"))
        .on_hover_text(tr("Save the board as a .pboard file on this computer"))
        .clicked()
    {
        actions.push(Action::DownloadBoard);
        ui.close();
    }
    if ui
        .button(tr("Boards in this browser"))
        .on_hover_text(tr("Open, rename, download or delete them"))
        .clicked()
    {
        actions.push(Action::ToggleBoards);
        ui.close();
    }
}

/// The start screen of the web version: the boards kept in the browser, to open, rename,
/// download or delete, and a new board or one from a file. `renaming` is the board being
/// renamed, with the name typed.
fn boards_window(
    ctx: &Context,
    model: &Model,
    renaming: &mut Option<(std::path::PathBuf, String)>,
    actions: &mut Vec<Action>,
) {
    let mut open = true;
    egui::Window::new(tr("Boards in this browser"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(tr("+ New board")).clicked() {
                    actions.push(Action::NewBoard);
                    actions.push(Action::ToggleBoards);
                }
                if ui
                    .button(tr("Open a file…"))
                    .on_hover_text(tr("A .pboard file from this computer"))
                    .clicked()
                {
                    actions.push(Action::Open);
                    actions.push(Action::ToggleBoards);
                }
            });
            ui.separator();
            if model.stored_boards.is_empty() {
                ui.weak(tr("No boards here yet"));
                return;
            }
            egui::ScrollArea::vertical()
                .max_height(ctx.content_rect().height() * 0.6)
                .show(ui, |ui| {
                    egui::Grid::new("boards")
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            for path in &model.stored_boards {
                                board_row(ui, model, path, renaming, actions);
                                ui.end_row();
                            }
                        });
                });
        });
    if !open {
        *renaming = None;
        actions.push(Action::ToggleBoards);
    }
}

/// The name field of board `path` being renamed.
fn rename_field_id(path: &std::path::Path) -> Id {
    Id::unique(("rename board", path))
}

/// One board of the boards window.
fn board_row(
    ui: &mut egui::Ui,
    model: &Model,
    path: &std::path::Path,
    renaming: &mut Option<(std::path::PathBuf, String)>,
    actions: &mut Vec<Action>,
) {
    let name = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let open = model.open_board.as_deref() == Some(path);
    if let Some((_, typed)) = renaming.as_mut().filter(|(p, _)| p == path) {
        let field = egui::TextEdit::singleline(typed)
            .id(rename_field_id(path))
            .desired_width(220.0);
        ui.add(field);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button(tr("OK")).clicked() || enter {
            actions.push(Action::RenameStored(path.to_owned(), typed.clone()));
            *renaming = None;
        } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            *renaming = None;
        }
        return;
    }
    let label = if open {
        RichText::new(&name).strong()
    } else {
        RichText::new(&name)
    };
    let title = ui
        .allocate_ui_with_layout(
            egui::vec2(220.0, 22.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(220.0);
                ui.add(egui::Label::new(label).sense(egui::Sense::click()))
            },
        )
        .inner;
    if open {
        title.on_hover_text(tr("Open now"));
    } else if title.on_hover_text(tr("Open")).clicked() {
        actions.push(Action::OpenFile(path.to_owned()));
        actions.push(Action::ToggleBoards);
    }
    ui.horizontal(|ui| {
        // Inter has no pencil or arrow signs: words instead.
        if ui.add(Button::new(tr("Rename")).small()).clicked() {
            // The field takes the keys at once, with the old name selected to type over.
            let id = rename_field_id(path);
            let mut state = egui::text_edit::TextEditState::default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(name.chars().count()),
                )));
            state.store(ui.ctx(), id);
            ui.memory_mut(|m| m.request_focus(id));
            ui.ctx().request_repaint();
            *renaming = Some((path.to_owned(), name.clone()));
        }
        if ui
            .add(Button::new(tr("Download")).small())
            .on_hover_text(tr("Save the board as a .pboard file on this computer"))
            .clicked()
        {
            actions.push(Action::DownloadStored(path.to_owned()));
        }
        let delete = ui
            .add_enabled(!open, Button::new(tr("Delete")).small())
            .on_hover_text(tr("Delete this board from the browser"))
            .on_disabled_hover_text(tr("It is open: open another board first"));
        if delete.clicked() {
            actions.push(Action::DeleteStored(path.to_owned()));
        }
    });
}

/// What is typed in the menus of the board menu.
struct Typed<'a> {
    version_name: &'a mut String,
    timer_text: &'a mut String,
}

/// The menu bar at the top left.
fn board_menu(ctx: &Context, model: &Model, typed: Typed, actions: &mut Vec<Action>) {
    let Typed {
        version_name,
        timer_text,
    } = typed;
    Area::new(Id::unique("board"))
        .fixed_pos(Pos2::new(MARGIN, MARGIN))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                // Too wide for the window: the buttons go on in more rows.
                ui.set_max_width(ctx.content_rect().width() - 2.0 * MARGIN - 16.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(4.0);
                    ui.strong(model.board);
                    if model.read_only {
                        ui.weak(tr("read-only"));
                    }
                    ui.separator();
                    ui.menu_button(tr("File"), |ui| {
                        let items = [
                            ("New board", "Ctrl+N", Action::NewBoard),
                            ("Open…", "Ctrl+O", Action::Open),
                            ("Save", "Ctrl+S", Action::Save),
                            ("Save as…", "Ctrl+Shift+S", Action::SaveAs),
                            ("Insert file…", "", Action::InsertImage),
                            ("Diagram from text…", "", Action::OpenDiagram),
                            ("Graph of a function…", "", Action::OpenGraph),
                            ("Export…", "Ctrl+Shift+E", Action::OpenExport),
                            ("Print…", "Ctrl+P", Action::Print),
                        ];
                        for (label, shortcut, action) in items {
                            // Inserting changes the board.
                            let on = !model.read_only
                                || !matches!(
                                    action,
                                    Action::InsertImage | Action::OpenDiagram | Action::OpenGraph
                                );
                            let button = Button::new(tr(label)).shortcut_text(shortcut);
                            if ui.add_enabled(on, button).clicked() {
                                actions.push(action);
                                ui.close();
                            }
                        }
                        if model.web {
                            stored_boards_menu(ui, actions);
                        }
                        ui.separator();
                        if ui
                            .add(Button::selectable(model.read_only, tr("Read-only")))
                            .on_hover_text(tr("Protect the board: only looking and commenting"))
                            .clicked()
                        {
                            actions.push(Action::ToggleReadOnly);
                            ui.close();
                        }
                    });
                    ui.menu_button(tr("Frames"), |ui| {
                        if model.frames.is_empty() {
                            ui.weak(tr("No frames yet: draw one with F"));
                        }
                        for (id, title) in &model.frames {
                            let title = if title.trim().is_empty() {
                                tr("Untitled frame")
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
                        RichText::new(tr("Filter"))
                    } else {
                        RichText::new(tr("Filter ●")).color(Color32::from_rgb(0x3B, 0x82, 0xF6))
                    };
                    ui.menu_button(label, |ui| filter_menu(ui, model, actions));
                    if ui
                        .add_enabled(model.can_present, Button::new(tr("Present")))
                        .on_hover_text(tr("Show the frames one by one (F5)"))
                        .on_disabled_hover_text(tr("Add frames to present them"))
                        .clicked()
                    {
                        actions.push(Action::Present);
                    }
                    ui.menu_button(tr("Plugins"), |ui| plugins_menu(ui, model, actions));
                    ui.menu_button(tr("Talktracks"), |ui| talktracks_menu(ui, model, actions));
                    menu_with_fields(ui, tr("History"), |ui| {
                        history_menu(ui, model, version_name, actions)
                    });
                    ui.menu_button(tr("Vote"), |ui| {
                        voting_menu(ui, model.voting, &mut *actions)
                    });
                    menu_with_fields(ui, tr("Timer"), |ui| {
                        timer_menu(ui, model.view_only, timer_text, actions)
                    });
                    if ui
                        .add_enabled(!model.read_only, Button::new(tr("Templates")))
                        .on_hover_text(tr("Start from a ready-made layout"))
                        .clicked()
                    {
                        actions.push(Action::ToggleTemplates);
                    }
                    let open = model.threads.iter().filter(|(_, _, r)| !r).count();
                    let label = if open > 0 {
                        trf("Comments ({})", &[&open])
                    } else {
                        tr("Comments").to_owned()
                    };
                    ui.menu_button(label, |ui| comments_menu(ui, model, actions));
                    if ui
                        .button(tr("Find"))
                        .on_hover_text(tr("Find on the board (Ctrl+F)"))
                        .clicked()
                    {
                        actions.push(Action::OpenSearch);
                    }
                    if ui
                        .button(tr("Commands"))
                        .on_hover_text(tr("Command palette: any command, frame or text (Ctrl+K)"))
                        .clicked()
                    {
                        actions.push(Action::TogglePalette);
                    }
                    if ui
                        .button("?")
                        .on_hover_text(tr("Keyboard shortcuts (F1)"))
                        .clicked()
                    {
                        actions.push(Action::ToggleShortcuts);
                    }
                    ui.separator();
                    let share = match &model.share_badge {
                        Some((text, online)) => {
                            let color = if *online {
                                Color32::from_rgb(0x16, 0xA3, 0x4A)
                            } else {
                                Color32::from_rgb(0xD9, 0x77, 0x06)
                            };
                            RichText::new(format!("● {text}")).color(color)
                        }
                        None => RichText::new(tr("Share")),
                    };
                    if ui
                        .button(share)
                        .on_hover_text(tr("Work on this board together through a server"))
                        .clicked()
                    {
                        actions.push(Action::ToggleShare);
                    }
                    hide_button(ui, "«", "Hide the menu", Panel::Menu, actions);
                });
            });
        });
}

/// What the sharing window shows.
/// What is typed in the sharing window.
#[derive(Default)]
struct ShareTexts {
    server: String,
    link: String,
    /// The name being typed, while it is.
    name: Option<String>,
    email: String,
    /// Someone to let in to the board, and as what.
    member: String,
    member_role: String,
}

pub struct ShareView {
    /// The name the user goes by.
    pub name: String,
    pub account: AccountView,
    /// Shared: how it is going, the links and who is there.
    pub shared: Option<SharedView>,
    /// The server address last used.
    pub server: String,
    /// A board is being made on the server.
    pub making: bool,
    /// What went wrong last.
    pub error: Option<String>,
}

/// The account on the server in use.
pub struct AccountView {
    /// The server; empty if none is known yet.
    pub server: String,
    /// Signed in with this email.
    pub email: Option<String>,
    /// Waiting for the link in the email to this address, which shows this code.
    pub signing_in: Option<(String, Option<String>)>,
    /// The account's boards: id, title, role.
    pub boards: Option<Vec<(String, String, String)>>,
    pub error: Option<String>,
}

pub struct SharedView {
    /// Everyone is asked to follow this view.
    pub leading: bool,
    pub status: String,
    pub online: bool,
    pub offline: bool,
    pub role: pb_core::Role,
    pub edit_link: Option<String>,
    pub comment_link: Option<String>,
    pub view_link: Option<String>,
    /// People let in by email, when this account owns the board.
    pub members: Option<Vec<(String, String)>>,
    /// The others on the board.
    pub people: Vec<PersonView>,
}

pub struct PersonView {
    pub client: pb_sync::ClientId,
    pub name: String,
    pub color: u32,
    /// This copy follows their view.
    pub followed: bool,
    /// They ask everyone to follow them.
    pub leading: bool,
}

fn rgb(color: u32) -> Color32 {
    Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8)
}

/// How a role reads in the sharing window.
fn role_label(role: &str) -> &'static str {
    match role {
        "owner" => tr("owner"),
        "edit" => tr("can edit"),
        "comment" => tr("can comment"),
        _ => tr("can look"),
    }
}

/// Signing in, the account's boards, signing out.
fn account_section(
    ui: &mut egui::Ui,
    account: &AccountView,
    texts: &mut ShareTexts,
    actions: &mut Vec<Action>,
) {
    let server = if account.server.is_empty() {
        texts.server.trim().to_owned()
    } else {
        account.server.clone()
    };
    if server.is_empty() {
        return;
    }
    if let Some(email) = &account.email {
        ui.horizontal_wrapped(|ui| {
            ui.label(trf("Signed in as {}", &[email]));
            if ui.small_button(tr("Sign out")).clicked() {
                actions.push(Action::SignOut);
            }
            if ui
                .small_button(tr("Delete account…"))
                .on_hover_text(tr(
                    "Deletes the account and the boards it owns from the server",
                ))
                .clicked()
            {
                actions.push(Action::DeleteAccount);
            }
        });
        if let Some(boards) = &account.boards {
            egui::CollapsingHeader::new(trf("My boards ({})", &[&boards.len()]))
                .id_salt("my boards")
                .show(ui, |ui| {
                    if boards.is_empty() {
                        ui.weak(tr("None yet: boards you share while signed in, and those you are let in to, show here."));
                    }
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        for (board, title, role) in boards {
                            ui.horizontal(|ui| {
                                let title = if title.is_empty() { board } else { title };
                                if ui.link(title).on_hover_text(tr("Open this board")).clicked() {
                                    actions.push(Action::OpenMyBoard(board.clone()));
                                }
                                ui.weak(role_label(role));
                            });
                        }
                    });
                    if ui.small_button(tr("Refresh")).clicked() {
                        actions.push(Action::RefreshBoards);
                    }
                });
        }
    } else if let Some((email, code)) = &account.signing_in {
        ui.label(trf(
            "A link to sign in was sent to {}. Open it in the email.",
            &[email],
        ));
        match code {
            Some(code) => {
                ui.horizontal(|ui| {
                    ui.label(tr("The page it opens must show the code"));
                    ui.strong(code);
                });
            }
            None => {
                ui.weak(tr("Sending the email…"));
            }
        }
        if ui.button(tr("Cancel")).clicked() {
            actions.push(Action::CancelSignIn);
        }
    } else {
        ui.label(tr(
            "Sign in to see your boards in one list and let people in by email.",
        ));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut texts.email)
                    .hint_text("name@example.com")
                    .desired_width(240.0),
            );
            let typed = texts.email.contains('@');
            if ui.add_enabled(typed, Button::new(tr("Sign in"))).clicked() {
                actions.push(Action::SignIn(
                    server.clone(),
                    texts.email.trim().to_owned(),
                ));
            }
        });
    }
    if let Some(error) = &account.error {
        ui.colored_label(Color32::from_rgb(0xDC, 0x26, 0x26), error);
    }
    ui.separator();
}

/// The owner's list of people let in by email, and a row to let someone in.
fn members_section(
    ui: &mut egui::Ui,
    members: &[(String, String)],
    texts: &mut ShareTexts,
    actions: &mut Vec<Action>,
) {
    const ROLES: [&str; 3] = ["edit", "comment", "view"];
    ui.add_space(6.0);
    ui.label(tr("People let in by email")).on_hover_text(tr(
        "They open the board from My boards after signing in with that address",
    ));
    for (email, role) in members {
        ui.horizontal(|ui| {
            ui.label(email);
            if role == "owner" {
                ui.weak(role_label(role));
                return;
            }
            let mut chosen = role.clone();
            egui::ComboBox::from_id_salt(("member role", email))
                .selected_text(role_label(&chosen))
                .show_ui(ui, |ui| {
                    for r in ROLES {
                        ui.selectable_value(&mut chosen, r.to_owned(), role_label(r));
                    }
                });
            if chosen != *role {
                actions.push(Action::SetMember(email.clone(), chosen));
            }
            if ui.small_button("×").on_hover_text(tr("Remove")).clicked() {
                actions.push(Action::SetMember(email.clone(), "none".into()));
            }
        });
    }
    ui.horizontal(|ui| {
        if texts.member_role.is_empty() {
            texts.member_role = "edit".into();
        }
        ui.add(
            egui::TextEdit::singleline(&mut texts.member)
                .hint_text("name@example.com")
                .desired_width(200.0),
        );
        egui::ComboBox::from_id_salt("new member role")
            .selected_text(role_label(&texts.member_role))
            .show_ui(ui, |ui| {
                for r in ROLES {
                    ui.selectable_value(&mut texts.member_role, r.to_owned(), role_label(r));
                }
            });
        let typed = texts.member.contains('@');
        if ui
            .add_enabled(typed, Button::new(tr("Let in")))
            .on_hover_text(tr("They get an email about it"))
            .clicked()
        {
            actions.push(Action::SetMember(
                std::mem::take(&mut texts.member).trim().to_owned(),
                texts.member_role.clone(),
            ));
        }
    });
}

fn share_window(
    ctx: &Context,
    view: &ShareView,
    texts: &mut ShareTexts,
    actions: &mut Vec<Action>,
) {
    let mut open = true;
    egui::Window::new(tr("Share"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.horizontal(|ui| {
                ui.label(tr("Your name"));
                let name = texts.name.get_or_insert_with(|| view.name.clone());
                let field = ui.add(egui::TextEdit::singleline(name).desired_width(200.0))
                    .on_hover_text(tr("Shown to others next to your pointer, comments and votes"));
                if field.lost_focus() {
                    actions.push(Action::SetName(name.clone()));
                    texts.name = None;
                }
            });
            ui.separator();
            account_section(ui, &view.account, texts, actions);
            if let Some(shared) = &view.shared {
                ui.horizontal(|ui| {
                    let color = if shared.online {
                        Color32::from_rgb(0x16, 0xA3, 0x4A)
                    } else {
                        Color32::from_rgb(0xD9, 0x77, 0x06)
                    };
                    ui.colored_label(color, "●");
                    ui.label(&shared.status);
                    if shared.offline && ui.button(tr("Try again")).clicked() {
                        actions.push(Action::RetrySharing);
                    }
                });
                if shared.online {
                    match shared.role {
                        pb_core::Role::View => {
                            ui.weak(tr("You can look at this board but not change it."));
                        }
                        pb_core::Role::Comment => {
                            ui.weak(tr("You can comment, react and vote here, but not change the board."));
                        }
                        pb_core::Role::Edit => {}
                    }
                }
                if !shared.people.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr("Here now:"));
                        for person in &shared.people {
                            let mut name = RichText::new(&person.name).color(rgb(person.color));
                            if person.followed {
                                name = name.underline();
                            }
                            let hint = if person.followed {
                                tr("Stop following")
                            } else {
                                tr("Follow: see what they see")
                            };
                            if ui
                                .add(Button::new(name).frame(false))
                                .on_hover_text(hint)
                                .clicked()
                            {
                                actions.push(Action::Follow(person.client));
                            }
                            if person.leading {
                                ui.weak(tr("(presenting)"));
                            }
                        }
                    });
                }
                if shared.online {
                    let label = if shared.leading {
                        tr("Stop leading")
                    } else {
                        tr("Bring everyone to me")
                    };
                    if ui
                        .button(label)
                        .on_hover_text(tr("Everyone on the board follows your view until they move"))
                        .clicked()
                    {
                        actions.push(Action::ToggleLead);
                    }
                }
                ui.add_space(6.0);
                let owner = shared.members.is_some();
                // The owner sees every link, and can replace each: the old one stops working.
                let link_row = |ui: &mut egui::Ui, label: &str, hint: &str, link: Option<&String>, role: pb_core::Role, actions: &mut Vec<Action>| {
                    if link.is_none() && !owner {
                        return;
                    }
                    ui.label(label).on_hover_text(hint);
                    ui.horizontal(|ui| {
                        let mut shown = link.cloned().unwrap_or_default();
                        ui.add(egui::TextEdit::singleline(&mut shown).desired_width(300.0));
                        if ui.add_enabled(link.is_some(), Button::new(tr("Copy"))).clicked()
                            && let Some(link) = link
                        {
                            actions.push(Action::CopyText(link.clone()));
                        }
                        if owner
                            && ui
                                .button(tr("New link"))
                                .on_hover_text(tr("Makes a new link: the old one stops working"))
                                .clicked()
                        {
                            actions.push(Action::RenewLink(role));
                        }
                    });
                };
                link_row(ui, tr("Link to edit"), tr("Whoever has it can change the board"), shared.edit_link.as_ref(), pb_core::Role::Edit, actions);
                link_row(ui, tr("Link to comment"), tr("Whoever has it can comment, react and vote, but not change the board"), shared.comment_link.as_ref(), pb_core::Role::Comment, actions);
                link_row(ui, tr("Link to look"), tr("Whoever has it can only look"), shared.view_link.as_ref(), pb_core::Role::View, actions);
                if let Some(members) = &shared.members {
                    members_section(ui, members, texts, actions);
                }
                ui.add_space(6.0);
                if ui
                    .button(tr("Stop sharing"))
                    .on_hover_text(tr("Keep working on this copy alone"))
                    .clicked()
                {
                    actions.push(Action::StopSharing);
                }
                if shared.members.is_some()
                    && ui
                        .button(tr("Delete from the server…"))
                        .on_hover_text(tr("Everyone loses the board; your copy here stays"))
                        .clicked()
                {
                    actions.push(Action::DeleteSharedBoard);
                }
            } else {
                ui.label(tr("Put this board on a server to work on it together: everyone with the link sees changes as they are made."));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(tr("Server"));
                    if texts.server.is_empty() {
                        texts.server.clone_from(&view.server);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut texts.server)
                            .hint_text("boards.example.com")
                            .desired_width(260.0),
                    );
                });
                let can_make = !view.making && !texts.server.trim().is_empty();
                let label = if view.making { tr("Making the board…") } else { tr("Share this board") };
                if ui.add_enabled(can_make, Button::new(label)).clicked() {
                    actions.push(Action::CreateShared(texts.server.trim().to_owned()));
                }
                ui.separator();
                ui.label(tr("Or open a board someone shared with you:"));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut texts.link)
                            .hint_text("https://…/board/…?key=…")
                            .desired_width(300.0),
                    );
                    if ui
                        .add_enabled(!texts.link.trim().is_empty(), Button::new(tr("Open")))
                        .clicked()
                    {
                        actions.push(Action::JoinShared(texts.link.trim().to_owned()));
                    }
                });
            }
            if let Some(error) = &view.error {
                ui.add_space(4.0);
                ui.colored_label(Color32::from_rgb(0xDC, 0x26, 0x26), error);
            }
        });
    if !open {
        actions.push(Action::ToggleShare);
    }
}

/// While following someone or leading: a colored edge around the window and a banner.
fn follow_banner(ctx: &Context, banner: &(String, u32, bool), actions: &mut Vec<Action>) {
    let (text, color, leading) = banner;
    let color = rgb(*color);
    let painter = ctx.layer_painter(egui::LayerId::new(
        Order::Foreground,
        Id::unique("following edge"),
    ));
    painter.rect_stroke(
        ctx.content_rect().shrink(2.0),
        0.0,
        egui::Stroke::new(4.0, color),
        egui::StrokeKind::Inside,
    );
    Area::new(Id::unique("following"))
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(color)
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(text).color(Color32::WHITE).strong());
                        let stop = if *leading {
                            tr("Stop leading")
                        } else {
                            tr("Stop following")
                        };
                        if ui.button(stop).clicked() {
                            actions.push(if *leading {
                                Action::ToggleLead
                            } else {
                                Action::StopFollowing
                            });
                        }
                    });
                });
        });
}

/// Other people's pointers with their names, and outlines of what they selected.
fn others(ctx: &Context, model: &Model) {
    if model.cursors.is_empty() && model.their_selections.is_empty() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(Order::Middle, Id::unique("others")));
    for (rect, color) in &model.their_selections {
        let r = to_points(*rect);
        let rect = egui::Rect::from_min_max(
            Pos2::new(r.x0 as f32, r.y0 as f32),
            Pos2::new(r.x1 as f32, r.y1 as f32),
        );
        painter.rect_stroke(
            rect.expand(3.0),
            2.0,
            egui::Stroke::new(2.0, rgb(*color)),
            egui::StrokeKind::Outside,
        );
    }
    for (at, name, color) in &model.cursors {
        let p = to_points(kurbo::Rect::from_origin_size(*at, (0.0, 0.0)));
        let at = Pos2::new(p.x0 as f32, p.y0 as f32);
        let color = rgb(*color);
        // An arrow like the system's pointer.
        let arrow = vec![
            at,
            at + egui::vec2(0.0, 16.0),
            at + egui::vec2(4.5, 12.0),
            at + egui::vec2(11.0, 12.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            arrow,
            color,
            egui::Stroke::new(1.0, Color32::WHITE),
        ));
        let font = egui::FontId::proportional(12.0);
        let galley = painter.layout_no_wrap(name.clone(), font, Color32::WHITE);
        let label = egui::Rect::from_min_size(
            at + egui::vec2(10.0, 16.0),
            galley.size() + egui::vec2(10.0, 4.0),
        );
        painter.rect_filled(label, 4.0, color);
        painter.galley(label.min + egui::vec2(5.0, 2.0), galley, Color32::WHITE);
    }
}

/// The toolbar of a read-only board: selecting and commenting.
/// A book's page buttons: previous, the page number (typed or dragged to jump), next.
fn book_controls(ui: &mut egui::Ui, page: u32, pages: u32, actions: &mut Vec<Action>) {
    let previous = ui
        .add_enabled(page > 0, Button::new("◀"))
        .on_hover_text(tr("Previous page (PageUp)"));
    if previous.clicked() {
        actions.push(Action::TurnPage(page - 1));
    }
    let mut shown = page + 1;
    let number = ui
        .add(
            egui::DragValue::new(&mut shown)
                .range(1..=pages.max(1))
                .speed(0.2),
        )
        .on_hover_text(tr("Page: type a number or drag"));
    if number.changed() && shown != page + 1 {
        actions.push(Action::TurnPage(shown.saturating_sub(1)));
    }
    ui.label(trf("of {}", &[&pages]));
    let next = ui
        .add_enabled(page + 1 < pages, Button::new("▶"))
        .on_hover_text(tr("Next page (PageDown)"));
    if next.clicked() {
        actions.push(Action::TurnPage(page + 1));
    }
}

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

/// Room the toolbar leaves at the top and bottom of the window for the board menu and
/// the view controls.
const TOOLBAR_CLEARANCE: f32 = 64.0;

fn toolbar(ctx: &Context, model: &Model, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    let shapes_button = Area::new(Id::unique("tools"))
        .fixed_pos(Pos2::new(MARGIN, screen.center().y))
        .pivot(Align2::LEFT_CENTER)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            // Between the board menu and the view controls; it scrolls when it does not fit
            // (a low window, a large UI).
            let max_height = (screen.height() - 2.0 * TOOLBAR_CLEARANCE).max(BUTTON * 3.0);
            panel()
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(max_height)
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
                            let shapes =
                                icon_button(ui, shape_active, "Shapes (S, R, O)", |p, r, c| {
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
                            let structure_active =
                                STRUCTURES.iter().any(|(t, ..)| *t == model.tool);
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
                            let image =
                                icon_button(ui, false, "Image (or drop, paste)", paint_image);
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
                            ui.separator();
                            ui.vertical_centered(|ui| {
                                hide_button(ui, "«", "Hide the tools", Panel::Tools, actions);
                            });
                            (shapes.rect, ink_button, structures.rect)
                        })
                        .inner
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

    if model.tool.draws_ink() && model.ink_panel {
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
                        ui.weak(tr(set));
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
        .selectable_label(c.start_cap == Cap::Arrow, tr("Start arrow"))
        .on_hover_text(tr("Arrowhead at the start"));
    if start.clicked() {
        actions.push(Action::SetConnector(ConnectorChange {
            start_cap: Some(toggled(c.start_cap)),
            ..ConnectorChange::default()
        }));
    }
    let end = ui
        .selectable_label(c.end_cap == Cap::Arrow, tr("End arrow"))
        .on_hover_text(tr("Arrowhead at the end"));
    if end.clicked() {
        actions.push(Action::SetConnector(ConnectorChange {
            end_cap: Some(toggled(c.end_cap)),
            ..ConnectorChange::default()
        }));
    }
    if !c.bends.is_empty()
        && ui
            .button(tr("Reset path"))
            .on_hover_text(tr("Drop the bends set by hand"))
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
                // Too wide for the window: the buttons go on in more rows.
                ui.set_max_width(ctx.content_rect().width() - 2.0 * MARGIN - 16.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(tr(model.grid.name()))
                        .on_hover_text(tr("Grid style (G)"))
                        .clicked()
                    {
                        actions.push(Action::NextGrid);
                    }
                    if ui
                        .selectable_label(model.snap, tr("Snap to grid"))
                        .on_hover_text(tr("Shift+G"))
                        .clicked()
                    {
                        actions.push(Action::ToggleSnap);
                    }
                    ui.menu_button(tr("View"), |ui| {
                        ui.label(tr("Interface size"));
                        ui.horizontal(|ui| {
                            for scale in [0.75, 1.0, 1.25, 1.5, 1.75, 2.0] {
                                let on = (model.ui_scale - scale).abs() < 0.01;
                                let label = format!("{:.0}%", scale * 100.0);
                                if ui.selectable_label(on, label).clicked() {
                                    actions.push(Action::SetUiScale(scale));
                                }
                            }
                        });
                        ui.weak(tr("Ctrl+Alt+= / Ctrl+Alt+- / Ctrl+Alt+0"));
                        ui.separator();
                        if ui
                            .add(Button::selectable(model.high_contrast, tr("High contrast")))
                            .on_hover_text(tr("Black or white text and clear outlines"))
                            .clicked()
                        {
                            actions.push(Action::ToggleHighContrast);
                        }
                        ui.separator();
                        let fullscreen = Button::selectable(model.fullscreen, tr("Full screen"))
                            .shortcut_text("F11");
                        if ui.add(fullscreen).clicked() {
                            actions.push(Action::ToggleFullscreen);
                            ui.close();
                        }
                        let clean = Button::new(tr("Clean board")).shortcut_text("Ctrl+\\");
                        if ui
                            .add(clean)
                            .on_hover_text(tr(
                                "Hide all panels; a button in the corner brings them back",
                            ))
                            .clicked()
                        {
                            actions.push(Action::ToggleClean);
                            ui.close();
                        }
                        ui.separator();
                        ui.label(tr("Board color"));
                        if ui
                            .add(Button::selectable(model.background.is_none(), tr("Usual")))
                            .clicked()
                        {
                            actions.push(Action::SetBackground(None));
                        }
                        let current = model.background.unwrap_or(Rgba(0));
                        if let Some(color) = color_grid(ui, &BOARD_COLORS, current, 4) {
                            actions.push(Action::SetBackground(Some(color)));
                        }
                        ui.separator();
                        ui.label(tr("Panels"));
                        let panels = [
                            (Panel::Menu, "Menu", model.panels.menu),
                            (Panel::Tools, "Tools", model.panels.tools),
                            (Panel::View, "These controls", model.panels.view),
                        ];
                        for (panel, label, shown) in panels {
                            if ui.add(Button::selectable(shown, tr(label))).clicked() {
                                actions.push(Action::TogglePanel(panel));
                                ui.close();
                            }
                        }
                    });
                    let lang = crate::i18n::lang().code().to_uppercase();
                    if ui
                        .button(lang)
                        .on_hover_text(tr("Language: English / Russian"))
                        .clicked()
                    {
                        actions.push(Action::ToggleLanguage);
                    }
                    if ui
                        .selectable_label(model.dark, tr("Dark"))
                        .on_hover_text(tr("Dark theme"))
                        .clicked()
                    {
                        actions.push(Action::ToggleTheme);
                    }
                    if ui
                        .selectable_label(model.minimap.is_some(), tr("Map"))
                        .on_hover_text(tr("Minimap"))
                        .clicked()
                    {
                        actions.push(Action::ToggleMinimap);
                    }
                    if ui
                        .selectable_label(model.layers.is_some(), tr("Layers"))
                        .on_hover_text(tr("Layers of the board"))
                        .clicked()
                    {
                        actions.push(Action::ToggleLayers);
                    }
                    let (hidden, shown) = model.hidden;
                    if (hidden > 0 || shown)
                        && ui
                            .selectable_label(shown, trf("Hidden ({})", &[&hidden]))
                            .on_hover_text(tr("Show hidden objects faded, to pick and show them"))
                            .clicked()
                    {
                        actions.push(Action::ShowHidden);
                    }
                    ui.separator();
                    if ui
                        .button("−")
                        .on_hover_text(tr("Zoom out (Ctrl+−)"))
                        .clicked()
                    {
                        actions.push(Action::ZoomOut);
                    }
                    let zoom = format!("{:.0}%", model.zoom * 100.0);
                    if ui
                        .button(zoom)
                        .on_hover_text(tr("Reset zoom (Ctrl+0)"))
                        .clicked()
                    {
                        actions.push(Action::ZoomReset);
                    }
                    if ui
                        .button("+")
                        .on_hover_text(tr("Zoom in (Ctrl+=)"))
                        .clicked()
                    {
                        actions.push(Action::ZoomIn);
                    }
                    hide_button(ui, "»", "Hide these controls", Panel::View, actions);
                });
            });
        })
        .response
        .rect
}

/// A small button that puts a panel away.
fn hide_button(
    ui: &mut egui::Ui,
    arrow: &str,
    tooltip: &'static str,
    panel: Panel,
    actions: &mut Vec<Action>,
) {
    let button = Button::new(RichText::new(arrow).weak()).frame(false);
    if ui.add(button).on_hover_text(tr(tooltip)).clicked() {
        actions.push(Action::TogglePanel(panel));
    }
}

/// Where panels are put away, small buttons that bring them back; in clean mode, one
/// button in the corner brings all of them.
fn panel_handles(ctx: &Context, panels: Panels, focus: bool, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    let corner = screen.max - Vec2::splat(MARGIN);
    let left = Pos2::new(MARGIN, screen.center().y);
    let top = Pos2::new(MARGIN, MARGIN);
    let shows_menu = panels.menu || focus;
    let handles = [
        (
            panels.clean,
            corner,
            Align2::RIGHT_BOTTOM,
            "«",
            format!("{} (Ctrl+\\)", tr("Show the panels")),
            Action::ToggleClean,
        ),
        (
            !panels.clean && !shows_menu,
            top,
            Align2::LEFT_TOP,
            "»",
            tr("Show the menu").to_owned(),
            Action::TogglePanel(Panel::Menu),
        ),
        (
            !panels.clean && !panels.tools,
            left,
            Align2::LEFT_CENTER,
            "»",
            tr("Show the tools").to_owned(),
            Action::TogglePanel(Panel::Tools),
        ),
        (
            !panels.clean && !panels.view,
            corner,
            Align2::RIGHT_BOTTOM,
            "«",
            tr("Show the view controls").to_owned(),
            Action::TogglePanel(Panel::View),
        ),
    ];
    for (i, (shown, pos, pivot, arrow, tooltip, action)) in handles.into_iter().enumerate() {
        if !shown {
            continue;
        }
        Area::new(Id::unique(("panel handle", i)))
            .fixed_pos(pos)
            .pivot(pivot)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                panel().show(ui, |ui| {
                    if ui.button(arrow).on_hover_text(tooltip).clicked() {
                        actions.push(action);
                    }
                });
            });
    }
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
                let background = if dark() { 0x22 } else { 246 };
                painter.rect_filled(rect, 4, Color32::from_gray(background));
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
    let r = to_points(selection.screen);
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
        .constrain(true)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                // Too wide for the window: the buttons go on in more rows.
                ui.set_max_width(ctx.content_rect().width() - 2.0 * MARGIN - 16.0);
                ui.horizontal_wrapped(|ui| {
                    // Pages turn on read-only boards and locked books too.
                    if let Some((page, pages)) = selection.book {
                        book_controls(ui, page, pages, actions);
                        ui.separator();
                    }
                    if selection.read_only {
                        if ui.button(tr("Copy")).on_hover_text(tr("Ctrl+C")).clicked() {
                            actions.push(Action::Copy);
                        }
                        return;
                    }
                    if selection.locked {
                        if ui.button(tr("Unlock")).on_hover_text(tr("Ctrl+Shift+L")).clicked() {
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
                        let label = RichText::new(tr("Aa")).color(to_color32(text.color)).strong();
                        let button = ui.button(label).on_hover_text(tr("Text color, size and alignment"));
                        toggle(popup, Popup::Text, button.clicked());
                    }
                    if let Some(busy) = selection.handwriting {
                        ui.add_enabled_ui(!busy, |ui| {
                            let label = if busy { tr("Recognizing…") } else { tr("To text") };
                            if ui
                                .button(label)
                                .on_hover_text(tr("Turn the handwriting into typed text"))
                                .clicked()
                            {
                                actions.push(Action::ToText);
                            }
                            if ui
                                .button(tr("Calculate"))
                                .on_hover_text(tr("Solve a written expression such as 12 + 7 ="))
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
                                .add_enabled(enabled, Button::new(tr(label)))
                                .on_hover_text(tr(tip))
                                .clicked()
                            {
                                actions.push(action);
                            }
                        }
                        ui.separator();
                    }
                    if let Some((children, collapsed)) = selection.mind_node {
                        if ui.button(tr("+ Child")).on_hover_text(tr("Add a subtopic (Tab)")).clicked() {
                            actions.push(Action::AddNode(true));
                        }
                        if ui.button(tr("+ Sibling")).on_hover_text(tr("Add a topic after this one (Enter)")).clicked() {
                            actions.push(Action::AddNode(false));
                        }
                        if children {
                            let label = if collapsed { tr("Expand") } else { tr("Collapse") };
                            if ui.button(label).on_hover_text(tr("Show or hide the subtopics")).clicked() {
                                actions.push(Action::ToggleNode);
                            }
                        }
                        if ui.button(tr("Delete topic")).on_hover_text(tr("Delete it with its subtopics (Delete)")).clicked() {
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
                                .add_enabled(enabled, Button::new(tr(label)))
                                .on_hover_text(tr(tip))
                                .clicked()
                            {
                                actions.push(action);
                            }
                        }
                        ui.separator();
                    }
                    if selection.is_embed
                        && ui
                            .button(tr("Update preview"))
                            .on_hover_text(tr("Fetch the page's title and description again"))
                            .clicked()
                    {
                        actions.push(Action::FetchPreview);
                    }
                    if selection.can_crop
                        && ui
                            .button(tr("Crop"))
                            .on_hover_text(tr("Double click or Enter; Enter or a click outside to finish, Esc to cancel"))
                            .clicked()
                    {
                        actions.push(Action::Crop);
                    }
                    if selection.link.is_some() {
                        let link = ui.button(tr("Link")).on_hover_text(tr("Link to a page or another object"));
                        toggle(popup, Popup::Link, link.clicked());
                    }
                    if selection.my_reactions.is_some() {
                        let react = ui.button(tr("React")).on_hover_text(tr("Add a reaction"));
                        toggle(popup, Popup::React, react.clicked());
                    }
                    let tags = ui.button(tr("Tags")).on_hover_text(tr("Tags on the selection"));
                    toggle(popup, Popup::Tags, tags.clicked());
                    let opacity = format!("{:.0}%", style.opacity * 100.0);
                    let opacity = ui.button(opacity).on_hover_text(tr("Opacity"));
                    toggle(popup, Popup::Opacity, opacity.clicked());
                    ui.separator();
                    if selection.several {
                        let arrange = ui.button(tr("Arrange")).on_hover_text(tr("Lay out in a grid or in groups"));
                        toggle(popup, Popup::Arrange, arrange.clicked());
                    }
                    if selection.several && ui.button(tr("Group")).on_hover_text(tr("Ctrl+G")).clicked() {
                        actions.push(Action::Group);
                    }
                    if selection.has_group
                        && ui.button(tr("Ungroup")).on_hover_text(tr("Ctrl+Shift+G")).clicked()
                    {
                        actions.push(Action::Ungroup);
                    }
                    if ui.button(tr("Lock")).on_hover_text(tr("Ctrl+Shift+L")).clicked() {
                        actions.push(Action::ToggleLock);
                    }
                    if !selection.layers.is_empty() {
                        ui.menu_button(tr("Layer"), |ui| {
                            for (id, name) in &selection.layers {
                                let here = selection.layer.as_ref() == Some(id);
                                if ui.add(Button::selectable(here, name)).clicked() {
                                    actions.push(Action::MoveToLayer(id.clone()));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text(tr("Move to another layer"));
                    }
                    let hide = if selection.hidden { tr("Show") } else { tr("Hide") };
                    if ui.button(hide).on_hover_text(tr("Ctrl+Shift+H")).clicked() {
                        actions.push(Action::ToggleHidden);
                    }
                    ui.separator();
                    if ui.button(tr("Front")).on_hover_text(tr("Ctrl+Shift+]")).clicked() {
                        actions.push(Action::Reorder(ZOrder::Front));
                    }
                    if ui.button(tr("Back")).on_hover_text(tr("Ctrl+Shift+[")).clicked() {
                        actions.push(Action::Reorder(ZOrder::Back));
                    }
                    ui.separator();
                    if ui.button(tr("Duplicate")).on_hover_text(tr("Ctrl+D")).clicked() {
                        actions.push(Action::Duplicate);
                    }
                    if ui.button(tr("Delete")).on_hover_text(tr("Delete")).clicked() {
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
                            .selectable_label(text.auto, tr("Auto"))
                            .on_hover_text(tr("As large as fits the shape"));
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
                        let response = ui.add(slider).on_hover_text(tr(
                            "Font size; text that does not fit the shape shrinks",
                        ));
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
                            if ui.selectable_label(text.align == align, tr(name)).clicked() {
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
                            if ui
                                .selectable_label(text.valign == valign, tr(name))
                                .clicked()
                            {
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
                            if ui.button(tr("Open")).clicked() {
                                actions.push(Action::FollowLink);
                            }
                            if ui.button(tr("Remove")).clicked() {
                                actions.push(Action::RemoveLink);
                            }
                        });
                        ui.separator();
                    }
                    ui.horizontal(|ui| {
                        let field = ui.add(
                            egui::TextEdit::singleline(link_text)
                                .hint_text(tr("Web address or copied object link"))
                                .desired_width(220.0),
                        );
                        let valid = pb_core::Link::from_input(link_text).is_some();
                        let entered =
                            field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let set = ui.add_enabled(valid, Button::new(tr("Set")));
                        if valid && (entered || set.clicked()) {
                            actions.push(Action::SetLink(std::mem::take(link_text)));
                        }
                    });
                    if !link.frames.is_empty() {
                        ui.weak(tr("Link to a frame:"));
                        for (id, title) in &link.frames {
                            let title = if title.trim().is_empty() {
                                tr("Untitled frame")
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
                            tr("Click an object…")
                        } else {
                            tr("Pick on board")
                        };
                        if ui
                            .add_enabled(!link.picking, Button::new(label))
                            .on_hover_text(tr("Then click the object to link to; Esc cancels"))
                            .clicked()
                        {
                            actions.push(Action::PickLinkTarget);
                            *popup = None;
                        }
                        if ui
                            .button(tr("Copy link"))
                            .on_hover_text(tr(
                                "Copy a link to this object, to paste into another one's Link",
                            ))
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
                            if ui.button(tr(label)).on_hover_text(tr(tip)).clicked() {
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
                                .hint_text(tr("New tag"))
                                .desired_width(140.0),
                        );
                        let entered =
                            field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let add =
                            ui.add_enabled(!new_tag.trim().is_empty(), Button::new(tr("Add")));
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
fn context_menu(
    ctx: &Context,
    info: ContextMenuInfo,
    plugin_commands: &[PluginCommandView],
    actions: &mut Vec<Action>,
) {
    let area = Area::new(Id::unique("context menu"))
        .fixed_pos(Pos2::new(info.pos.0, info.pos.1) / ui_scale())
        .order(Order::Foreground)
        .constrain(true)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.set_min_width(180.0);
                // Plugins' commands for the selection come first.
                let mut offered = false;
                for c in plugin_commands
                    .iter()
                    .filter(|c| c.selection && info.selection && !info.read_only)
                {
                    let button = Button::new(&c.title).frame(false);
                    if ui.add(button).on_hover_text(&c.plugin_name).clicked() {
                        actions.push(Action::RunPluginCommand(c.plugin.clone(), c.id.clone()));
                        actions.push(Action::CloseContextMenu);
                    }
                    offered = true;
                }
                if offered {
                    ui.separator();
                }
                let mut item = |ui: &mut egui::Ui,
                                label: &'static str,
                                keys: &str,
                                action: Action,
                                on: bool| {
                    let button = Button::new(tr(label)).shortcut_text(keys).frame(false);
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
        ui.weak(tr("No comments yet: use the comment tool (C)"));
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
        .fixed_pos(Pos2::new(thread.pos.0, thread.pos.1) / ui_scale() + Vec2::new(14.0, -10.0))
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
                        if ui
                            .small_button("×")
                            .on_hover_text(tr("Close (Esc)"))
                            .clicked()
                        {
                            actions.push(Action::CloseThread);
                        }
                        if !thread.is_new {
                            if ui
                                .small_button(tr("Delete"))
                                .on_hover_text(tr("Delete the thread"))
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
                            if ui.small_button(tr(label)).on_hover_text(tr(tip)).clicked() {
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
                                            if ui.small_button(tr("Delete")).clicked() {
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
                        .hint_text(tr("Write a comment… @name mentions someone"))
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
                    let label = if thread.is_new {
                        tr("Post")
                    } else {
                        tr("Reply")
                    };
                    let button = ui
                        .add_enabled(!text.trim().is_empty(), Button::new(label))
                        .on_hover_text(tr("Ctrl+Enter"));
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
        ui.weak(tr("Votes per person:"));
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
            ui.label(trf("{} of {} votes left", &[&v.left, &v.limit]));
            if ui.button(tr("End voting")).clicked() {
                actions.push(Action::EndVoting);
                ui.close();
            }
        }
        Some(_) => {
            if ui.button(tr("Results")).clicked() {
                actions.push(Action::ToggleResults);
                ui.close();
            }
            if ui.button(tr("Clear votes")).clicked() {
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
                    ui.strong(tr("Voting"));
                    ui.label(trf(
                        "{} of {} votes left · click an object to vote, Shift+click to take one back",
                        &[&voting.left, &voting.limit],
                    ));
                    if ui.button(tr("End voting")).clicked() {
                        actions.push(Action::EndVoting);
                    }
                });
            });
        });
}

/// The voting results: objects by votes, each going to its object.
fn results_window(ctx: &Context, results: &[(ObjectId, u32, String)], actions: &mut Vec<Action>) {
    let mut open = true;
    let shown = egui::Window::new(tr("Voting results"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_TOP, egui::vec2(-MARGIN, 64.0))
        .show(ctx, |ui| {
            if results.is_empty() {
                ui.weak(tr("No votes yet"));
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

/// The longest countdown that can be set, in milliseconds: a day.
const TIMER_MAX: u64 = 24 * 3_600_000;

/// A countdown's length typed by someone: minutes (`7`, `1.5`), or minutes and seconds
/// (`2:30`), or hours, minutes and seconds (`1:00:00`). In milliseconds.
pub fn parse_timer(text: &str) -> Option<u64> {
    let text = text.trim().replace(',', ".");
    let ms = if text.contains(':') {
        let parts: Vec<u64> = text
            .split(':')
            .map(|p| p.trim().parse().ok())
            .collect::<Option<_>>()?;
        let (h, m, s) = match parts[..] {
            [m, s] if s < 60 => (0, m, s),
            [h, m, s] if m < 60 && s < 60 => (h, m, s),
            _ => return None,
        };
        ((h * 60 + m) * 60 + s).checked_mul(1000)?
    } else {
        let minutes: f64 = text.parse().ok()?;
        if !minutes.is_finite() || minutes < 0.0 {
            return None;
        }
        (minutes * 60_000.0).round().min(u64::MAX as f64) as u64
    };
    (1000..=TIMER_MAX).contains(&ms).then_some(ms)
}

/// A menu with fields to type in: clicking inside it (into a field, say) keeps it open, so
/// its buttons close it themselves.
fn menu_with_fields(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    let config = egui::containers::menu::MenuConfig::new()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    egui::containers::menu::MenuButton::new(label)
        .config(config)
        .ui(ui, add);
}

/// Countdowns of a few usual lengths, and one of the length typed in `text`.
fn timer_menu(ui: &mut egui::Ui, view_only: bool, text: &mut String, actions: &mut Vec<Action>) {
    ui.weak(tr("Everyone on the board sees it"));
    for minutes in [1, 3, 5, 10, 15, 30] {
        let button = Button::new(trf("{} min", &[&minutes]));
        if ui.add_enabled(!view_only, button).clicked() {
            actions.push(Action::StartTimer(minutes * 60_000));
            ui.close();
        }
    }
    ui.separator();
    ui.weak(tr("Your own time, like 7 or 2:30:"));
    ui.horizontal(|ui| {
        let field = ui.add_enabled(
            !view_only,
            egui::TextEdit::singleline(text)
                .desired_width(70.0)
                .hint_text(tr("min")),
        );
        let ms = parse_timer(text);
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let start = ui
            .add_enabled(!view_only && ms.is_some(), Button::new(tr("Start")))
            .clicked();
        if let Some(ms) = ms.filter(|_| start || enter) {
            actions.push(Action::StartTimer(ms));
            ui.close();
        }
    });
    if !text.trim().is_empty() && parse_timer(text).is_none() {
        ui.colored_label(
            Color32::from_rgb(0xDC, 0x26, 0x26),
            tr("From 1 second to 24 hours"),
        );
    }
}

/// The countdown at the top middle, under the board menu, with its buttons if it can be
/// `changed` here.
fn timer_pill(ctx: &Context, timer: &TimerView, changed: bool, actions: &mut Vec<Action>) {
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
                        (tr("Time's up").to_owned(), color)
                    } else {
                        (timer.text.clone(), ui.visuals().strong_text_color())
                    };
                    ui.label(RichText::new(text).size(20.0).color(color).strong());
                    if !changed {
                        return;
                    }
                    if !timer.done {
                        let label = if timer.running {
                            tr("Pause")
                        } else {
                            tr("Resume")
                        };
                        let tip = if timer.running {
                            tr("Stop the clock")
                        } else {
                            tr("Run the clock on")
                        };
                        if ui.button(label).on_hover_text(tip).clicked() {
                            actions.push(Action::PauseTimer);
                        }
                    }
                    if ui.button("↺").on_hover_text(tr("Start over")).clicked() {
                        actions.push(Action::ResetTimer);
                    }
                    if ui.button("×").on_hover_text(tr("Close")).clicked() {
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
                    ui.strong(tr("Layers"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(tr("+ Add"))
                            .on_hover_text(tr("New layer on top"))
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
                                .on_hover_text(tr("Visible"))
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
                                        .on_hover_text(tr("New objects go to the selected layer; double click to rename"));
                                    if label.double_clicked() {
                                        *renaming =
                                            Some((layer.id.clone(), layer.name.to_string()));
                                    } else if label.clicked() {
                                        actions.push(Action::ActivateLayer(layer.id.clone()));
                                    }
                                }
                            }
                            if ui
                                .selectable_label(layer.locked, tr("Lock"))
                                .on_hover_text(tr("Nothing on a locked layer can be changed"))
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
                                .on_hover_text(tr("Remove the layer and everything on it"));
                            if remove.clicked() {
                                actions.push(Action::RemoveLayer(layer.id.clone()));
                            }
                            ui.end_row();
                        }
                    });
            });
        });
}

/// Minutes and seconds, like 1:05.
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Recording a talktrack, and the board's talktracks to play or remove.
fn talktracks_menu(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    let record = Button::new(tr("● Record talktrack"));
    if ui
        .add_enabled(model.recording.is_none(), record)
        .on_hover_text(tr("Record your voice while you move around the board"))
        .clicked()
    {
        actions.push(Action::StartRecording);
        ui.close();
    }
    if !model.talktracks.is_empty() {
        ui.separator();
    }
    for (id, title, length) in &model.talktracks {
        ui.horizontal(|ui| {
            let label = format!("▶  {title}  ({})", clock(f64::from(*length) / 1000.0));
            if ui.add(Button::new(label).frame(false)).clicked() {
                actions.push(Action::PlayTalktrack(id.clone()));
                ui.close();
            }
            if ui.small_button("×").on_hover_text(tr("Remove")).clicked() {
                actions.push(Action::RemoveTalktrack(id.clone()));
            }
        });
    }
}

/// Saving versions of the board and going back to them.
fn history_menu(ui: &mut egui::Ui, model: &Model, name: &mut String, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(name)
                .hint_text(tr("Version name"))
                .desired_width(170.0),
        );
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui
            .button(tr("Save version"))
            .on_hover_text(tr("Remember the board as it is now"))
            .clicked()
            || entered
        {
            actions.push(Action::SaveVersion(std::mem::take(name)));
            ui.close();
        }
    });
    ui.separator();
    if model.versions.is_empty() {
        ui.weak(tr("No saved versions yet"));
    }
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .show(ui, |ui| {
            for version in &model.versions {
                ui.horizontal(|ui| {
                    let label = format!("{}  ·  {}", version.name, version.when);
                    let hint = trf(
                        "Saved by {}. Look at the board as it was",
                        &[&version.author],
                    );
                    if ui
                        .add(Button::new(label).frame(false))
                        .on_hover_text(hint)
                        .clicked()
                    {
                        actions.push(Action::OpenVersion(version.id.clone()));
                        ui.close();
                    }
                    if ui
                        .small_button("×")
                        .on_hover_text(tr("Forget this version"))
                        .clicked()
                    {
                        actions.push(Action::RemoveVersion(version.id.clone()));
                    }
                });
            }
        });
}

/// Shown at the top while a saved version is looked at.
fn version_bar(ctx: &Context, version: &VersionBanner, read_only: bool, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    Area::new(Id::unique("version"))
        .fixed_pos(Pos2::new(screen.center().x, MARGIN))
        .pivot(Align2::CENTER_TOP)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(&version.name);
                    ui.label(trf("saved {} by {}", &[&version.when, &version.author]));
                    ui.separator();
                    ui.weak(&version.summary)
                        .on_hover_text(tr("Changed and removed objects are outlined"));
                    ui.separator();
                    if ui
                        .add_enabled(!read_only, Button::new(tr("Restore")))
                        .on_hover_text(tr(
                            "Bring the board back to this version (Ctrl+Z undoes it)",
                        ))
                        .on_disabled_hover_text(tr("The board is read-only"))
                        .clicked()
                    {
                        actions.push(Action::RestoreVersion);
                    }
                    if ui
                        .button(tr("Close"))
                        .on_hover_text(tr("Back to the board (Esc)"))
                        .clicked()
                    {
                        actions.push(Action::CloseVersion);
                    }
                });
            });
        });
}

/// Shows that a talktrack is being recorded, with the time and the buttons to stop.
fn recording_pill(ctx: &Context, seconds: u64, sound: bool, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    let red = Color32::from_rgb(0xDC, 0x26, 0x26);
    Area::new(Id::unique("recording"))
        .fixed_pos(Pos2::new(MARGIN, screen.bottom() - MARGIN))
        .pivot(Align2::LEFT_BOTTOM)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").color(red).size(16.0));
                    ui.strong(clock(seconds as f64));
                    if !sound {
                        ui.weak(tr("no microphone"));
                    }
                    if ui
                        .button(tr("Stop"))
                        .on_hover_text(tr("Keep the talktrack"))
                        .clicked()
                    {
                        actions.push(Action::StopRecording);
                    }
                    if ui
                        .button(tr("Cancel"))
                        .on_hover_text(tr("Throw it away"))
                        .clicked()
                    {
                        actions.push(Action::CancelRecording);
                    }
                });
            });
        });
}

/// The controls at the bottom while a talktrack plays.
fn playback_bar(ctx: &Context, view: &PlaybackView, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    Area::new(Id::unique("playback"))
        .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - MARGIN))
        .pivot(Align2::CENTER_BOTTOM)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let play = if view.paused { tr("Play") } else { tr("Pause") };
                    if ui.button(play).on_hover_text(tr("Space")).clicked() {
                        actions.push(Action::TogglePlayback);
                    }
                    let mut at = view.position;
                    let slider =
                        egui::Slider::new(&mut at, 0.0..=view.length.max(0.1)).show_value(false);
                    if ui.add_sized([240.0, 18.0], slider).changed() {
                        actions.push(Action::SeekPlayback(at));
                    }
                    ui.label(format!("{} / {}", clock(view.position), clock(view.length)));
                    ui.separator();
                    ui.label(&view.title);
                    if ui.button(tr("Exit")).on_hover_text(tr("Esc")).clicked() {
                        actions.push(Action::StopPlayback);
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
                    ui.strong(tr("Focus mode"));
                    ui.weak(tr("the rest of the board is out of reach"));
                    ui.separator();
                    if ui
                        .button(tr("Exit"))
                        .on_hover_text(tr("Esc, Shift+F"))
                        .clicked()
                    {
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
                        .on_hover_text(tr("Previous (←)"))
                        .clicked()
                    {
                        actions.push(Action::PresentStep(-1));
                    }
                    let title = if title.trim().is_empty() {
                        tr("Untitled frame")
                    } else {
                        title
                    };
                    ui.label(format!("{title}  ·  {} / {count}", index + 1));
                    if ui
                        .add_enabled(index + 1 < count, Button::new("▶"))
                        .on_hover_text(tr("Next (→, Space)"))
                        .clicked()
                    {
                        actions.push(Action::PresentStep(1));
                    }
                    ui.separator();
                    if ui
                        .selectable_label(laser, tr("Laser"))
                        .on_hover_text(tr("Point with a red dot (L)"))
                        .clicked()
                    {
                        actions.push(Action::ToggleLaser);
                    }
                    if ui.button(tr("Exit")).on_hover_text(tr("Esc")).clicked() {
                        actions.push(Action::StopPresenting);
                    }
                });
            });
        });
}

/// The template gallery: a card per template.
fn templates_window(ctx: &Context, actions: &mut Vec<Action>) {
    let mut open = true;
    let shown = egui::Window::new(tr("Templates"))
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
                                job.append(
                                    tr(template.name),
                                    0.0,
                                    egui::TextFormat::simple(body, ink),
                                );
                                job.append(
                                    &format!(
                                        "
{}",
                                        tr(template.description)
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

/// Saving a picture of the board, a frame or the selection.
fn export_window(
    ctx: &Context,
    model: &Model,
    info: ExportInfo,
    settings: &mut crate::export::Settings,
    actions: &mut Vec<Action>,
) {
    use crate::export::{Format, Scope};

    let gone = match settings.scope {
        Scope::Frame(id) => !model.frames.iter().any(|(f, _)| *f == id),
        // One page per frame needs pages.
        Scope::Frames => !settings.format.has_pages() || model.frames.is_empty(),
        _ => false,
    };
    if gone {
        settings.scope = Scope::Board;
    }
    let title_of = |id: ObjectId| {
        model
            .frames
            .iter()
            .find(|(f, _)| *f == id)
            .map(|(_, t)| frame_title(t))
            .unwrap_or_default()
    };
    let mut open = true;
    let shown = egui::Window::new(tr("Export"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            egui::Grid::new("export")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(tr("What"));
                    ui.vertical(|ui| {
                        ui.radio_value(&mut settings.scope, Scope::Board, tr("Whole board"));
                        ui.add_enabled_ui(info.has_selection, |ui| {
                            ui.radio_value(&mut settings.scope, Scope::Selection, tr("Selection"));
                        });
                        ui.add_enabled_ui(!model.frames.is_empty(), |ui| {
                            ui.horizontal(|ui| {
                                let frame = match settings.scope {
                                    Scope::Frame(id) => Some(id),
                                    _ => None,
                                };
                                let shown = frame.or(model.frames.first().map(|(id, _)| *id));
                                if ui.radio(frame.is_some(), tr("Frame")).clicked()
                                    && let Some(id) = shown
                                {
                                    settings.scope = Scope::Frame(id);
                                }
                                egui::ComboBox::from_id_salt("export frame")
                                    .selected_text(shown.map(title_of).unwrap_or_default())
                                    .show_ui(ui, |ui| {
                                        for (id, title) in &model.frames {
                                            let on = frame == Some(*id);
                                            if ui.selectable_label(on, frame_title(title)).clicked()
                                            {
                                                settings.scope = Scope::Frame(*id);
                                            }
                                        }
                                    });
                            });
                        });
                        let pages = settings.format.has_pages() && !model.frames.is_empty();
                        ui.add_enabled_ui(pages, |ui| {
                            ui.radio_value(
                                &mut settings.scope,
                                Scope::Frames,
                                tr("All frames, a page each"),
                            )
                            .on_disabled_hover_text(tr("PDF only, for boards with frames"));
                        });
                    });
                    ui.end_row();

                    ui.label(tr("Format"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut settings.format, Format::Png, tr("PNG"));
                        ui.selectable_value(&mut settings.format, Format::Jpeg, tr("JPEG"));
                        ui.selectable_value(&mut settings.format, Format::Svg, tr("SVG"))
                            .on_hover_text(tr("Shapes and text as lines, sharp at any size"));
                        ui.selectable_value(&mut settings.format, Format::Pdf, tr("PDF"))
                            .on_hover_text(tr("Shapes and text as lines, sharp at any size"));
                        ui.selectable_value(
                            &mut settings.format,
                            Format::PdfPictures,
                            tr("PDF (pictures)"),
                        )
                        .on_hover_text(tr("Each page a picture, as it looks on the board"));
                        ui.separator();
                        ui.selectable_value(&mut settings.format, Format::Markdown, tr("Markdown"))
                            .on_hover_text(tr("The text: frames as headings, notes as a list"));
                        ui.selectable_value(&mut settings.format, Format::Csv, tr("CSV"))
                            .on_hover_text(tr("The first table, for spreadsheets"));
                    });
                    ui.end_row();

                    ui.label(tr("Size"));
                    let pixels = !settings.format.is_text() && !settings.format.is_vector();
                    ui.add_enabled_ui(pixels, |ui| {
                        ui.horizontal(|ui| {
                            for scale in [1.0, 2.0, 3.0, 4.0] {
                                ui.selectable_value(
                                    &mut settings.scale,
                                    scale,
                                    format!("{scale}×"),
                                )
                                .on_hover_text(tr(
                                    "Pixels per board unit: 1× is the board at 100%",
                                ));
                            }
                        });
                    });
                    ui.end_row();

                    ui.label("");
                    ui.add_enabled(
                        matches!(settings.format, Format::Png | Format::Svg),
                        egui::Checkbox::new(
                            &mut settings.transparent,
                            tr("Transparent background"),
                        ),
                    );
                    ui.end_row();
                });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Export…")).clicked() {
                    actions.push(Action::Export(*settings));
                }
                if ui.button(tr("Cancel")).clicked() {
                    actions.push(Action::CloseExport);
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
        actions.push(Action::CloseExport);
    }
}

/// How a frame is listed: its title, or a stand-in when it has none.
fn frame_title(title: &str) -> String {
    if title.trim().is_empty() {
        tr("Untitled frame").to_owned()
    } else {
        title.to_owned()
    }
}

/// A Mermaid flowchart to turn into shapes and connectors.
fn diagram_window(
    ctx: &Context,
    error: Option<&str>,
    text: &mut String,
    actions: &mut Vec<Action>,
) {
    let mut open = true;
    let shown = egui::Window::new(tr("Diagram from text"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(tr("A Mermaid flowchart: nodes like A[Box], B{Decision}, C((Circle)); edges like A --> B or A -->|label| B."));
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
                if ui.button(tr("Insert")).clicked() {
                    actions.push(Action::InsertDiagram(text.clone()));
                }
                if ui.button(tr("Cancel")).clicked() {
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
    // Esc closes it even from the text field, which keeps the key from the board.
    if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(Action::CloseDiagram);
    }
}

/// The window that draws graphs of functions: formulas and the range of x.
fn graph_window(
    ctx: &Context,
    error: Option<&str>,
    (formulas, from, to): &mut (String, String, String),
    actions: &mut Vec<Action>,
) {
    let mut open = true;
    let mut insert = false;
    let shown = egui::Window::new(tr("Graph of a function"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(tr("y = … of x, several separated by ;. For example: sin(x); x^2 - 3x + 1; 1/x; sqrt(x); |x|; 2^x; ln(x)"));
            let field = ui.add(egui::TextEdit::singleline(formulas).desired_width(420.0));
            insert |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                ui.label(tr("x from"));
                ui.add(egui::TextEdit::singleline(from).desired_width(60.0));
                ui.label(tr("to"));
                ui.add(egui::TextEdit::singleline(to).desired_width(60.0));
            });
            if let Some(error) = error {
                ui.colored_label(Color32::from_rgb(0xD0, 0x30, 0x30), error);
            }
            ui.horizontal(|ui| {
                insert |= ui.button(tr("Insert")).clicked();
                if ui.button(tr("Cancel")).clicked() {
                    actions.push(Action::CloseGraph);
                }
            });
        });
    if insert {
        actions.push(Action::InsertGraph {
            formulas: formulas.clone(),
            from: from.clone(),
            to: to.clone(),
        });
    }
    if let Some(shown) = shown {
        let layer = shown.response.layer_id;
        let on_top = ctx.memory(|m| m.areas().top_layer_id(Order::Foreground)) == Some(layer);
        if !on_top {
            ctx.move_to_top(layer);
            ctx.request_repaint();
        }
    }
    if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(Action::CloseGraph);
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
            ("Shift (drawing)", "Straight line with the pen or marker"),
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
            (
                "Ctrl+K, Ctrl+Shift+P",
                "Command palette: any command, frame or text",
            ),
            ("Ctrl+F", "Find"),
            ("Ctrl+N / O / S", "New / open / save"),
            ("Ctrl+Shift+S", "Save as"),
            ("Ctrl+Shift+E", "Export a picture or PDF"),
            ("Ctrl+P", "Print (opens a PDF)"),
            (
                "PageDown / PageUp",
                "Next / previous page of a selected PDF book",
            ),
            ("Wheel, Shift+wheel", "Scroll up and down, sideways"),
            ("Space+drag, middle drag", "Pan"),
            ("Ctrl+wheel, Ctrl+= / -", "Zoom"),
            ("Ctrl+0", "Zoom to 100%"),
            (
                "Ctrl+Alt+= / - / 0",
                "Interface size: larger / smaller / normal",
            ),
            ("Home", "Back to the start"),
            ("G / Shift+G", "Grid style / snap to grid"),
            ("F1, ?", "This list"),
            ("Shift+F", "Focus on the selection or the view"),
            ("F5", "Present the frames"),
            ("F11", "Full screen"),
            ("Ctrl+\\", "Clean board: hide the panels"),
            ("L (presenting)", "Laser pointer"),
        ],
    ),
];

/// Whether shortcut keys say more than key names, like "Enter, double click": those are
/// translated, while "Ctrl+B / I / U" or "Delete" stay as on the keyboard.
fn has_words(keys: &str) -> bool {
    keys.split(|c: char| !c.is_ascii_alphabetic())
        .any(|word| word.len() > 1 && word.chars().all(|c| c.is_ascii_lowercase()))
}

/// The list of keyboard shortcuts, above the other panels.
fn shortcuts_window(ctx: &Context, actions: &mut Vec<Action>) {
    let mut open = true;
    let max_height = ctx.content_rect().height() * 0.75;
    let shown = egui::Window::new(tr("Keyboard shortcuts"))
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
                        ui.strong(tr(topic));
                        egui::Grid::new(*topic)
                            .num_columns(2)
                            .spacing([16.0, 4.0])
                            .min_col_width(150.0)
                            .show(ui, |ui| {
                                for (key, what) in *keys {
                                    let key = if has_words(key) { tr(key) } else { key };
                                    ui.label(RichText::new(key).monospace());
                                    ui.label(tr(what));
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
                            .hint_text(tr("Find on the board"))
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
                        (0, true) => tr("No results").to_owned(),
                        (0, false) => String::new(),
                        (n, _) => trf("{} of {}", &[&(info.current + 1), &n]),
                    };
                    ui.label(status);
                    let some = info.count > 0;
                    if ui
                        .add_enabled(some, Button::new("↑"))
                        .on_hover_text(tr("Previous (Shift+Enter)"))
                        .clicked()
                    {
                        actions.push(Action::SearchStep(false));
                    }
                    if ui
                        .add_enabled(some, Button::new("↓"))
                        .on_hover_text(tr("Next (Enter)"))
                        .clicked()
                    {
                        actions.push(Action::SearchStep(true));
                    }
                    if ui.button("×").on_hover_text(tr("Close (Esc)")).clicked() {
                        actions.push(Action::CloseSearch);
                    }
                });
            });
        });
}

/// What the command palette keeps between frames.
#[derive(Default)]
struct PaletteState {
    /// What is typed.
    text: String,
    /// The entry picked with the arrow keys, in the list shown.
    picked: usize,
    /// Entries run lately, last first.
    recent: Vec<Action>,
}

/// What the board allows now, for the commands the palette offers.
#[derive(Clone, Copy, Debug, Default)]
struct PaletteCan {
    read_only: bool,
    view_only: bool,
    undo: bool,
    redo: bool,
    present: bool,
    /// Something is selected; and several things, a group, something locked.
    selection: bool,
    several: bool,
    group: bool,
    locked: bool,
    voting: Option<VotingView>,
    timer: bool,
    recording: bool,
}

/// What the menus and the tools offer, for the command palette: the English label, the
/// keys, the action, and whether it can be done now.
fn palette_commands(can: PaletteCan) -> Vec<(&'static str, &'static str, Action, bool)> {
    let edit = !can.read_only;
    let selected = can.selection;
    let free = selected && edit && !can.locked;
    let voting = can.voting.is_some_and(|v| v.open);
    let voted = can.voting.is_some_and(|v| !v.open);
    let timer = !can.view_only;
    let tool = |kind| Action::Tool(kind);
    vec![
        ("New board", "Ctrl+N", Action::NewBoard, true),
        ("Open…", "Ctrl+O", Action::Open, true),
        ("Save", "Ctrl+S", Action::Save, true),
        ("Save as…", "Ctrl+Shift+S", Action::SaveAs, true),
        ("Insert file…", "", Action::InsertImage, edit),
        ("Diagram from text…", "", Action::OpenDiagram, edit),
        ("Graph of a function…", "", Action::OpenGraph, edit),
        ("Export…", "Ctrl+Shift+E", Action::OpenExport, true),
        ("Print…", "Ctrl+P", Action::Print, true),
        ("Read-only", "", Action::ToggleReadOnly, true),
        ("Undo", "Ctrl+Z", Action::Undo, can.undo),
        ("Redo", "Ctrl+Y", Action::Redo, can.redo),
        ("Select all", "Ctrl+A", Action::SelectAll, true),
        ("Copy", "Ctrl+C", Action::Copy, selected),
        ("Cut", "Ctrl+X", Action::Cut, free),
        ("Duplicate", "Ctrl+D", Action::Duplicate, selected && edit),
        ("Delete", "Delete", Action::Delete, free),
        (
            "Bring to front",
            "Ctrl+Shift+]",
            Action::Reorder(ZOrder::Front),
            free,
        ),
        (
            "Send to back",
            "Ctrl+Shift+[",
            Action::Reorder(ZOrder::Back),
            free,
        ),
        ("Group", "Ctrl+G", Action::Group, free && can.several),
        (
            "Ungroup",
            "Ctrl+Shift+G",
            Action::Ungroup,
            free && can.group,
        ),
        (
            "Lock / unlock",
            "Ctrl+Shift+L",
            Action::ToggleLock,
            selected && edit,
        ),
        (
            "Hide / show",
            "Ctrl+Shift+H",
            Action::ToggleHidden,
            selected && edit,
        ),
        (
            "Focus on the selection or the view",
            "Shift+F",
            Action::ToggleFocus,
            true,
        ),
        ("Find", "Ctrl+F", Action::OpenSearch, true),
        ("Present", "F5", Action::Present, can.present),
        ("Templates", "", Action::ToggleTemplates, edit),
        ("Share", "", Action::ToggleShare, true),
        ("Manage plugins…", "", Action::TogglePlugins, true),
        ("Keyboard shortcuts", "F1", Action::ToggleShortcuts, true),
        (
            "Record talktrack",
            "",
            Action::StartRecording,
            !can.recording,
        ),
        ("Save version", "", Action::SaveVersion(String::new()), true),
        ("Start voting", "", Action::StartVoting(5), !voting),
        ("End voting", "", Action::EndVoting, voting),
        ("Voting results", "", Action::ToggleResults, voted),
        ("Clear votes", "", Action::ClearVotes, voted),
        (
            "Timer: 5 minutes",
            "",
            Action::StartTimer(5 * 60_000),
            timer,
        ),
        (
            "Timer: 10 minutes",
            "",
            Action::StartTimer(10 * 60_000),
            timer,
        ),
        (
            "Pause or resume the timer",
            "",
            Action::PauseTimer,
            timer && can.timer,
        ),
        (
            "Close the timer",
            "",
            Action::CloseTimer,
            timer && can.timer,
        ),
        ("Zoom in", "Ctrl+=", Action::ZoomIn, true),
        ("Zoom out", "Ctrl+−", Action::ZoomOut, true),
        ("Zoom to 100%", "Ctrl+0", Action::ZoomReset, true),
        ("Zoom to fit", "", Action::ZoomToFit, true),
        ("Grid style", "G", Action::NextGrid, true),
        ("Snap to grid", "Shift+G", Action::ToggleSnap, true),
        ("Minimap", "", Action::ToggleMinimap, true),
        ("Layers", "", Action::ToggleLayers, true),
        ("Dark theme", "", Action::ToggleTheme, true),
        (
            "Language: English / Russian",
            "",
            Action::ToggleLanguage,
            true,
        ),
        ("High contrast", "", Action::ToggleHighContrast, true),
        ("Full screen", "F11", Action::ToggleFullscreen, true),
        ("Clean board", "Ctrl+\\", Action::ToggleClean, true),
        ("Select", "V", tool(ToolKind::Select), true),
        (
            "Rectangle",
            "R",
            tool(ToolKind::Shape(ShapeKind::Rectangle)),
            edit,
        ),
        (
            "Ellipse",
            "O",
            tool(ToolKind::Shape(ShapeKind::Ellipse)),
            edit,
        ),
        ("Text", "T", tool(ToolKind::Text), edit),
        ("Sticky note", "N", tool(ToolKind::Sticky), edit),
        ("Connector", "L", tool(ToolKind::Connector), edit),
        ("Pen", "P", tool(ToolKind::Pen), edit),
        ("Marker", "M", tool(ToolKind::Marker), edit),
        ("Eraser", "E", tool(ToolKind::Eraser), edit),
        ("Comment", "C", tool(ToolKind::Comment), true),
        ("Frame", "F", tool(ToolKind::Frame), edit),
        ("Table", "", tool(ToolKind::Table), edit),
        ("Kanban board", "K", tool(ToolKind::Kanban), edit),
        ("Mind map", "", tool(ToolKind::MindMap), edit),
        ("Markdown document", "D", tool(ToolKind::Doc), edit),
    ]
}

/// An entry of the command palette.
struct PaletteEntry {
    /// As shown, and in English.
    label: String,
    english: String,
    /// Keys or a word about it, shown at the right.
    hint: String,
    action: Action,
}

/// Everything the command palette offers now, but objects on the board: commands, frames
/// and plugins' commands.
fn palette_entries(model: &Model) -> Vec<PaletteEntry> {
    let selection = model.selection.as_ref();
    let can = PaletteCan {
        read_only: model.read_only,
        view_only: model.view_only,
        undo: model.can_undo,
        redo: model.can_redo,
        present: model.can_present,
        selection: selection.is_some(),
        several: selection.is_some_and(|s| s.several),
        group: selection.is_some_and(|s| s.has_group),
        locked: selection.is_some_and(|s| s.locked),
        voting: model.voting,
        timer: model.timer.is_some(),
        recording: model.recording.is_some(),
    };
    let mut entries: Vec<PaletteEntry> = palette_commands(can)
        .into_iter()
        .filter(|(.., on)| *on)
        .map(|(label, keys, action, _)| PaletteEntry {
            label: tr(label).to_owned(),
            english: label.to_owned(),
            hint: keys.to_owned(),
            action,
        })
        .collect();
    for (id, title) in &model.frames {
        let title = if title.trim().is_empty() {
            tr("Untitled frame")
        } else {
            title
        };
        entries.push(PaletteEntry {
            label: trf("Go to frame: {}", &[&title]),
            english: format!("Go to frame: {title}"),
            hint: String::new(),
            action: Action::GoToFrame(*id),
        });
    }
    if !model.read_only {
        for c in &model.plugin_commands {
            let label = format!("{}: {}", c.plugin_name, c.title);
            entries.push(PaletteEntry {
                english: label.clone(),
                label,
                hint: tr("Plugin").to_owned(),
                action: Action::RunPluginCommand(c.plugin.clone(), c.id.clone()),
            });
        }
    }
    entries
}

/// The command palette at the top middle (Ctrl+K): a field to type in and the commands,
/// frames and objects that fit, to run with Enter or a click.
fn command_palette(
    ctx: &Context,
    model: &Model,
    objects: &[(String, kurbo::Rect)],
    state: &mut PaletteState,
    actions: &mut Vec<Action>,
) {
    let entries = palette_entries(model);
    let labels: Vec<(&str, &str)> = entries
        .iter()
        .map(|e| (e.label.as_str(), e.english.as_str()))
        .collect();
    let recent: Vec<usize> = state
        .recent
        .iter()
        .filter_map(|a| entries.iter().position(|e| e.action == *a))
        .collect();
    let found: Vec<PaletteEntry> = objects
        .iter()
        .map(|(text, area)| PaletteEntry {
            label: text.clone(),
            english: text.clone(),
            hint: tr("On the board").to_owned(),
            action: Action::ShowArea(*area),
        })
        .collect();
    let shown: Vec<&PaletteEntry> = crate::palette::rank(&state.text, &labels, &recent)
        .into_iter()
        .map(|i| &entries[i])
        .chain(&found)
        .collect();

    // The field keeps the keyboard: these keys move through the list and run or close.
    use egui::{Key, Modifiers};
    let (up, down, enter, escape, toggle) = ctx.input_mut(|i| {
        let up = i.consume_key(Modifiers::SHIFT, Key::Tab)
            | i.consume_key(Modifiers::NONE, Key::ArrowUp);
        let down = i.consume_key(Modifiers::NONE, Key::Tab)
            | i.consume_key(Modifiers::NONE, Key::ArrowDown);
        let enter = i.consume_key(Modifiers::NONE, Key::Enter);
        let escape = i.consume_key(Modifiers::NONE, Key::Escape);
        let toggle = i.consume_key(Modifiers::COMMAND, Key::K)
            | i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
        (up, down, enter, escape, toggle)
    });
    let count = shown.len();
    if count > 0 {
        if down {
            state.picked = (state.picked + 1) % count;
        }
        if up {
            state.picked = (state.picked + count - 1) % count;
        }
    }
    state.picked = state.picked.min(count.saturating_sub(1));
    let mut run = enter.then_some(state.picked).filter(|_| count > 0);

    let screen = ctx.content_rect();
    let width = (screen.width() - 2.0 * MARGIN - 16.0).clamp(160.0, 480.0);
    let area = Area::new(Id::unique("command palette"))
        .fixed_pos(Pos2::new(screen.center().x, MARGIN + 52.0))
        .pivot(Align2::CENTER_TOP)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.set_width(width);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut state.text)
                        .id(Id::unique("command palette field"))
                        .hint_text(tr("Type a command, a frame or text on the board"))
                        .desired_width(f32::INFINITY)
                        .event_filter(egui::EventFilter {
                            tab: true,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: true,
                        }),
                );
                field.request_focus();
                if field.changed() {
                    state.picked = 0;
                    actions.push(Action::PaletteQuery(state.text.clone()));
                }
                ui.add_space(4.0);
                if shown.is_empty() {
                    ui.weak(tr("Nothing found"));
                }
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (i, entry) in shown.iter().enumerate() {
                            let picked = i == state.picked;
                            let button = Button::selectable(picked, entry.label.as_str())
                                .shortcut_text(entry.hint.as_str())
                                .min_size(Vec2::new(ui.available_width(), 0.0));
                            let row = ui.add(button);
                            if row.clicked() {
                                run = Some(i);
                            }
                            if picked && (up || down) {
                                row.scroll_to_me(None);
                            }
                        }
                    });
            });
        });
    let layer = area.response.layer_id;
    if ctx.memory(|m| m.areas().top_layer_id(Order::Foreground)) != Some(layer) {
        ctx.move_to_top(layer);
        ctx.request_repaint();
    }
    let pressed_elsewhere = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .pointer_interact_pos()
            .is_some_and(|p| !area.response.rect.contains(p));
    if let Some(entry) = run.and_then(|i| shown.get(i)) {
        actions.push(Action::ClosePalette);
        actions.push(entry.action.clone());
        // Objects found are not commands to offer again.
        if !matches!(entry.action, Action::ShowArea(_)) {
            crate::palette::remember(&mut state.recent, entry.action.clone());
        }
    } else if escape || toggle || pressed_elsewhere {
        actions.push(Action::ClosePalette);
    }
}

/// Tags and fill colors to show only some objects, and managing the board's tags.
fn filter_menu(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    let filter = model.filter;
    ui.strong(tr("Tags"));
    if model.tags.is_empty() {
        ui.weak(tr("No tags yet: select objects and use Tags"));
    }
    for (tag, count) in &model.tags {
        ui.horizontal(|ui| {
            if tag_dot(ui, tag.color)
                .on_hover_text(tr("Change the color"))
                .clicked()
            {
                actions.push(Action::RecolorTag(tag.id.clone()));
            }
            let on = filter.tags.contains(&tag.id);
            let label = format!("{} ({count})", tag.name);
            if ui
                .selectable_label(on, label)
                .on_hover_text(tr("Show objects with this tag"))
                .clicked()
            {
                actions.push(Action::FilterTag(tag.id.clone()));
            }
            if ui
                .small_button("×")
                .on_hover_text(tr("Remove the tag from the board"))
                .clicked()
            {
                actions.push(Action::RemoveTag(tag.id.clone()));
            }
        });
    }
    if !model.fill_colors.is_empty() {
        ui.separator();
        ui.strong(tr("Colors"));
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
        if ui.button(tr("Show everything")).clicked() {
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
    let r = to_points(format.screen);
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
                // Too wide for the window: the buttons go on in more rows.
                ui.set_max_width(ctx.content_rect().width() - 2.0 * MARGIN - 16.0);
                ui.horizontal_wrapped(|ui| {
                    let m = format.marks;
                    let marks = [
                        (
                            Mark::Bold,
                            m.bold,
                            RichText::new(tr("B")).strong(),
                            "Bold (Ctrl+B)",
                        ),
                        (
                            Mark::Italic,
                            m.italic,
                            RichText::new(tr("I")).italics(),
                            "Italic (Ctrl+I)",
                        ),
                        (
                            Mark::Underline,
                            m.underline,
                            RichText::new(tr("U")).underline(),
                            "Underline (Ctrl+U)",
                        ),
                        (
                            Mark::Strike,
                            m.strike,
                            RichText::new(tr("S")).strikethrough(),
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
                            tr("Text color of the object")
                        } else {
                            tr("Text color")
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
                let name = if color.to_bytes()[3] == 0 {
                    tr("None")
                } else {
                    ""
                };
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

fn swatch_button(ui: &mut egui::Ui, color: Rgba, ring: bool, tooltip: &'static str) -> Response {
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
    tooltip: &'static str,
    paint: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
    let painter = ui.painter();
    let enabled = ui.is_enabled();
    if selected {
        painter.rect_filled(rect, 8, ACCENT.gamma_multiply(0.15));
    } else if response.hovered() && enabled {
        painter.rect_filled(rect, 8, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let color = if selected {
        ACCENT
    } else if enabled {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    paint(painter, rect.shrink(9.0), color);
    response.on_hover_text(tr(tooltip))
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

/// The Plugins menu: the commands plugins offer, and managing plugins.
fn plugins_menu(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    if model.plugin_commands.is_empty() {
        ui.weak(tr("No plugin commands yet"));
    }
    for c in &model.plugin_commands {
        let button = Button::new(&c.title);
        if ui
            .add_enabled(!model.read_only, button)
            .on_hover_text(&c.plugin_name)
            .clicked()
        {
            actions.push(Action::RunPluginCommand(c.plugin.clone(), c.id.clone()));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tr("Manage plugins…")).clicked() {
        actions.push(Action::TogglePlugins);
        ui.close();
    }
}

/// Installed plugins: what they are and may do, on or off, their log.
fn plugins_window(ctx: &Context, plugins: &[PluginView], web: bool, actions: &mut Vec<Action>) {
    let mut open = true;
    egui::Window::new(tr("Plugins"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(460.0)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            if plugins.is_empty() {
                ui.label(tr("No plugins installed. A plugin is a folder with manifest.json and a script; see the user guide."));
            }
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for p in plugins {
                    ui.horizontal(|ui| {
                        let mut on = p.enabled;
                        if ui.checkbox(&mut on, "").on_hover_text(tr("On")).changed() {
                            actions.push(Action::SetPluginEnabled(p.id.clone(), on));
                        }
                        ui.strong(&p.name);
                        ui.weak(&p.version);
                        if !p.author.is_empty() {
                            ui.weak(trf("by {}", &[&p.author]));
                        }
                        if p.dev {
                            ui.colored_label(Color32::from_rgb(0x3B, 0x82, 0xF6), tr("development"))
                                .on_hover_text(tr("Runs from its own folder and reloads when its files change"));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(tr("Remove")).clicked() {
                                actions.push(Action::RemovePlugin(p.id.clone()));
                            }
                            if p.dev
                                && ui
                                    .small_button(tr("Pack…"))
                                    .on_hover_text(tr("Save it as one .pbplugin file to give to others"))
                                    .clicked()
                            {
                                actions.push(Action::PackPlugin(p.id.clone()));
                            }
                        });
                    });
                    if !p.description.is_empty() {
                        ui.label(&p.description);
                    }
                    let may = if p.permissions.is_empty() {
                        tr("Asks for no permissions").to_owned()
                    } else {
                        trf("May: {}", &[&p.permissions.join(", ")])
                    };
                    ui.weak(may);
                    if let Some(error) = &p.error {
                        ui.colored_label(Color32::from_rgb(0xDC, 0x26, 0x26), error);
                    } else if p.enabled && !p.running {
                        ui.weak(tr("Not running"));
                    }
                    egui::CollapsingHeader::new(trf("Log ({})", &[&p.log.len()]))
                        .id_salt(("plugin log", &p.id))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt(("plugin log lines", &p.id))
                                .max_height(160.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    for (level, text) in &p.log {
                                        let line = RichText::new(text).monospace().small();
                                        let line = match level.as_str() {
                                            "error" => line.color(Color32::from_rgb(0xDC, 0x26, 0x26)),
                                            "warn" => line.color(Color32::from_rgb(0xD9, 0x77, 0x06)),
                                            _ => line,
                                        };
                                        ui.label(line);
                                    }
                                });
                        });
                    ui.separator();
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .button(tr("Install plugin…"))
                    .on_hover_text(tr("Pick a .pbplugin file, or the plugin's manifest.json (in the browser: with its scripts)"))
                    .clicked()
                {
                    actions.push(Action::InstallPlugin);
                }
                if !web
                    && ui
                        .button(tr("Develop from a folder…"))
                        .on_hover_text(tr("Runs a plugin from its folder and reloads it when you save its files"))
                        .clicked()
                {
                    actions.push(Action::LoadDevPlugin);
                }
            });
            ui.weak(tr("Install plugins only from people you trust: a plugin can change your boards."));
        });
    if !open {
        actions.push(Action::TogglePlugins);
    }
}

/// A plugin's own window: text, fields, buttons and lists it described.
fn plugin_panel(
    ctx: &Context,
    plugin: &str,
    name: &str,
    panel: &serde_json::Value,
    values: &mut std::collections::HashMap<(String, String), String>,
    actions: &mut Vec<Action>,
) {
    use serde_json::{Value, json};
    let title = panel
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(name)
        .to_owned();
    let items = panel
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let field = |item: &Value, key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut open = true;
    egui::Window::new(&title)
        .id(Id::unique(("plugin panel", plugin)))
        .open(&mut open)
        .collapsible(true)
        .resizable(true)
        .default_width(280.0)
        // On the right, clear of the menus and tools.
        .default_pos(egui::pos2(ctx.content_rect().right() - 320.0, 110.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            let mut event: Option<Value> = None;
            for item in &items {
                let id = field(item, "id");
                match item.get("type").and_then(Value::as_str).unwrap_or("text") {
                    "text" => {
                        ui.label(field(item, "text"));
                    }
                    "separator" => {
                        ui.separator();
                    }
                    "input" => {
                        let label = field(item, "label");
                        if !label.is_empty() {
                            ui.label(label);
                        }
                        let value = values
                            .entry((plugin.to_owned(), id.clone()))
                            .or_insert_with(|| field(item, "value"));
                        let multiline = item
                            .get("multiline")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let edit = if multiline {
                            egui::TextEdit::multiline(value)
                        } else {
                            egui::TextEdit::singleline(value)
                        };
                        ui.add(
                            edit.hint_text(field(item, "placeholder"))
                                .desired_width(f32::INFINITY),
                        );
                    }
                    "button" => {
                        if ui.button(field(item, "label")).clicked() {
                            event = Some(json!({ "button": id }));
                        }
                    }
                    "list" => {
                        let label = field(item, "label");
                        if !label.is_empty() {
                            ui.label(label);
                        }
                        let entries = item
                            .get("items")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        egui::ScrollArea::vertical()
                            .id_salt(("plugin list", plugin, &id))
                            .max_height(200.0)
                            .show(ui, |ui| {
                                for (index, entry) in entries.iter().enumerate() {
                                    let text = entry
                                        .as_str()
                                        .map_or_else(|| entry.to_string(), str::to_owned);
                                    if ui.add(Button::new(text).frame(false)).clicked() {
                                        event = Some(json!({ "list": id, "index": index }));
                                    }
                                }
                            });
                    }
                    _ => {}
                }
            }
            if let Some(mut event) = event {
                let fields: serde_json::Map<String, Value> = values
                    .iter()
                    .filter(|((p, _), _)| p == plugin)
                    .map(|((_, k), v)| (k.clone(), Value::String(v.clone())))
                    .collect();
                event["values"] = Value::Object(fields);
                actions.push(Action::PluginPanel(plugin.to_owned(), event));
            }
        });
    if !open {
        values.retain(|(p, _), _| p != plugin);
        actions.push(Action::ClosePluginPanel(plugin.to_owned()));
    }
}

/// Plugins' messages, at the bottom of the window.
fn notices(ctx: &Context, notices: &[(String, String)]) {
    Area::new(Id::unique("plugin notices"))
        .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -72.0))
        .order(Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            for (name, text) in notices {
                panel().show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(name);
                        ui.label(text);
                    });
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_lengths_are_read_as_people_type_them() {
        assert_eq!(parse_timer("7"), Some(7 * 60_000));
        assert_eq!(parse_timer(" 1,5 "), Some(90_000));
        assert_eq!(parse_timer("2:30"), Some(150_000));
        assert_eq!(parse_timer("1:00:00"), Some(3_600_000));
        for bad in ["", "abc", "2:75", "0", "-3", "25:00:00", "1:2:3:4", "inf"] {
            assert_eq!(parse_timer(bad), None, "{bad}");
        }
    }

    #[test]
    fn panels_come_back_one_by_one_or_all_at_once() {
        let mut panels = Panels::default();
        panels.toggle(Panel::Tools);
        assert!(!panels.shows(Panel::Tools) && panels.shows(Panel::Menu));
        assert_eq!(Panels::from_words(&panels.to_words()), panels);
        // Clean, nothing shows; one panel brought back comes alone.
        panels.clean = true;
        assert!(!panels.shows(Panel::Menu));
        assert_eq!(Panels::from_words(&panels.to_words()), panels);
        panels.toggle(Panel::View);
        assert!(!panels.clean);
        assert!(panels.shows(Panel::View) && !panels.shows(Panel::Menu));
        assert_eq!(Panels::from_words(""), Panels::default());
    }

    /// Texts kept in tables are translated where they are shown; all have a translation.
    #[test]
    fn texts_in_tables_are_translated() {
        let mut texts: Vec<&str> = Vec::new();
        for (set, kinds) in SHAPE_SETS {
            texts.push(set);
            texts.extend(kinds.iter().map(|(_, name)| *name));
        }
        texts.extend(STRUCTURES.iter().map(|(_, tip, _)| *tip));
        for (topic, keys) in SHORTCUTS {
            texts.push(topic);
            for (key, what) in *keys {
                texts.push(what);
                if has_words(key) {
                    texts.push(key);
                }
            }
        }
        texts.extend(
            palette_commands(PaletteCan::default())
                .into_iter()
                .map(|(label, ..)| label),
        );
        for template in &crate::templates::TEMPLATES {
            texts.push(template.name);
            texts.push(template.description);
        }
        for grid in [GridStyle::Dots, GridStyle::Lines, GridStyle::None] {
            texts.push(grid.name());
        }
        let missing: Vec<_> = texts
            .into_iter()
            .filter(|t| !crate::i18n::translated(t))
            .collect();
        assert!(missing.is_empty(), "no translation for {missing:#?}");
    }
}
