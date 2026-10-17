//! Canvas tools: select, pen, shapes, connectors, ...
//!
//! Tools turn pointer input (in world coordinates) into [`Command`]s and previews. They only
//! read the document, so the same input logic will drive local and remote edits.

mod arrange;
pub mod bend;
mod clipboard;
mod connect;
mod crop;
mod handles;
mod hit;
mod ink;
pub mod math;
pub mod quick;
mod selection;
mod table;

pub use arrange::{
    Align, Axis, ClusterBy, ClusterLabel, align, arrange_grid, cluster, color_name, distribute,
};
pub use clipboard::{Clipboard, duplicate};
pub use connect::Which;
pub use crop::Crop;
pub use handles::{Handle, rotation_knob};
pub use ink::{ERASER_RADIUS, InkStyle};
pub use selection::{Selection, selection_frame, unit_frame};

use bend::BendGrip;
use pb_core::{
    Cap, CellRange, Column, Command, Connector, ConnectorChange, Content, Document, End, FrameData,
    Kanban, KanbanPart, Label, MindMap, MindMapPart, Object, ObjectData, ObjectId, ObjectKind,
    Route, ShapeKind, Style, Table,
};
use pb_geom::kurbo::{Line, Point, Rect, Vec2};
use pb_geom::{Frame, GridLevel, Snapper, xs_of, ys_of};
use table::Divider;

/// Drags shorter than this on screen (logical pixels) count as clicks.
const CLICK_SLOP: f64 = 4.0;
/// Extra hit area around objects, in logical pixels.
const HIT_SLOP: f64 = 3.0;
/// Hit radius of resize and rotation handles, in logical pixels.
const HANDLE_HIT_RADIUS: f64 = 7.0;
/// Dragged edges snap to other objects within this distance, in logical pixels.
const SNAP_DISTANCE: f64 = 6.0;
/// Rotation snaps to multiples of this with Shift held.
const ROTATION_SNAP: f64 = std::f64::consts::PI / 12.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolKind {
    #[default]
    Select,
    Shape(ShapeKind),
    Pen,
    /// A wide, see-through pen, like a highlighter.
    Marker,
    /// Removes whole ink strokes it touches.
    Eraser,
    /// Click to type. The app runs the text editor; the tool only marks the mode.
    Text,
    /// Click to place a sticky note and type on it; run by the app like [`Self::Text`].
    Sticky,
    /// Drag from an object (or anywhere) to another to connect them.
    Connector,
    /// Drag out a frame, or click to place one.
    Frame,
    /// Drag out a table, or click to place a 3 x 3 one.
    Table,
    /// Drag out a kanban board, or click to place one with three columns.
    Kanban,
    /// Click to place a mind map and type its central topic; run by the app like
    /// [`Self::Text`].
    MindMap,
    /// Click to place a Markdown document and type it; run by the app like [`Self::Text`].
    Doc,
    /// Click an object or the board to start a comment thread there; run by the app.
    Comment,
}

impl ToolKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Shape(shape) => shape.name(),
            Self::Pen => "Pen",
            Self::Marker => "Marker",
            Self::Eraser => "Eraser",
            Self::Text => "Text",
            Self::Sticky => "Sticky note",
            Self::Connector => "Connector",
            Self::Frame => "Frame",
            Self::Table => "Table",
            Self::Kanban => "Kanban",
            Self::MindMap => "Mind map",
            Self::Doc => "Document",
            Self::Comment => "Comment",
        }
    }

    /// Pen or marker. Like Miro, these stay active after a stroke, and strokes are not
    /// selected.
    pub fn draws_ink(self) -> bool {
        matches!(self, Self::Pen | Self::Marker)
    }
}

/// Platform-neutral mouse cursor the tools ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorHint {
    Default,
    Crosshair,
    Move,
    Copy,
    Rotate,
    ResizeNs,
    ResizeEw,
    ResizeNwse,
    ResizeNesw,
}

#[derive(Clone, Copy, Debug)]
pub struct Pointer {
    pub world: Point,
    /// Current camera zoom, to measure distances in screen pixels.
    pub zoom: f64,
    /// The part of the world on screen; only visible objects are snapped to.
    pub visible: Rect,
    /// Pen pressure in `[0, 1]`; 1 for a mouse.
    pub pressure: f64,
    pub shift: bool,
    /// Held during a drag, turns snapping off.
    pub ctrl: bool,
    pub alt: bool,
}

impl Pointer {
    /// Shift or Ctrl add to / remove from the selection.
    fn toggles_selection(&self) -> bool {
        self.shift || self.ctrl
    }

    fn screen_distance(&self, from: Point) -> f64 {
        (self.world - from).hypot() * self.zoom
    }
}

/// A grabbable part of the selection frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grip {
    Resize(Handle),
    Rotate,
    /// An end of the selected connector.
    End(Which),
    /// The label of the selected connector.
    Label,
    /// A bend handle of the selected connector.
    Bend(BendGrip),
    /// A line between two rows or columns of the selected table.
    Divider(Divider),
}

/// What a drawing gesture makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drawn {
    Shape(ShapeKind),
    Frame,
    Table,
    Kanban,
}

enum Gesture {
    Shape {
        kind: Drawn,
        start: Point,
        /// `start` after snapping: the corner the shape grows from.
        anchor: Point,
        current: Pointer,
    },
    Move {
        start: Point,
        current: Pointer,
        /// Set once the pointer leaves the click slop; until then nothing moves.
        dragging: bool,
        /// Alt was held at the start: drop copies and leave the originals in place.
        duplicate: bool,
        /// Clicking (not dragging) an object of a multi-selection selects just that object.
        select_only_on_click: Option<ObjectId>,
        /// Clicking (not dragging) the selected table selects this cell of it.
        cell_on_click: Option<(usize, usize)>,
    },
    /// Dragging over the cells of the selected table to select a block of them.
    SelectCells,
    /// Pressed on the collapse toggle of this node of the selected mind map.
    ToggleNode(usize),
    /// Pressed on card `card` of the selected kanban board `id`: a click selects it, a drag
    /// moves it to where its middle is dropped.
    MoveCard {
        id: ObjectId,
        card: std::sync::Arc<str>,
        start: Point,
        current: Point,
        /// From the pointer to the card's middle.
        offset: Vec2,
        dragging: bool,
    },
    Resize {
        handle: Handle,
        from: Frame,
        start: Point,
        current: Pointer,
    },
    Rotate {
        from: Frame,
        start: Point,
        current: Pointer,
    },
    Marquee {
        start: Point,
        current: Point,
        /// Selection before the marquee started (kept when adding with Shift/Ctrl).
        base: Vec<ObjectId>,
    },
    Ink {
        marker: bool,
        /// World points with pressure.
        points: Vec<(Point, f64)>,
        /// Zoom when the stroke started; its width and smoothing follow it.
        zoom: f64,
    },
    Erase {
        last: Point,
        /// Strokes touched so far; hidden until the eraser is released.
        erased: Vec<ObjectId>,
    },
    /// Drawing a new connector.
    Connect {
        start: End,
        end: End,
        current: Pointer,
    },
    /// Dragging an end of a selected connector to another place or object.
    MoveEnd {
        id: ObjectId,
        which: Which,
        connector: Connector,
        end: End,
        current: Pointer,
    },
    /// Sliding the label of a selected connector along its line.
    /// Dragging a line between rows or columns of table `id`; `lines` is the result so far.
    MoveDivider {
        id: ObjectId,
        divider: Divider,
        start: Point,
        lines: Option<std::sync::Arc<[pb_core::Line]>>,
    },
    /// Dragging a bend handle of connector `id`; `bends` is the result so far.
    MoveBend {
        id: ObjectId,
        grip: BendGrip,
        start: Point,
        bends: Option<Vec<Point>>,
    },
    MoveLabel {
        id: ObjectId,
        label: Label,
        start: Point,
        /// From the pointer to the label's middle, kept while dragging.
        offset: Vec2,
        dragging: bool,
    },
}

pub struct Tools {
    active: ToolKind,
    selection: Selection,
    gesture: Option<Gesture>,
    hover: Option<ObjectId>,
    hover_grip: Option<Grip>,
    /// Rotation of the frame owning `hover_grip`, to orient its resize cursor.
    hover_rotation: f64,
    snap_to_grid: bool,
    /// Shape the shape tool starts with.
    last_shape: ShapeKind,
    pen: InkStyle,
    marker: InkStyle,
    /// Where the eraser hovers, to show its size.
    eraser_at: Option<(Point, f64)>,
    /// Route of new connectors.
    connector_route: Route,
    /// Where a connector would start, while the connector tool hovers.
    connect_hover: Option<End>,
    /// Selected cells of the selected table.
    cells: Option<CellSelection>,
    /// The selected card of the selected kanban board: the board and the card's id.
    card: Option<(ObjectId, std::sync::Arc<str>)>,
    /// The selected node of the selected mind map: the map and the node's id.
    node: Option<(ObjectId, std::sync::Arc<str>)>,
}

/// A block of cells of table `id`, from the cell first picked to the one picked last.
#[derive(Clone, Copy, Debug)]
struct CellSelection {
    id: ObjectId,
    anchor: (usize, usize),
    end: (usize, usize),
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            active: ToolKind::default(),
            selection: Selection::default(),
            gesture: None,
            hover: None,
            hover_grip: None,
            hover_rotation: 0.0,
            snap_to_grid: false,
            last_shape: ShapeKind::Rectangle,
            pen: InkStyle::PEN,
            marker: InkStyle::MARKER,
            eraser_at: None,
            connector_route: Route::default(),
            connect_hover: None,
            cells: None,
            card: None,
            node: None,
        }
    }
}

impl Tools {
    pub fn active(&self) -> ToolKind {
        self.active
    }

    pub fn set_active(&mut self, tool: ToolKind) {
        if let ToolKind::Shape(shape) = tool {
            self.last_shape = shape;
        }
        self.active = tool;
        self.gesture = None;
        self.hover = None;
        self.hover_grip = None;
        self.eraser_at = None;
        self.connect_hover = None;
    }

    /// Route of new connectors.
    pub fn connector_route(&self) -> Route {
        self.connector_route
    }

    pub fn set_connector_route(&mut self, route: Route) {
        self.connector_route = route;
    }

    /// Color and width of the pen, or of the marker.
    pub fn ink_style(&self, marker: bool) -> InkStyle {
        if marker { self.marker } else { self.pen }
    }

    pub fn set_ink_style(&mut self, marker: bool, style: InkStyle) {
        if marker {
            self.marker = style;
        } else {
            self.pen = style;
        }
    }

    /// The shape the shape tool starts with.
    pub fn last_shape(&self) -> ShapeKind {
        self.last_shape
    }

    /// Whether a drag (move, resize, marquee, drawing) is in progress.
    pub fn is_busy(&self) -> bool {
        self.gesture.is_some()
    }

    /// Forgets the hovered object, e.g. when the pointer moves onto the UI.
    pub fn clear_hover(&mut self) -> bool {
        let had = self.hover.is_some() || self.hover_grip.is_some();
        self.hover = None;
        self.hover_grip = None;
        had
    }

    /// Picks the shape tool: the next kind if it is already active, else the last one used.
    pub fn cycle_shape(&mut self) {
        let shape = match self.active {
            ToolKind::Shape(shape) => shape.next(),
            _ => self.last_shape,
        };
        self.set_active(ToolKind::Shape(shape));
    }

    pub fn snap_to_grid(&self) -> bool {
        self.snap_to_grid
    }

    pub fn set_snap_to_grid(&mut self, on: bool) {
        self.snap_to_grid = on;
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn select_only(&mut self, ids: impl IntoIterator<Item = ObjectId>) {
        self.selection.set(ids);
        self.cells = None;
        self.card = None;
        self.node = None;
    }

    /// Selects mind map `id` and its node `node`.
    pub fn select_node(&mut self, id: ObjectId, node: std::sync::Arc<str>) {
        self.select_only([id]);
        self.node = Some((id, node));
    }

    /// The selected node: its map and index, if a node of the single selected map is
    /// selected and shown.
    pub fn selected_node(&self, doc: &Document) -> Option<(ObjectId, usize)> {
        let (id, node) = self.node.as_ref()?;
        let (map_id, _, map) = self.single_mindmap(doc)?;
        let i = map.index(node).filter(|_| map_id == *id)?;
        map.layout().find(i).map(|_| (map_id, i))
    }

    /// Where the selected node is (world).
    fn node_frame(&self, doc: &Document) -> Option<Frame> {
        let (_, i) = self.selected_node(doc)?;
        let (_, data, map) = self.single_mindmap(doc)?;
        let rect = map.node_rect(data.bounds, i)?;
        Some(Frame::axis_aligned(rect).rotated_about(data.bounds.center(), data.rotation))
    }

    /// The single selected unlocked mind map, with its data.
    fn single_mindmap<'a>(
        &self,
        doc: &'a Document,
    ) -> Option<(ObjectId, &'a ObjectData, &'a MindMap)> {
        let [id] = self.selection.ids() else {
            return None;
        };
        let object = doc.get(*id).filter(|_| !doc.is_locked(*id))?;
        match &object.data.kind {
            ObjectKind::MindMap(map) => Some((*id, &object.data, map)),
            _ => None,
        }
    }

    /// The selected node's map and the node: whether it has children and is collapsed.
    pub fn node_state(&self, doc: &Document) -> Option<(bool, bool)> {
        let (_, i) = self.selected_node(doc)?;
        let (_, _, map) = self.single_mindmap(doc)?;
        Some((!map.children(i).is_empty(), map.nodes[i].collapsed))
    }

    /// Collapses or expands node `i` of the selected mind map, or the selected node.
    pub fn toggle_node(&self, doc: &Document, i: Option<usize>) -> Option<Command> {
        let (id, data, map) = self.single_mindmap(doc)?;
        let i = match i {
            Some(i) => i,
            None => self.selected_node(doc)?.1,
        };
        if map.children(i).is_empty() {
            return None;
        }
        let node = pb_core::Node {
            collapsed: !map.nodes[i].collapsed,
            ..map.nodes[i].clone()
        };
        let bounds = map.with_node(node.clone()).refit(map, data.bounds);
        Some(Command::SetNode { id, node, bounds })
    }

    /// Removes the selected node and everything under it, and selects its parent; the
    /// central topic takes the whole map with it.
    pub fn remove_selected_node(&mut self, doc: &Document) -> Option<Command> {
        let (id, i) = self.selected_node(doc)?;
        let (_, data, map) = self.single_mindmap(doc)?;
        let Some(parent) = map.nodes[i].parent.clone() else {
            self.node = None;
            return Some(Command::Delete(vec![id]));
        };
        let gone: Vec<_> = map
            .subtree(i)
            .iter()
            .map(|&n| map.nodes[n].id.clone())
            .collect();
        let bounds = map.without(&gone).refit(map, data.bounds);
        self.node = Some((id, parent));
        Some(Command::RemoveNodes {
            id,
            nodes: gone,
            bounds,
        })
    }

    /// Selects kanban board `id` and its card `card`.
    pub fn select_card(&mut self, id: ObjectId, card: std::sync::Arc<str>) {
        self.select_only([id]);
        self.card = Some((id, card));
    }

    /// The selected card: its board and index, if a card of the single selected board is
    /// selected and shown.
    pub fn selected_card(&self, doc: &Document) -> Option<(ObjectId, usize)> {
        let (id, card) = self.card.as_ref()?;
        let (board, _, kanban) = self.single_kanban(doc)?;
        let i = kanban.card_index(card).filter(|_| board == *id)?;
        kanban.place_of(i).map(|_| (board, i))
    }

    /// Where the selected card is (world).
    fn card_frame(&self, doc: &Document) -> Option<Frame> {
        let (_, i) = self.selected_card(doc)?;
        let (_, data, kanban) = self.single_kanban(doc)?;
        let rect = kanban.card_rect(data.bounds, i)?;
        Some(Frame::axis_aligned(rect).rotated_about(data.bounds.center(), data.rotation))
    }

    /// Selects table `id` and its cell `cell`.
    pub fn select_cell(&mut self, id: ObjectId, cell: (usize, usize)) {
        self.selection.set([id]);
        self.cells = Some(CellSelection {
            id,
            anchor: cell,
            end: cell,
        });
    }

    /// The selected cells, if a block of cells of the single selected table is selected.
    pub fn table_cells(&self, doc: &Document) -> Option<(ObjectId, CellRange)> {
        let sel = self.cells?;
        let (id, _, table) = self.single_table(doc).filter(|(id, ..)| *id == sel.id)?;
        let range = CellRange::new(sel.anchor, sel.end);
        (range.row1 < table.rows.len() && range.col1 < table.cols.len())
            .then(|| (id, table.expand(range)))
    }

    /// Where the selected cells or card are (world), following the object's rotation.
    pub fn cells_frame(&self, doc: &Document) -> Option<Frame> {
        if let Some(frame) = self.card_frame(doc).or_else(|| self.node_frame(doc)) {
            return Some(frame);
        }
        let (_, range) = self.table_cells(doc)?;
        let (_, data, table) = self.single_table(doc)?;
        let rect = table.range_rect(data.bounds, range)?;
        Some(Frame::axis_aligned(rect).rotated_about(data.bounds.center(), data.rotation))
    }

    /// Deselects the cells or card but keeps their table or board selected. Returns true if
    /// any were selected.
    pub fn clear_cells(&mut self) -> bool {
        self.cells.take().is_some() | self.card.take().is_some() | self.node.take().is_some()
    }

    /// Selects everything that is not locked.
    pub fn select_all(&mut self, doc: &Document) {
        self.set_active(ToolKind::Select);
        let ids = doc
            .roots()
            .iter()
            .filter(|&&id| !doc.is_locked(id) && doc.reachable(id));
        self.selection.set(ids.copied());
        self.cells = None;
        self.card = None;
        self.node = None;
    }

    /// Returns true if something was selected.
    pub fn clear_selection(&mut self) -> bool {
        let had = !self.selection.is_empty();
        self.selection.clear();
        self.cells = None;
        self.card = None;
        self.node = None;
        had
    }

    /// Call after every document change: objects may have disappeared or been grouped.
    pub fn sync(&mut self, doc: &Document) {
        self.selection.retain_existing(doc);
        if self.cells.is_some() && self.table_cells(doc).is_none() {
            self.cells = None;
        }
        if self.card.is_some() && self.selected_card(doc).is_none() {
            self.card = None;
        }
        if self.node.is_some() && self.selected_node(doc).is_none() {
            self.node = None;
        }
        if self.hover.is_some_and(|id| !doc.contains(id)) {
            self.hover = None;
        }
    }

    /// Abandons the current gesture. Returns true if there was one.
    pub fn cancel(&mut self) -> bool {
        match self.gesture.take() {
            Some(Gesture::Marquee { base, .. }) => {
                self.selection.set(base);
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    /// Deletes the selection, except locked objects.
    pub fn delete_selected(&self, doc: &Document) -> Option<Command> {
        let ids = self.unlocked(doc);
        (!ids.is_empty()).then_some(Command::Delete(ids))
    }

    pub fn nudge_selected(&self, doc: &Document, delta: Vec2) -> Option<Command> {
        let ids = self.moving(doc);
        (!ids.is_empty() && self.gesture.is_none()).then_some(Command::Move { ids, delta })
    }

    /// Groups the selection (Ctrl+G). Needs two or more objects, none of them locked.
    pub fn group_selected(&self, doc: &Document) -> Option<Command> {
        let ids = self.selection.ids();
        (ids.len() > 1 && !self.selection.has_locked(doc)).then(|| Command::Group(ids.to_vec()))
    }

    /// Dissolves the selected groups that are not locked (Ctrl+Shift+G).
    pub fn ungroup_selected(&self, doc: &Document) -> Option<Command> {
        let ids: Vec<_> = self
            .unlocked(doc)
            .into_iter()
            .filter(|&id| doc.group(id).is_some())
            .collect();
        (!ids.is_empty()).then_some(Command::Ungroup(ids))
    }

    /// Locks the selection, or unlocks it if everything in it is locked (Ctrl+Shift+L).
    pub fn toggle_lock_selected(&self, doc: &Document) -> Option<Command> {
        let ids = self.selection.ids();
        let locked = !ids.iter().all(|&id| doc.is_locked(id));
        (!ids.is_empty()).then(|| Command::SetLocked {
            ids: ids.to_vec(),
            locked,
        })
    }

    /// Hides the selection, or shows it again if everything in it is hidden (Ctrl+Shift+H).
    pub fn toggle_hidden_selected(&self, doc: &Document) -> Option<Command> {
        let ids = self.selection.ids();
        let hidden = !ids.iter().all(|&id| doc.is_hidden(id));
        (!ids.is_empty()).then(|| Command::SetHidden {
            ids: ids.to_vec(),
            hidden,
        })
    }

    /// Returns true if the view needs a redraw.
    pub fn pointer_down(&mut self, doc: &Document, pointer: Pointer) -> bool {
        let p = pointer.world;
        match self.active {
            ToolKind::Pen | ToolKind::Marker => {
                let marker = self.active == ToolKind::Marker;
                // A highlighter has an even width.
                let pressure = if marker { 1.0 } else { pointer.pressure };
                self.gesture = Some(Gesture::Ink {
                    marker,
                    points: vec![(p, pressure)],
                    zoom: pointer.zoom,
                });
                return true;
            }
            // Text editing is handled by the app.
            ToolKind::Text
            | ToolKind::Sticky
            | ToolKind::MindMap
            | ToolKind::Doc
            | ToolKind::Comment => {
                return false;
            }
            ToolKind::Connector => {
                let start = connect::end_at(doc, &pointer, None);
                self.gesture = Some(Gesture::Connect {
                    start,
                    end: End::Free(p),
                    current: pointer,
                });
                self.connect_hover = None;
                return true;
            }
            ToolKind::Eraser => {
                let radius = ERASER_RADIUS / pointer.zoom;
                self.gesture = Some(Gesture::Erase {
                    last: p,
                    erased: ink::erased(doc, p, p, radius),
                });
                return true;
            }
            _ => {}
        }
        let drawn = match self.active {
            ToolKind::Shape(shape) => Some(Drawn::Shape(shape)),
            ToolKind::Frame => Some(Drawn::Frame),
            ToolKind::Table => Some(Drawn::Table),
            ToolKind::Kanban => Some(Drawn::Kanban),
            _ => None,
        };
        if let Some(kind) = drawn {
            self.gesture = Some(Gesture::Shape {
                kind,
                start: p,
                anchor: self.snap_point(doc, pointer).0,
                current: pointer,
            });
            return true;
        }

        if let Some((from, grip)) = self.grip_at(doc, pointer) {
            self.gesture = Some(match grip {
                Grip::Resize(handle) => Gesture::Resize {
                    handle,
                    from,
                    start: p,
                    current: pointer,
                },
                Grip::Rotate => Gesture::Rotate {
                    from,
                    start: p,
                    current: pointer,
                },
                Grip::End(which) => {
                    let Some((id, connector)) = self.single_connector(doc) else {
                        return false;
                    };
                    let end = match which {
                        Which::Start => connector.start,
                        Which::End => connector.end,
                    };
                    Gesture::MoveEnd {
                        id,
                        which,
                        connector,
                        end,
                        current: pointer,
                    }
                }
                Grip::Divider(divider) => {
                    let Some((id, _, _)) = self.single_table(doc) else {
                        return false;
                    };
                    Gesture::MoveDivider {
                        id,
                        divider,
                        start: p,
                        lines: None,
                    }
                }
                Grip::Bend(grip) => {
                    let Some((id, _)) = self.single_connector(doc) else {
                        return false;
                    };
                    Gesture::MoveBend {
                        id,
                        grip,
                        start: p,
                        bends: None,
                    }
                }
                Grip::Label => {
                    let Some((id, connector)) = self.single_connector(doc) else {
                        return false;
                    };
                    let Some(rect) = doc.connector_label(id) else {
                        return false;
                    };
                    Gesture::MoveLabel {
                        id,
                        label: connector.label,
                        start: p,
                        offset: rect.center() - p,
                        dragging: false,
                    }
                }
            });
            return true;
        }

        let toggle = pointer.toggles_selection();
        let under = self.object_at(doc, pointer);
        let cell = self
            .single_table(doc)
            .filter(|(id, ..)| under == Some(*id))
            .and_then(|(id, data, table)| {
                let local = data.frame().to_local(p);
                Some((id, table.cell_at(data.bounds, local)?))
            });
        let card = self
            .single_kanban(doc)
            .filter(|(id, ..)| under == Some(*id) && !toggle)
            .and_then(|(id, data, kanban)| {
                let local = data.frame().to_local(p);
                match kanban.part_at(data.bounds, local)? {
                    KanbanPart::Card(i) => {
                        let middle = data
                            .frame()
                            .to_world(kanban.card_rect(data.bounds, i)?.center());
                        Some((id, kanban.cards[i].id.clone(), middle - p))
                    }
                    _ => None,
                }
            });
        let part = self
            .single_mindmap(doc)
            .filter(|(id, ..)| under == Some(*id) && !toggle)
            .and_then(|(id, data, map)| {
                let local = data.frame().to_local(p);
                Some((id, map.part_at(data.bounds, local)?))
            });
        match part {
            Some((_, MindMapPart::Toggle(i))) => {
                self.gesture = Some(Gesture::ToggleNode(i));
                self.hover = None;
                return true;
            }
            Some((id, MindMapPart::Node(i))) => {
                if let Some((_, _, map)) = self.single_mindmap(doc) {
                    self.node = Some((id, map.nodes[i].id.clone()));
                }
            }
            None => self.node = None,
        }
        if let Some((id, card, offset)) = card {
            self.cells = None;
            self.gesture = Some(Gesture::MoveCard {
                id,
                card,
                start: p,
                current: p,
                offset,
                dragging: false,
            });
            self.hover = None;
            return true;
        }
        self.card = None;
        let mut cell_on_click = None;
        if let Some((id, cell)) = cell {
            let picking = self.cells.is_some_and(|c| c.id == id);
            if picking || pointer.shift {
                // With cells selected, a drag selects cells; Shift extends the block.
                let anchor = match self.cells {
                    Some(c) if c.id == id && pointer.shift => c.anchor,
                    _ => cell,
                };
                self.cells = Some(CellSelection {
                    id,
                    anchor,
                    end: cell,
                });
                self.gesture = Some(Gesture::SelectCells);
                self.hover = None;
                return true;
            }
            if !toggle {
                cell_on_click = Some(cell);
            }
        }
        self.cells = None;
        match under {
            Some(id) => {
                let mut select_only_on_click = None;
                if toggle && self.selection.contains(id) {
                    self.selection.remove(id);
                    return true;
                } else if doc.is_locked(id) && !toggle {
                    // Locked objects can be selected (to unlock or copy them), not dragged.
                    self.selection.set([id]);
                    self.hover = None;
                    return true;
                } else if toggle {
                    self.selection.add(id);
                } else if self.selection.contains(id) {
                    select_only_on_click = Some(id);
                } else {
                    self.selection.set([id]);
                }
                self.gesture = Some(Gesture::Move {
                    start: p,
                    current: pointer,
                    dragging: false,
                    duplicate: pointer.alt,
                    select_only_on_click,
                    cell_on_click,
                });
            }
            None => {
                if !toggle {
                    self.selection.clear();
                }
                self.gesture = Some(Gesture::Marquee {
                    start: p,
                    current: p,
                    base: self.selection.ids().to_vec(),
                });
            }
        }
        self.hover = None;
        true
    }

    /// Returns true if the view needs a redraw.
    pub fn pointer_move(&mut self, doc: &Document, pointer: Pointer) -> bool {
        match &mut self.gesture {
            Some(Gesture::Shape { current, .. })
            | Some(Gesture::Resize { current, .. })
            | Some(Gesture::Rotate { current, .. }) => {
                *current = pointer;
                true
            }
            Some(Gesture::Move {
                start,
                current,
                dragging,
                ..
            }) => {
                *current = pointer;
                *dragging |= pointer.screen_distance(*start) >= CLICK_SLOP;
                *dragging
            }
            Some(Gesture::MoveCard {
                start,
                current,
                dragging,
                ..
            }) => {
                *current = pointer.world;
                *dragging |= pointer.screen_distance(*start) >= CLICK_SLOP;
                *dragging
            }
            Some(Gesture::ToggleNode(_)) => false,
            Some(Gesture::SelectCells) => {
                let Some((id, data, table)) = self.single_table(doc) else {
                    return false;
                };
                let b = data.bounds;
                let local = data.frame().to_local(pointer.world);
                // Past the table's edge, the nearest cell.
                let inside = Point::new(local.x.clamp(b.x0, b.x1), local.y.clamp(b.y0, b.y1));
                match (&mut self.cells, table.cell_at(b, inside)) {
                    (Some(sel), Some(cell)) if sel.id == id && sel.end != cell => {
                        sel.end = cell;
                        true
                    }
                    _ => false,
                }
            }
            Some(Gesture::Marquee {
                start,
                current,
                base,
            }) => {
                *current = pointer.world;
                let area = Rect::from_points(*start, *current);
                let touched = hit::objects_in(doc, area);
                let ids: Vec<_> = base.iter().copied().chain(touched).collect();
                self.selection.set(ids);
                true
            }
            Some(Gesture::Ink { marker, points, .. }) => {
                let last = points.last().map(|(p, _)| *p);
                if last.is_some_and(|last| pointer.screen_distance(last) < ink::MIN_STEP) {
                    return false;
                }
                let pressure = if *marker { 1.0 } else { pointer.pressure };
                points.push((pointer.world, pressure));
                true
            }
            Some(Gesture::Erase { last, erased }) => {
                let radius = ERASER_RADIUS / pointer.zoom;
                for id in ink::erased(doc, *last, pointer.world, radius) {
                    if !erased.contains(&id) {
                        erased.push(id);
                    }
                }
                *last = pointer.world;
                self.eraser_at = Some((pointer.world, radius));
                true
            }
            Some(Gesture::Connect {
                start,
                end,
                current,
            }) => {
                *current = pointer;
                *end = connect::end_at(doc, &pointer, start.object());
                true
            }
            Some(Gesture::MoveEnd {
                which,
                connector,
                end,
                current,
                ..
            }) => {
                *current = pointer;
                let other = match which {
                    Which::Start => connector.end,
                    Which::End => connector.start,
                };
                *end = connect::end_at(doc, &pointer, other.object());
                true
            }
            Some(Gesture::MoveDivider {
                id,
                divider,
                start,
                lines,
            }) => {
                let Some(object) = doc.get(*id) else {
                    return false;
                };
                let ObjectKind::Table(t) = &object.data.kind else {
                    return false;
                };
                // Along the table's own axes, so rotated tables work too.
                let frame = object.data.frame();
                let moved = frame.to_local(pointer.world) - frame.to_local(*start);
                let delta = if divider.rows { moved.y } else { moved.x };
                *lines = Some(table::resized(&object.data, t, *divider, delta));
                true
            }
            Some(Gesture::MoveBend {
                id,
                grip,
                start,
                bends,
            }) => {
                if bends.is_none() && pointer.screen_distance(*start) < CLICK_SLOP {
                    return false;
                }
                *bends = bend::dragged(doc, *id, *grip, pointer.world - *start);
                true
            }
            Some(Gesture::MoveLabel {
                id,
                label,
                start,
                offset,
                dragging,
            }) => {
                *dragging |= pointer.screen_distance(*start) >= CLICK_SLOP;
                if !*dragging {
                    return false;
                }
                if let Some(path) = doc.connector_path(*id) {
                    label.at = path.nearest_fraction(pointer.world + *offset);
                }
                true
            }
            None if self.active == ToolKind::Eraser => {
                self.eraser_at = Some((pointer.world, ERASER_RADIUS / pointer.zoom));
                true
            }
            None if self.active == ToolKind::Connector => {
                let hover = Some(connect::end_at(doc, &pointer, None));
                let changed = hover != self.connect_hover;
                self.connect_hover = hover;
                changed
            }
            None => self.update_hover(doc, pointer),
        }
    }

    pub fn pointer_up(&mut self, doc: &Document, pointer: Pointer) -> Option<Command> {
        self.pointer_move(doc, pointer);
        let command = match self.gesture.as_ref()? {
            Gesture::Shape { kind, .. } => {
                let kind = *kind;
                // Like Miro, a shape tool is one-shot: go back to selecting afterwards.
                self.active = ToolKind::Select;
                let (bounds, _) = self.shape_bounds(doc, true);
                Some(Command::Create(vec![drawn(doc, kind, bounds).into()]))
            }
            Gesture::Ink {
                marker,
                points,
                zoom,
            } => ink::stroke(points, self.ink_style(*marker), *marker, *zoom, true)
                .map(|data| Command::Create(vec![data.into()])),
            Gesture::Erase { erased, .. } => {
                (!erased.is_empty()).then(|| Command::Delete(erased.clone()))
            }
            Gesture::Move {
                dragging: true,
                duplicate: true,
                ..
            } => Some(Command::Create(self.moved_copies(doc))),
            Gesture::Move { dragging: true, .. } => {
                let ids = self.moving(doc);
                (!ids.is_empty()).then(|| Command::Move {
                    ids,
                    delta: self.move_delta(doc).0,
                })
            }
            Gesture::Move {
                select_only_on_click,
                cell_on_click,
                ..
            } => {
                if let Some(id) = *select_only_on_click {
                    self.selection.set([id]);
                    if let Some(cell) = *cell_on_click {
                        self.select_cell(id, cell);
                    }
                }
                None
            }
            Gesture::SelectCells => None,
            Gesture::ToggleNode(i) => self.toggle_node(doc, Some(*i)),
            Gesture::MoveCard {
                id,
                card,
                dragging: false,
                ..
            } => {
                self.card = Some((*id, card.clone()));
                None
            }
            Gesture::MoveCard { .. } => {
                let moved = self.moved_card(doc);
                if let (Some((id, card, bounds)), Some(Gesture::MoveCard { card: name, .. })) =
                    (&moved, &self.gesture)
                {
                    self.card = Some((*id, name.clone()));
                    Some(Command::SetCard {
                        id: *id,
                        card: card.clone(),
                        bounds: *bounds,
                    })
                } else {
                    None
                }
            }
            Gesture::Resize { .. } | Gesture::Rotate { .. } => {
                let changes = self.geometry_changes(doc);
                let changed = changes
                    .iter()
                    .any(|(id, f)| doc.get(*id).is_some_and(|o| o.data.frame() != *f));
                changed.then_some(Command::SetGeometry(changes))
            }
            Gesture::Marquee { .. } => None,
            Gesture::Connect { start, end, .. } => {
                self.active = ToolKind::Select;
                // A click, not a drag, between two free points draws nothing.
                let tiny = start.object().is_none()
                    && end.object().is_none()
                    && pointer.screen_distance(start.point()) < CLICK_SLOP;
                (!tiny).then(|| {
                    Command::Create(vec![connector(*start, *end, self.connector_route).into()])
                })
            }
            Gesture::MoveEnd {
                id,
                which,
                connector,
                end,
                ..
            } => {
                let old = match which {
                    Which::Start => connector.start,
                    Which::End => connector.end,
                };
                let change = match which {
                    Which::Start => ConnectorChange {
                        start: Some(*end),
                        ..Default::default()
                    },
                    Which::End => ConnectorChange {
                        end: Some(*end),
                        ..Default::default()
                    },
                };
                (*end != old).then(|| Command::SetConnector {
                    ids: vec![*id],
                    change,
                })
            }
            Gesture::MoveLabel {
                id,
                label,
                dragging: true,
                ..
            } => Some(Command::SetLabel {
                id: *id,
                label: label.clone(),
            }),
            Gesture::MoveLabel { .. } => None,
            Gesture::MoveBend {
                id,
                bends: Some(bends),
                ..
            } => Some(Command::SetBends {
                id: *id,
                bends: bends.clone().into(),
            }),
            Gesture::MoveBend { .. } => None,
            Gesture::MoveDivider {
                id,
                divider,
                lines: Some(lines),
                ..
            } => match doc.get(*id).map(|o| (&o.data.kind, o.data.bounds)) {
                Some((ObjectKind::Table(t), bounds)) => {
                    let (rows, cols) = if divider.rows {
                        (lines.clone(), t.cols.clone())
                    } else {
                        (t.rows.clone(), lines.clone())
                    };
                    let changed = rows != t.rows || cols != t.cols;
                    changed.then_some(Command::SetTable {
                        id: *id,
                        rows,
                        cols,
                        merges: t.merges.clone(),
                        bounds,
                    })
                }
                _ => None,
            },
            Gesture::MoveDivider { .. } => None,
        };
        self.gesture = None;
        command
    }

    /// The selected connector's end handles, when a single unlocked connector is selected
    /// and nothing else is going on.
    pub fn connector_handles(&self, doc: &Document) -> Option<[Point; 2]> {
        if self.active != ToolKind::Select || self.gesture.is_some() {
            return None;
        }
        let (id, _) = self.single_connector(doc)?;
        let (s, e) = doc.connector_ends(id)?;
        Some([s.point, e.point])
    }

    /// Bend handles of the selected connector, when its end handles show.
    pub fn bend_handles(&self, doc: &Document) -> Vec<Point> {
        if self.connector_handles(doc).is_none() {
            return Vec::new();
        }
        self.single_connector(doc)
            .map(|(id, _)| bend::handles(doc, id).into_iter().map(|(p, _)| p).collect())
            .unwrap_or_default()
    }

    /// The single selected unlocked kanban board, with its data.
    fn single_kanban<'a>(
        &self,
        doc: &'a Document,
    ) -> Option<(ObjectId, &'a ObjectData, &'a Kanban)> {
        let [id] = self.selection.ids() else {
            return None;
        };
        let object = doc.get(*id).filter(|_| !doc.is_locked(*id))?;
        match &object.data.kind {
            ObjectKind::Kanban(kanban) => Some((*id, &object.data, kanban)),
            _ => None,
        }
    }

    /// Where the card being dragged lands: its board, the card in its new place, and the
    /// board's bounds then; `None` while it would stay where it is.
    fn moved_card(&self, doc: &Document) -> Option<(ObjectId, pb_core::Card, Rect)> {
        let Some(Gesture::MoveCard {
            id,
            card,
            current,
            offset,
            dragging: true,
            ..
        }) = &self.gesture
        else {
            return None;
        };
        let object = doc.get(*id)?;
        let ObjectKind::Kanban(kanban) = &object.data.kind else {
            return None;
        };
        let data = &object.data;
        let i = kanban.card_index(card)?;
        let middle = data.frame().to_local(*current + *offset);
        let (col, at) = kanban.drop_at(data.bounds, middle, Some(i));
        let old = kanban.place_of(i)?;
        let others_before = old.1;
        if col == old.0 && at == others_before {
            return None;
        }
        let moved = pb_core::Card {
            column: kanban.columns[col].id.clone(),
            order: kanban.order_at(col, at, Some(i)),
            ..kanban.cards[i].clone()
        };
        let bounds = kanban.with_card(moved.clone()).fitted(data.bounds);
        Some((*id, moved, bounds))
    }

    /// Whether a kanban board is selected alone, and how many columns it has.
    pub fn kanban_columns(&self, doc: &Document) -> Option<usize> {
        self.single_kanban(doc).map(|(_, _, k)| k.columns.len())
    }

    /// Adds a column at the right of the selected kanban board; the board grows by a column.
    pub fn add_kanban_column(&self, doc: &Document) -> Option<Command> {
        let (id, data, kanban) = self.single_kanban(doc)?;
        let mut columns = kanban.columns.to_vec();
        let n = columns.len() as f64;
        columns.push(Column {
            id: doc.new_line_id().into(),
            title: "New column".into(),
        });
        let b = data.bounds;
        let grown = Kanban {
            columns: columns.into(),
            ..kanban.clone()
        };
        let wide = Rect::new(b.x0, b.y0, b.x0 + b.width() * (n + 1.0) / n, b.y1);
        Some(Command::SetColumns {
            id,
            columns: grown.columns.clone(),
            bounds: grown.fitted(wide),
        })
    }

    /// Removes the right column of the selected kanban board, if it has more than one; the
    /// board shrinks by a column.
    pub fn remove_kanban_column(&self, doc: &Document) -> Option<Command> {
        let (id, data, kanban) = self.single_kanban(doc)?;
        let n = kanban.columns.len();
        if n < 2 {
            return None;
        }
        let shrunk = Kanban {
            columns: kanban.columns[..n - 1].into(),
            ..kanban.clone()
        };
        let b = data.bounds;
        let narrow = Rect::new(
            b.x0,
            b.y0,
            b.x0 + b.width() * (n - 1) as f64 / n as f64,
            b.y1,
        );
        Some(Command::SetColumns {
            id,
            columns: shrunk.columns.clone(),
            bounds: shrunk.fitted(narrow),
        })
    }

    /// Removes the selected card from its board.
    pub fn remove_selected_card(&mut self, doc: &Document) -> Option<Command> {
        let (id, i) = self.selected_card(doc)?;
        let (_, data, kanban) = self.single_kanban(doc)?;
        let card = kanban.cards[i].id.clone();
        let cards: Vec<_> = kanban
            .cards
            .iter()
            .filter(|c| c.id != card)
            .cloned()
            .collect();
        let left = Kanban {
            cards: cards.into(),
            ..kanban.clone()
        };
        self.card = None;
        Some(Command::RemoveCard {
            id,
            card,
            bounds: left.fitted(data.bounds),
        })
    }

    /// The "add card" row of the selected kanban board at `pointer`: the board and the
    /// column.
    pub fn add_card_at(&self, doc: &Document, pointer: Pointer) -> Option<(ObjectId, usize)> {
        let (id, data, kanban) = self.single_kanban(doc)?;
        let local = data.frame().to_local(pointer.world);
        match kanban.part_at(data.bounds, local)? {
            KanbanPart::Add(col) => Some((id, col)),
            _ => None,
        }
    }

    /// The single selected unlocked table, with its data.
    fn single_table<'a>(&self, doc: &'a Document) -> Option<(ObjectId, &'a ObjectData, &'a Table)> {
        let [id] = self.selection.ids() else {
            return None;
        };
        let object = doc.get(*id).filter(|_| !doc.is_locked(*id))?;
        match &object.data.kind {
            ObjectKind::Table(table) => Some((*id, &object.data, table)),
            _ => None,
        }
    }

    /// Adds a row (`rows`) or a column to the selected table, as big as the average one;
    /// the table grows to make room. It goes after the selected cells (`after`) or before
    /// them, or at the bottom or right when no cells are selected.
    pub fn add_table_line(&mut self, doc: &Document, rows: bool, after: bool) -> Option<Command> {
        let (id, data, table) = self.single_table(doc)?;
        let cells = self.table_cells(doc).map(|(_, range)| range);
        let lines = if rows { &table.rows } else { &table.cols };
        let at = match cells {
            Some(range) if after => range.lines(rows).end() + 1,
            Some(range) => *range.lines(rows).start(),
            None => lines.len(),
        };
        let total: f64 = lines.iter().map(|l| l.weight).sum();
        let weight = total / lines.len() as f64;
        let mut grown = lines.to_vec();
        grown.insert(
            at,
            pb_core::Line {
                id: doc.new_line_id().into(),
                weight,
            },
        );
        let bounds = resized_bounds(data.bounds, rows, (total + weight) / total);
        // Merged blocks are named by their corner lines, so one inside them widens them.
        let command = self.table_lines(id, table, rows, grown, table.merges.clone(), bounds);
        // The selected cells stay on their lines.
        if let Some(sel) = &mut self.cells
            && !after
            && cells.is_some()
        {
            for cell in [&mut sel.anchor, &mut sel.end] {
                let i = if rows { &mut cell.0 } else { &mut cell.1 };
                *i += 1;
            }
        }
        Some(command)
    }

    /// Removes the rows (`rows`) or columns of the selected cells from the selected table,
    /// or its bottom row or right column when no cells are selected; at least one is left.
    /// The table shrinks by their size.
    pub fn remove_table_lines(&mut self, doc: &Document, rows: bool) -> Option<Command> {
        let (id, data, table) = self.single_table(doc)?;
        let lines = if rows { &table.rows } else { &table.cols };
        let gone = match self.table_cells(doc) {
            Some((_, range)) => range.lines(rows),
            None => lines.len() - 1..=lines.len() - 1,
        };
        if gone.clone().count() >= lines.len() {
            return None;
        }
        let total: f64 = lines.iter().map(|l| l.weight).sum();
        let removed: f64 = lines[gone.clone()].iter().map(|l| l.weight).sum();
        let bounds = resized_bounds(data.bounds, rows, (total - removed) / total);
        let kept = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| !gone.contains(i))
            .map(|(_, l)| l.clone())
            .collect();
        self.cells = None;
        let merges = table.merges_without(rows, gone);
        Some(self.table_lines(id, table, rows, kept, merges, bounds))
    }

    /// Whether the selected cells can be merged (`merge`), or split back.
    pub fn can_merge_cells(&self, doc: &Document, merge: bool) -> bool {
        let (Some((_, range)), Some((_, _, table))) =
            (self.table_cells(doc), self.single_table(doc))
        else {
            return false;
        };
        let merged = table.merged();
        if merge {
            !range.is_single() && !merged.contains(&range)
        } else {
            merged.iter().any(|m| m.intersects(&range))
        }
    }

    /// Joins the selected cells into one (`merge`), or splits the merged ones among them.
    pub fn merge_cells(&self, doc: &Document, merge: bool) -> Option<Command> {
        if !self.can_merge_cells(doc, merge) {
            return None;
        }
        let (_, range) = self.table_cells(doc)?;
        let (id, data, table) = self.single_table(doc)?;
        let merges = if merge {
            table.with_merged(range)
        } else {
            table.with_split(range)
        };
        Some(Command::SetTable {
            id,
            rows: table.rows.clone(),
            cols: table.cols.clone(),
            merges,
            bounds: data.bounds,
        })
    }

    /// Empties the selected cells, one command per cell with text.
    pub fn clear_cells_text(&self, doc: &Document) -> Vec<Command> {
        let Some((id, range)) = self.table_cells(doc) else {
            return Vec::new();
        };
        let Some((_, _, table)) = self.single_table(doc) else {
            return Vec::new();
        };
        range
            .rows()
            .flat_map(|r| range.cols().map(move |c| (r, c)))
            .filter(|&(r, c)| !table.text(r, c).0.is_empty())
            .map(|(r, c)| Command::SetCell {
                id,
                row: table.rows[r].id.clone(),
                col: table.cols[c].id.clone(),
                cell: pb_core::Cell::default(),
            })
            .collect()
    }

    fn table_lines(
        &self,
        id: ObjectId,
        table: &Table,
        rows: bool,
        lines: Vec<pb_core::Line>,
        merges: std::sync::Arc<[pb_core::Merge]>,
        bounds: Rect,
    ) -> Command {
        let (r, c) = if rows {
            (lines.into(), table.cols.clone())
        } else {
            (table.rows.clone(), lines.into())
        };
        Command::SetTable {
            id,
            rows: r,
            cols: c,
            merges,
            bounds,
        }
    }

    /// Puts the selected connectors back on their automatic paths.
    pub fn reset_bends_selected(&self, doc: &Document) -> Option<Command> {
        let ids: Vec<_> = self
            .unlocked(doc)
            .into_iter()
            .filter(|&id| {
                matches!(doc.get(id).map(|o| &o.data.kind),
                    Some(ObjectKind::Connector(c)) if !c.bends.is_empty())
            })
            .collect();
        match ids[..] {
            [id] => Some(Command::SetBends {
                id,
                bends: Default::default(),
            }),
            _ => None,
        }
    }

    /// Connection points to show while placing a connector end, and the one it snaps to.
    pub fn connection_hints(&self, doc: &Document) -> (Vec<Point>, Option<Point>) {
        match &self.gesture {
            Some(Gesture::Connect { end, .. } | Gesture::MoveEnd { end, .. }) => {
                connect::hints(doc, end)
            }
            None if self.active == ToolKind::Connector => match &self.connect_hover {
                Some(end) => connect::hints(doc, end),
                None => (Vec::new(), None),
            },
            _ => (Vec::new(), None),
        }
    }

    /// The single selected connector, if it is not locked.
    fn single_connector(&self, doc: &Document) -> Option<(ObjectId, Connector)> {
        let [id] = self.selection.ids() else {
            return None;
        };
        match doc.get(*id).filter(|_| !doc.is_locked(*id))?.data.kind {
            ObjectKind::Connector(ref c) => Some((*id, c.clone())),
            _ => None,
        }
    }

    /// Objects as they look mid-gesture (moved, resized, rotated), replacing stored state.
    pub fn overrides(&self, doc: &Document) -> Vec<Object> {
        match &self.gesture {
            Some(Gesture::Move {
                dragging: true,
                duplicate: false,
                ..
            }) => {
                let delta = self.move_delta(doc).0;
                self.moving(doc)
                    .into_iter()
                    .flat_map(|id| doc.leaves(id))
                    .map(|o| {
                        let mut o = o.clone();
                        o.data.translate(delta);
                        o
                    })
                    .collect()
            }
            Some(Gesture::MoveEnd {
                id,
                which,
                connector,
                end,
                ..
            }) => {
                let Some(mut o) = doc.get(*id).cloned() else {
                    return Vec::new();
                };
                let mut c = connector.clone();
                match which {
                    Which::Start => c.start = *end,
                    Which::End => c.end = *end,
                }
                o.data.kind = ObjectKind::Connector(c);
                vec![o]
            }
            Some(Gesture::MoveCard {
                id, dragging: true, ..
            }) => {
                let Some(mut o) = doc.get(*id).cloned() else {
                    return Vec::new();
                };
                if let (Some((_, card, bounds)), ObjectKind::Kanban(k)) =
                    (self.moved_card(doc), &mut o.data.kind)
                {
                    *k = k.with_card(card);
                    o.data.bounds = bounds;
                }
                vec![o]
            }
            Some(Gesture::MoveDivider {
                id,
                divider,
                lines: Some(lines),
                ..
            }) => {
                let Some(mut o) = doc.get(*id).cloned() else {
                    return Vec::new();
                };
                if let ObjectKind::Table(t) = &mut o.data.kind {
                    if divider.rows {
                        t.rows = lines.clone();
                    } else {
                        t.cols = lines.clone();
                    }
                }
                vec![o]
            }
            Some(Gesture::MoveBend {
                id,
                bends: Some(bends),
                ..
            }) => {
                let Some(mut o) = doc.get(*id).cloned() else {
                    return Vec::new();
                };
                if let ObjectKind::Connector(c) = &mut o.data.kind {
                    c.bends = bends.clone().into();
                }
                vec![o]
            }
            Some(Gesture::MoveLabel {
                id,
                label,
                dragging: true,
                ..
            }) => {
                let Some(mut o) = doc.get(*id).cloned() else {
                    return Vec::new();
                };
                if let ObjectKind::Connector(c) = &mut o.data.kind {
                    c.label = label.clone();
                }
                vec![o]
            }
            Some(Gesture::Resize { .. } | Gesture::Rotate { .. }) => self
                .geometry_changes(doc)
                .into_iter()
                .filter_map(|(id, frame)| {
                    let mut o = doc.get(id)?.clone();
                    o.data.set_frame(frame);
                    Some(o)
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// New objects the gesture would create (shape being drawn, Alt-drag copies).
    pub fn previews(&self, doc: &Document) -> Vec<ObjectData> {
        match &self.gesture {
            Some(Gesture::Shape { kind, .. }) => {
                vec![drawn(doc, *kind, self.shape_bounds(doc, false).0)]
            }
            Some(Gesture::Connect { start, end, .. }) => {
                vec![connector(*start, *end, self.connector_route)]
            }
            Some(Gesture::Ink {
                marker,
                points,
                zoom,
            }) => ink::stroke(points, self.ink_style(*marker), *marker, *zoom, false)
                .into_iter()
                .collect(),
            Some(Gesture::Move {
                dragging: true,
                duplicate: true,
                ..
            }) => self
                .moved_copies(doc)
                .iter()
                .flat_map(Content::shapes)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Smart guides of the gesture in progress, in world space.
    pub fn guides(&self, doc: &Document) -> Vec<Line> {
        match &self.gesture {
            Some(Gesture::Move { dragging: true, .. }) => self.move_delta(doc).1,
            Some(Gesture::Resize { .. }) => self.resize_target(doc).map_or(Vec::new(), |t| t.1),
            Some(Gesture::Shape { .. }) => self.shape_bounds(doc, false).1,
            _ => Vec::new(),
        }
    }

    /// Objects the gesture is about to delete (the eraser), hidden until it ends.
    pub fn hidden(&self) -> &[ObjectId] {
        match &self.gesture {
            Some(Gesture::Erase { erased, .. }) => erased,
            _ => &[],
        }
    }

    /// The eraser's outline: center and radius in world units.
    pub fn eraser(&self) -> Option<(Point, f64)> {
        self.eraser_at.filter(|_| self.active == ToolKind::Eraser)
    }

    pub fn marquee(&self) -> Option<Rect> {
        match &self.gesture {
            Some(Gesture::Marquee { start, current, .. }) => {
                Some(Rect::from_points(*start, *current))
            }
            _ => None,
        }
    }

    /// Object under the pointer that is not selected, for a hover outline.
    pub fn hover(&self) -> Option<ObjectId> {
        self.hover.filter(|&id| !self.selection.contains(id))
    }

    /// Resize and rotation handles are shown only when nothing else is going on, and never
    /// on locked objects.
    pub fn shows_handles(&self, doc: &Document) -> bool {
        self.active == ToolKind::Select
            && !self.selection.has_locked(doc)
            && self.single_connector(doc).is_none()
            && !matches!(
                self.gesture,
                Some(Gesture::Move { dragging: true, .. })
                    | Some(Gesture::Marquee { .. })
                    | Some(Gesture::Rotate { .. })
            )
    }

    pub fn cursor(&self) -> CursorHint {
        match &self.gesture {
            Some(Gesture::Resize { handle, from, .. }) => handle.cursor(from.rotation),
            Some(Gesture::MoveDivider { divider, .. }) => divider_cursor(*divider),
            Some(Gesture::Rotate { .. }) => CursorHint::Rotate,
            Some(Gesture::Move {
                dragging: true,
                duplicate,
                ..
            }) => {
                if *duplicate {
                    CursorHint::Copy
                } else {
                    CursorHint::Move
                }
            }
            Some(Gesture::Shape { .. } | Gesture::Connect { .. } | Gesture::MoveEnd { .. }) => {
                CursorHint::Crosshair
            }
            Some(Gesture::MoveLabel { .. } | Gesture::MoveBend { .. }) => CursorHint::Move,
            Some(Gesture::MoveCard { dragging: true, .. }) => CursorHint::Move,
            _ if self.active != ToolKind::Select => CursorHint::Crosshair,
            _ => match self.hover_grip {
                Some(Grip::Resize(handle)) => handle.cursor(self.hover_rotation),
                Some(Grip::Divider(divider)) => divider_cursor(divider),
                Some(Grip::Rotate) => CursorHint::Rotate,
                Some(Grip::End(_)) => CursorHint::Crosshair,
                Some(Grip::Label | Grip::Bend(_)) => CursorHint::Move,
                None => CursorHint::Default,
            },
        }
    }

    /// All objects making up the selection, groups expanded.
    fn selected<'a>(&'a self, doc: &'a Document) -> impl Iterator<Item = &'a Object> + 'a {
        self.selection.ids().iter().flat_map(|&id| doc.leaves(id))
    }

    /// The top-level object or group under the pointer, or the frame whose title it is on.
    pub fn object_at(&self, doc: &Document, pointer: Pointer) -> Option<ObjectId> {
        hit::object_at(doc, pointer.world, HIT_SLOP / pointer.zoom)
            .or_else(|| hit::frame_title_at(doc, pointer.world, pointer.zoom))
    }

    /// The selected objects and groups that are not locked.
    fn unlocked(&self, doc: &Document) -> Vec<ObjectId> {
        let ids = self.selection.ids().iter().copied();
        ids.filter(|&id| !doc.is_locked(id)).collect()
    }

    /// What moving the selection moves: the unlocked selection and what its frames hold.
    fn moving(&self, doc: &Document) -> Vec<ObjectId> {
        let ids = doc.with_frame_members(&self.unlocked(doc));
        ids.into_iter().filter(|&id| !doc.is_locked(id)).collect()
    }

    /// The object quick create works on: a single unlocked object selected with nothing
    /// else going on.
    fn quick_target(&self, doc: &Document) -> Option<ObjectId> {
        match self.selection.ids() {
            [id] if self.active == ToolKind::Select
                && self.gesture.is_none()
                && !doc.is_locked(*id) =>
            {
                Some(*id)
            }
            _ => None,
        }
    }

    /// Quick create's "+" dots around the selection (world), if it has them.
    pub fn quick_dots(&self, doc: &Document, zoom: f64) -> Vec<Point> {
        self.quick_target(doc).map_or_else(Vec::new, |id| {
            quick::dots(doc, id, zoom)
                .into_iter()
                .map(|(_, p)| p)
                .collect()
        })
    }

    /// The quick create dot under the pointer, with the object it belongs to.
    pub fn quick_dot_at(
        &self,
        doc: &Document,
        pointer: Pointer,
    ) -> Option<(ObjectId, quick::Side)> {
        let id = self.quick_target(doc)?;
        quick::dot_at(doc, id, pointer.world, pointer.zoom).map(|side| (id, side))
    }

    /// Frames do not rotate, so neither does a selection with one.
    pub fn can_rotate(&self, doc: &Document) -> bool {
        !self.selected(doc).any(|o| o.data.kind.is_frame())
    }

    /// A single object (not a group) is selected: handles follow its own rotation.
    fn single_object(&self, doc: &Document) -> bool {
        matches!(self.selection.ids(), [id] if doc.get(*id).is_some())
    }

    /// The selection-frame grip under the pointer, with the frame it belongs to.
    fn grip_at(&self, doc: &Document, pointer: Pointer) -> Option<(Frame, Grip)> {
        if self.selection.has_locked(doc) {
            return None;
        }
        let frame = self.selection.frame(doc)?;
        let radius = HANDLE_HIT_RADIUS / pointer.zoom;
        // A single connector has handles on its ends instead of around it, and its label
        // slides along it.
        if let Some((id, _)) = self.single_connector(doc) {
            let (s, e) = doc.connector_ends(id)?;
            let near = |p: Point| (p - pointer.world).hypot() <= radius;
            let end = [(s.point, Which::Start), (e.point, Which::End)]
                .into_iter()
                .find(|&(p, _)| near(p))
                .map(|(_, which)| (frame, Grip::End(which)));
            let on_label = doc
                .connector_label(id)
                .is_some_and(|r| r.contains(pointer.world));
            let bend = bend::handles(doc, id)
                .into_iter()
                .find(|&(p, _)| near(p))
                .map(|(_, grip)| (frame, Grip::Bend(grip)));
            return end.or(bend).or(on_label.then_some((frame, Grip::Label)));
        }
        if self.can_rotate(doc)
            && (rotation_knob(frame, pointer.zoom) - pointer.world).hypot() <= radius
        {
            return Some((frame, Grip::Rotate));
        }
        if let Some(h) = handles::handle_at(frame, pointer.world, radius) {
            return Some((frame, Grip::Resize(h)));
        }
        let (_, data, table) = self.single_table(doc)?;
        let divider = table::divider_at(data, table, pointer.world, radius)?;
        Some((frame, Grip::Divider(divider)))
    }

    fn update_hover(&mut self, doc: &Document, pointer: Pointer) -> bool {
        if self.active != ToolKind::Select {
            return false;
        }
        let grip = self.grip_at(doc, pointer);
        let hover = match grip {
            Some(_) => None,
            None => self.object_at(doc, pointer),
        };
        let grip_kind = grip.map(|(_, g)| g);
        let changed = hover != self.hover || grip_kind != self.hover_grip;
        self.hover = hover;
        self.hover_grip = grip_kind;
        self.hover_rotation = grip.map_or(0.0, |(f, _)| f.rotation);
        changed
    }

    fn moved_copies(&self, doc: &Document) -> Vec<Content> {
        if !matches!(self.gesture, Some(Gesture::Move { .. })) {
            return Vec::new();
        }
        let items = clipboard::in_z_order(doc, &self.selection);
        clipboard::offset(&items, self.move_delta(doc).0)
    }

    /// What to snap to: visible top-level objects except `exclude`, and the grid if on.
    /// None while Ctrl is held.
    fn snapper(&self, doc: &Document, pointer: &Pointer, exclude: &[ObjectId]) -> Option<Snapper> {
        if pointer.ctrl {
            return None;
        }
        let mut units: Vec<ObjectId> = doc
            .objects_in(pointer.visible)
            .into_iter()
            .map(|o| doc.top_level(o.id))
            .filter(|id| !exclude.contains(id) && doc.reachable(*id))
            .collect();
        units.sort_by_key(|&id| doc.paint_order(id));
        units.dedup();
        let targets = units
            .into_iter()
            .filter_map(|id| unit_frame(doc, id, |o| o.data.frame()))
            .map(|f| f.aabb())
            .filter(|r| r.overlaps(pointer.visible))
            .collect();
        Some(Snapper {
            targets,
            tolerance: SNAP_DISTANCE / pointer.zoom,
            grid: self
                .snap_to_grid
                .then(|| GridLevel::for_zoom(pointer.zoom).snap_spacing()),
        })
    }

    /// Offset of the move gesture in progress after snapping, with its guides.
    fn move_delta(&self, doc: &Document) -> (Vec2, Vec<Line>) {
        let Some(Gesture::Move {
            start,
            current,
            duplicate,
            ..
        }) = &self.gesture
        else {
            return (Vec2::ZERO, Vec::new());
        };
        let raw = current.world - *start;
        let ids = if *duplicate {
            self.selection.ids().to_vec()
        } else {
            self.moving(doc)
        };
        // Copies leave the originals in place, so those are targets too.
        let exclude: &[ObjectId] = if *duplicate { &[] } else { &ids };
        let snapper = self.snapper(doc, current, exclude);
        let frame = selection_frame(doc, &ids, |o| o.data.frame());
        let (Some(snapper), Some(frame)) = (snapper, frame) else {
            return (raw, Vec::new());
        };
        let bounds = frame.aabb();
        let dragged = bounds + raw;
        let delta = raw + snapper.offset(&xs_of(dragged), &ys_of(dragged));
        let moved = bounds + delta;
        (delta, snapper.guides(moved, &xs_of(moved), &ys_of(moved)))
    }

    /// Frame-local bounds the resize gesture in progress leads to, with snapping guides.
    ///
    /// Only axis-aligned frames snap, and not while a corner keeps the aspect ratio: with
    /// Shift held, or by default when images are selected (then Shift stretches them).
    fn resize_target(&self, doc: &Document) -> Option<(Rect, Vec<Line>)> {
        let Some(Gesture::Resize {
            handle,
            from,
            start,
            current,
        }) = &self.gesture
        else {
            return None;
        };
        // Resize happens in the frame's own (unrotated) space.
        let mut delta = from.vec_to_local(current.world - *start);
        let has_image = self.selection.ids().iter().any(|&id| {
            doc.leaves(id)
                .iter()
                .any(|o| matches!(o.data.kind, ObjectKind::Image(_)))
        });
        let keep_aspect = current.shift != has_image;
        let snapper = (from.rotation == 0.0 && !(keep_aspect && handle.is_corner()))
            .then(|| self.snapper(doc, current, self.selection.ids()))
            .flatten();
        let Some(snapper) = snapper else {
            return Some((
                handles::resize(from.rect, *handle, delta, keep_aspect),
                Vec::new(),
            ));
        };
        let (x, y) = handle.dragged_edges(handles::resize(from.rect, *handle, delta, keep_aspect));
        delta += snapper.offset(&Vec::from_iter(x), &Vec::from_iter(y));
        let to = handles::resize(from.rect, *handle, delta, keep_aspect);
        let (x, y) = handle.dragged_edges(to);
        let guides = snapper.guides(to.abs(), &Vec::from_iter(x), &Vec::from_iter(y));
        Some((to, guides))
    }

    /// `pointer.world` snapped to objects and the grid, with guides through it.
    fn snap_point(&self, doc: &Document, pointer: Pointer) -> (Point, Vec<Line>) {
        let p = pointer.world;
        let Some(snapper) = self.snapper(doc, &pointer, &[]) else {
            return (p, Vec::new());
        };
        let p = p + snapper.offset(&[p.x], &[p.y]);
        (p, snapper.guides(Rect::from_points(p, p), &[p.x], &[p.y]))
    }

    /// Bounds of the shape being drawn, with guides through its snapped corner. On release
    /// (`finished`), a click instead of a drag makes a default-sized shape centered on the
    /// click point.
    fn shape_bounds(&self, doc: &Document, finished: bool) -> (Rect, Vec<Line>) {
        let Some(Gesture::Shape {
            kind,
            start,
            anchor,
            current,
        }) = &self.gesture
        else {
            return (Rect::ZERO, Vec::new());
        };
        if finished && current.screen_distance(*start) < CLICK_SLOP {
            let size = match kind {
                Drawn::Shape(kind) => kind.default_size(),
                Drawn::Frame => {
                    let (w, h) = FrameData::CLICK_SIZE;
                    (w / current.zoom, h / current.zoom)
                }
                Drawn::Table => {
                    let (w, h) = Table::CELL_SIZE;
                    (3.0 * w / current.zoom, 3.0 * h / current.zoom)
                }
                // The height is fitted to the columns, see `drawn`.
                Drawn::Kanban => (3.0 * Kanban::COLUMN_WIDTH / current.zoom, 1.0),
            };
            return (Rect::from_center_size(*anchor, size), Vec::new());
        }
        let (end, guides) = self.snap_point(doc, *current);
        (shape_rect(*anchor, end, current.shift), guides)
    }

    /// New frames of the selected objects for the resize or rotate gesture in progress.
    fn geometry_changes(&self, doc: &Document) -> Vec<(ObjectId, Frame)> {
        match &self.gesture {
            Some(Gesture::Resize { from, .. }) => {
                let Some((to, _)) = self.resize_target(doc) else {
                    return Vec::new();
                };
                if self.single_object(doc) {
                    let center = from.to_world(to.center());
                    let rect = Rect::from_center_size(center, to.size()).abs();
                    self.selected(doc)
                        .map(|o| (o.id, Frame::new(rect, from.rotation)))
                        .collect()
                } else {
                    // A group or multi-selection frame is axis-aligned; children keep their
                    // rotation.
                    self.selected(doc)
                        .map(|o| {
                            let rect = handles::map_rect(o.data.bounds, from.rect, to);
                            (o.id, Frame::new(rect, o.data.rotation))
                        })
                        .collect()
                }
            }
            Some(Gesture::Rotate {
                from,
                start,
                current,
            }) => {
                let pivot = from.center();
                let angle_of = |p: Point| (p - pivot).atan2();
                let mut delta = angle_of(current.world) - angle_of(*start);
                if current.shift {
                    // Snap the resulting angle of a single object, or the turn of a group.
                    let base = if self.single_object(doc) {
                        from.rotation
                    } else {
                        0.0
                    };
                    delta = ((base + delta) / ROTATION_SNAP).round() * ROTATION_SNAP - base;
                }
                self.selected(doc)
                    .map(|o| (o.id, o.data.frame().rotated_about(pivot, delta)))
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}

/// Up-down arrows over a line between rows, left-right ones over a line between columns.
/// `bounds` with its height (`rows`) or width scaled by `scale`, keeping the top left.
fn resized_bounds(b: Rect, rows: bool, scale: f64) -> Rect {
    if rows {
        Rect::new(b.x0, b.y0, b.x1, b.y0 + b.height() * scale)
    } else {
        Rect::new(b.x0, b.y0, b.x0 + b.width() * scale, b.y1)
    }
}

fn divider_cursor(divider: Divider) -> CursorHint {
    if divider.rows {
        CursorHint::ResizeNs
    } else {
        CursorHint::ResizeEw
    }
}

/// A new connector with an arrow at its end, in the default outline style.
fn connector(start: End, end: End, route: Route) -> ObjectData {
    ObjectData {
        kind: ObjectKind::Connector(Connector {
            bends: Default::default(),
            start,
            end,
            route,
            start_cap: Cap::None,
            end_cap: Cap::Arrow,
            label: Label::default(),
        }),
        bounds: Rect::from_points(start.point(), end.point()),
        rotation: 0.0,
        style: Style::default(),
        tags: Default::default(),
        link: None,
    }
}

/// A new shape or frame over `bounds`. Frames are named in the order they are made.
fn drawn(doc: &Document, kind: Drawn, bounds: Rect) -> ObjectData {
    match kind {
        Drawn::Shape(shape) => ObjectData {
            kind: shape.into(),
            bounds,
            rotation: 0.0,
            style: Style::shape(),
            tags: Default::default(),
            link: None,
        },
        Drawn::Kanban => {
            let kanban = Kanban::starter(|| doc.new_line_id());
            ObjectData {
                bounds: kanban.fitted(bounds),
                kind: ObjectKind::Kanban(kanban),
                rotation: 0.0,
                style: Kanban::style(),
                tags: Default::default(),
                link: None,
            }
        }
        Drawn::Table => ObjectData {
            kind: ObjectKind::Table(Table::new(3, 3, || doc.new_line_id())),
            bounds,
            rotation: 0.0,
            style: Table::style(),
            tags: Default::default(),
            link: None,
        },
        Drawn::Frame => {
            let count = doc
                .objects()
                .iter()
                .filter(|o| o.data.kind.is_frame())
                .count();
            ObjectData {
                kind: ObjectKind::Frame(FrameData {
                    title: format!("Frame {}", count + 1).into(),
                }),
                bounds,
                rotation: 0.0,
                style: Style {
                    stroke_width: 0.0,
                    ..Style::default()
                },
                tags: Default::default(),
                link: None,
            }
        }
    }
}

/// Rectangle from `anchor` to `end`; `square` (Shift) makes it a square.
fn shape_rect(anchor: Point, end: Point, square: bool) -> Rect {
    let mut delta = end - anchor;
    if square {
        let side = delta.x.abs().max(delta.y.abs());
        delta = Vec2::new(side.copysign(delta.x), side.copysign(delta.y));
    }
    Rect::from_points(anchor, anchor + delta)
}

#[cfg(test)]
mod tests;
