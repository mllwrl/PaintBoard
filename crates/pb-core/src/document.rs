use std::collections::{HashMap, HashSet};

use std::sync::Mutex;

use loro::event::{Diff, DiffEvent};
use loro::{Index, Subscription};
use rstar::{AABB, RTree, RTreeObject};
use std::fmt;
use std::sync::Arc;

use kurbo::{Point, Rect, Vec2};
use loro::{
    ExportMode, LoroDoc, LoroError, LoroMap, LoroTree, LoroValue, TreeID, TreeParentId,
    UndoManager, ValueOrContainer,
};
#[cfg(test)]
use pb_geom::ShapeKind;
use pb_geom::normalize_angle;

use crate::connector::{self, Anchor, Cap, ConnectorPath, EndPoint, Label, Route};
use crate::rich::{self, Span, Spans};

use crate::command::reorder;
use crate::{
    Applied, AssetId, Assets, Command, Connector, Content, End, FrameData, Group, HAlign, Image,
    Ink, Line, Object, ObjectData, ObjectId, ObjectKind, Rgba, Shape, Sticky, Style, StyleChange,
    Text, VAlign, ZOrder,
};

/// Font size of text objects stored without one.
const DEFAULT_FONT_SIZE: f64 = 24.0;

/// Bump when the stored layout changes incompatibly, and add a migration.
const SCHEMA_VERSION: i64 = 1;

/// Keys used in the CRDT. Never rename them: saved boards depend on them.
mod key {
    pub const META: &str = "meta";
    pub const SCHEMA: &str = "schema";
    pub const OBJECTS: &str = "objects";

    pub const ID: &str = "id";
    pub const KIND: &str = "kind";
    pub const X: &str = "x";
    pub const Y: &str = "y";
    pub const W: &str = "w";
    pub const H: &str = "h";
    pub const ROTATION: &str = "rot";
    pub const FILL: &str = "fill";
    pub const STROKE: &str = "stroke";
    pub const STROKE_WIDTH: &str = "strokeWidth";
    pub const OPACITY: &str = "opacity";
    /// A shape's text color and font size, absent for the defaults (see `Style`).
    pub const TEXT_COLOR: &str = "textColor";
    pub const TEXT_SIZE: &str = "textSize";
    /// A shape's text alignment, see `HAlign` and `VAlign`.
    pub const TEXT_ALIGN: &str = "textAlign";
    pub const TEXT_VALIGN: &str = "textVAlign";
    pub const LOCKED: &str = "locked";
    /// Points of an ink stroke, see `Ink::to_bytes`.
    pub const POINTS: &str = "points";
    pub const TEXT: &str = "text";
    pub const FONT_SIZE: &str = "fontSize";
    /// Inline styles of `text`, see `rich::to_bytes`.
    pub const MARKS: &str = "marks";
    /// Hash of an image's file, see `AssetId`.
    pub const ASSET: &str = "asset";
    /// Shown part of an image, see `Image::crop`.
    pub const CROP_X0: &str = "cropX0";
    pub const CROP_Y0: &str = "cropY0";
    pub const CROP_X1: &str = "cropX1";
    pub const CROP_Y1: &str = "cropY1";
    /// Connectors: route and arrowheads...
    pub const ROUTE: &str = "route";
    pub const START_CAP: &str = "startCap";
    pub const END_CAP: &str = "endCap";
    /// ...and per end (prefixed `start` or `end`): a point (free, or last known position),
    /// the attached object's id, and a fixed anchor (absent for an automatic one).
    pub const START: &str = "start";
    pub const END: &str = "end";
    pub const X_SUFFIX: &str = "X";
    pub const Y_SUFFIX: &str = "Y";
    pub const OBJECT_SUFFIX: &str = "Obj";
    pub const ANCHOR_X_SUFFIX: &str = "AnchorX";
    pub const ANCHOR_Y_SUFFIX: &str = "AnchorY";
    /// ...and the label: its text under `text`, where it sits and its measured size.
    pub const LABEL_AT: &str = "labelAt";
    pub const LABEL_W: &str = "labelW";
    pub const LABEL_H: &str = "labelH";
    /// ...and the points it runs through when set by hand: little-endian `f64` pairs.
    pub const BENDS: &str = "bends";
    /// Tables: rows and columns as `id=weight` pairs joined by `;`, and each cell's text
    /// and inline styles under these prefixes followed by `row:col` ids.
    pub const ROWS: &str = "rows";
    pub const COLS: &str = "cols";
    pub const CELL: &str = "cell:";
    pub const CELL_MARKS: &str = "cellMarks:";
    /// ...and merged cells as `row0,col0,row1,col1` ids joined by `;`.
    pub const MERGES: &str = "merges";

    /// `kind` of group nodes.
    pub const GROUP: &str = "group";
}

#[derive(Debug)]
pub enum Error {
    UnknownObject(ObjectId),
    /// The board was saved by a newer version of the app.
    UnsupportedSchema(i64),
    Crdt(LoroError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownObject(id) => write!(f, "object {id} does not exist"),
            Self::UnsupportedSchema(v) => {
                write!(
                    f,
                    "board schema v{v} is newer than supported v{SCHEMA_VERSION}"
                )
            }
            Self::Crdt(err) => write!(f, "document error: {err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<LoroError> for Error {
    fn from(err: LoroError) -> Self {
        Self::Crdt(err)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A board: the CRDT document plus a cached, typed view of its objects.
///
/// Objects are nodes of a Loro movable tree. Sibling order is the z-order (back to front);
/// groups are nodes whose children are their members. All edits go through
/// [`Document::apply`].
pub struct Document {
    doc: LoroDoc,
    tree: LoroTree,
    undo: UndoManager,
    /// Cached objects in paint order (back to front, groups flattened), rebuilt after every
    /// change.
    objects: Vec<Object>,
    index: HashMap<ObjectId, usize>,
    groups: HashMap<ObjectId, Group>,
    /// Top-level objects and groups in z-order.
    roots: Vec<ObjectId>,
    /// Tree node currently holding each object or group.
    nodes: HashMap<ObjectId, TreeID>,
    revision: u64,
    /// Connectors' resolved ends and paths, by connector id; rebuilt with `objects`.
    connectors: HashMap<ObjectId, Resolved>,
    /// Bounds of `objects` for finding those in an area; rebuilt with `objects`.
    spatial: RTree<Placed>,
    /// What each tree node held when last read, so a refresh reads only nodes that changed.
    node_cache: HashMap<TreeID, NodeData>,
    /// Nodes changed since the last refresh, collected from the CRDT's events.
    changes: Arc<Mutex<Changes>>,
    /// Keeps the events coming.
    _events: Subscription,
    /// Ids handed out by `new_line_id`.
    line_ids: std::cell::Cell<u64>,
    /// Files used by images. They are not part of the CRDT and never removed while the board
    /// is open, so undoing the deletion of an image finds its file again.
    assets: Assets,
}

/// An object in the spatial index: its world bounds (outline included) and its place in
/// paint order.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    bounds: AABB<[f64; 2]>,
    slot: usize,
}

impl RTreeObject for Placed {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.bounds
    }
}

fn envelope(r: Rect) -> AABB<[f64; 2]> {
    AABB::from_corners([r.x0, r.y0], [r.x1, r.y1])
}

fn placed(slot: usize, data: &ObjectData) -> Placed {
    let pad = data.style.stroke_width / 2.0;
    Placed {
        bounds: envelope(data.frame().aabb().inflate(pad, pad)),
        slot,
    }
}

/// A tree node's contents as last read.
#[derive(Clone, Debug)]
enum NodeData {
    Group {
        id: ObjectId,
        locked: bool,
    },
    Object {
        id: ObjectId,
        data: Box<ObjectData>,
        locked: bool,
    },
    /// An object of a kind this version does not know, or not readable: skipped.
    Unknown {
        id: ObjectId,
    },
}

/// Changes reported by the CRDT since the last refresh.
#[derive(Debug, Default)]
struct Changes {
    nodes: HashSet<TreeID>,
    /// Something changed that cannot be tied to nodes: read everything again.
    all: bool,
}

impl Changes {
    fn record(&mut self, event: &DiffEvent<'_>) {
        for diff in &event.events {
            let mut found = false;
            for (_, index) in diff.path {
                if let Index::Node(node) = index {
                    self.nodes.insert(*node);
                    found = true;
                }
            }
            if let Diff::Tree(tree) = &diff.diff {
                for item in &tree.diff {
                    self.nodes.insert(item.target);
                }
                found = true;
            }
            if !found || diff.is_unknown {
                self.all = true;
            }
        }
    }
}

/// Where a connector runs now.
struct Resolved {
    start: EndPoint,
    end: EndPoint,
    path: ConnectorPath,
    label: Option<Rect>,
}

/// An object made by a `Create` command, for re-attaching copied connectors.
struct Made {
    id: ObjectId,
    node: TreeID,
    /// The object it copies.
    from: Option<ObjectId>,
    connector: Option<Connector>,
}

impl Document {
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        doc.get_map(key::META)
            .insert(key::SCHEMA, SCHEMA_VERSION)
            .expect("inserting into a fresh map cannot fail");
        doc.commit();
        Self::from_loro(doc)
    }

    /// Loads a board saved with [`Document::snapshot`].
    pub fn from_snapshot(bytes: &[u8]) -> Result<Self> {
        let doc = LoroDoc::from_snapshot(bytes)?;
        let schema = match doc.get_map(key::META).get(key::SCHEMA) {
            Some(ValueOrContainer::Value(LoroValue::I64(v))) => v,
            _ => SCHEMA_VERSION,
        };
        if schema > SCHEMA_VERSION {
            return Err(Error::UnsupportedSchema(schema));
        }
        Ok(Self::from_loro(doc))
    }

    fn from_loro(doc: LoroDoc) -> Self {
        let tree = doc.get_tree(key::OBJECTS);
        tree.enable_fractional_index(0);
        let undo = UndoManager::new(&doc);
        let changes = Arc::new(Mutex::new(Changes {
            all: true,
            ..Changes::default()
        }));
        let sink = changes.clone();
        let events = doc.subscribe_root(Arc::new(move |event| {
            if let Ok(mut changes) = sink.lock() {
                changes.record(&event);
            }
        }));
        let mut document = Self {
            doc,
            tree,
            undo,
            objects: Vec::new(),
            index: HashMap::new(),
            groups: HashMap::new(),
            roots: Vec::new(),
            nodes: HashMap::new(),
            revision: 0,
            assets: Assets::default(),
            connectors: HashMap::new(),
            spatial: RTree::new(),
            node_cache: HashMap::new(),
            changes,
            _events: events,
            line_ids: std::cell::Cell::new(0),
        };
        document.refresh();
        document
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.doc
            .export(ExportMode::Snapshot)
            .expect("snapshot export of a committed document cannot fail")
    }

    pub fn assets(&self) -> &Assets {
        &self.assets
    }

    /// Adds a file for images to use. Not an edit: it does not change the revision or the
    /// undo history.
    pub fn add_asset(&mut self, bytes: impl Into<std::sync::Arc<[u8]>>) -> AssetId {
        self.assets.insert(bytes)
    }

    /// A connector's path as it is drawn.
    pub fn connector_path(&self, id: ObjectId) -> Option<&ConnectorPath> {
        self.connectors.get(&id).map(|r| &r.path)
    }

    /// A connector's ends where they are now.
    pub fn connector_ends(&self, id: ObjectId) -> Option<(EndPoint, EndPoint)> {
        self.connectors.get(&id).map(|r| (r.start, r.end))
    }

    /// The box of a connector's label text; `None` without a label.
    pub fn connector_label(&self, id: ObjectId) -> Option<Rect> {
        self.connectors.get(&id)?.label
    }

    /// Assets that objects on the board use, each once, in a stable order.
    pub fn used_assets(&self) -> Vec<AssetId> {
        let mut ids: Vec<_> = self
            .objects
            .iter()
            .filter_map(|o| match &o.data.kind {
                ObjectKind::Image(image) => Some(image.asset),
                _ => None,
            })
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// All objects in paint order, back to front, with groups flattened.
    pub fn objects(&self) -> &[Object] {
        &self.objects
    }

    /// An object (not a group).
    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.index.get(&id).map(|&i| &self.objects[i])
    }

    pub fn group(&self, id: ObjectId) -> Option<&Group> {
        self.groups.get(&id)
    }

    /// Top-level objects and groups in z-order, back to front.
    pub fn roots(&self) -> &[ObjectId] {
        &self.roots
    }

    /// Whether `id` is an existing object or group.
    pub fn contains(&self, id: ObjectId) -> bool {
        self.index.contains_key(&id) || self.groups.contains_key(&id)
    }

    /// The group directly containing `id`.
    pub fn parent(&self, id: ObjectId) -> Option<ObjectId> {
        match self.get(id) {
            Some(object) => object.parent,
            None => self.groups.get(&id)?.parent,
        }
    }

    /// The outermost group containing `id`, or `id` itself at the top level.
    pub fn top_level(&self, mut id: ObjectId) -> ObjectId {
        while let Some(parent) = self.parent(id) {
            id = parent;
        }
        id
    }

    /// The objects making up `id`: the object itself, or everything inside a group, in
    /// paint order.
    pub fn leaves(&self, id: ObjectId) -> Vec<&Object> {
        let mut out = Vec::new();
        self.collect_leaves(id, &mut out);
        out
    }

    fn collect_leaves<'a>(&'a self, id: ObjectId, out: &mut Vec<&'a Object>) {
        if let Some(object) = self.get(id) {
            out.push(object);
        } else if let Some(group) = self.groups.get(&id) {
            for &child in &group.children {
                self.collect_leaves(child, out);
            }
        }
    }

    /// Whether `id` or any group around it is locked.
    pub fn is_locked(&self, id: ObjectId) -> bool {
        let own = match self.get(id) {
            Some(object) => object.locked,
            None => self.groups.get(&id).is_some_and(|g| g.locked),
        };
        own || self.parent(id).is_some_and(|p| self.is_locked(p))
    }

    /// A copy of `id` and everything inside it, e.g. for the clipboard.
    pub fn content(&self, id: ObjectId) -> Option<Content> {
        match self.get(id) {
            Some(object) => {
                let mut data = object.data.clone();
                if let (ObjectKind::Connector(c), Some((s, e))) =
                    (&mut data.kind, self.connector_ends(id))
                {
                    c.start = with_position(c.start, s.point);
                    c.end = with_position(c.end, e.point);
                }
                Some(Content::Copied { from: id, data })
            }
            None => Some(Content::Group(
                self.groups
                    .get(&id)?
                    .children
                    .iter()
                    .filter_map(|&c| self.content(c))
                    .collect(),
            )),
        }
    }

    /// Increments on every change; lets views skip work when nothing changed.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn apply(&mut self, command: Command) -> Result<Applied> {
        let mut applied = Applied::default();
        match command {
            Command::Create(contents) => {
                let mut made = Vec::new();
                for content in &contents {
                    applied
                        .created
                        .push(self.create(TreeParentId::Root, content, &mut made)?);
                }
                self.attach_copies(&made)?;
                self.prune_empty_groups(TreeParentId::Root)?;
            }
            Command::Delete(ids) => {
                let outermost = self.outermost(&ids)?;
                self.remember_connector_ends(&outermost)?;
                for id in outermost {
                    self.tree.delete(self.nodes[&id])?;
                }
                self.prune_empty_groups(TreeParentId::Root)?;
            }
            Command::Move { ids, delta } => {
                let moves: Vec<_> = ids.into_iter().map(|id| (id, delta)).collect();
                self.move_each(&moves)?;
            }
            Command::MoveEach(moves) => self.move_each(&moves)?,
            Command::SetGeometry(changes) => {
                if let Some(&(id, _)) = changes.iter().find(|(id, _)| self.get(*id).is_none()) {
                    return Err(Error::UnknownObject(id));
                }
                for (id, frame) in changes {
                    let meta = self.tree.get_meta(self.nodes[&id])?;
                    if let Some(ObjectKind::Connector(c)) = self.get(id).map(|o| &o.data.kind) {
                        // Free ends keep their place relative to the connector's bounds.
                        let from = self.objects[self.index[&id]].data.bounds;
                        let map = |end: End| match end {
                            End::Free(p) => End::Free(map_point(p, from, frame)),
                            end => end,
                        };
                        write_end(&meta, key::START, &map(c.start))?;
                        write_end(&meta, key::END, &map(c.end))?;
                        let bends: Vec<_> =
                            c.bends.iter().map(|&p| map_point(p, from, frame)).collect();
                        write_bends(&meta, &bends)?;
                        write_bounds(&meta, frame.aabb())?;
                        meta.insert(key::ROTATION, 0.0)?;
                        continue;
                    }
                    write_bounds(&meta, frame.rect.abs())?;
                    meta.insert(key::ROTATION, normalize_angle(frame.rotation))?;
                }
            }
            Command::Reorder { ids, to } => {
                self.check_exist(&ids)?;
                let mut parents = Vec::new();
                for &id in &ids {
                    let parent = self.parent(id);
                    if !parents.contains(&parent) {
                        parents.push(parent);
                    }
                }
                for parent in parents {
                    self.reorder_siblings(parent, &ids, to)?;
                }
            }
            Command::Group(ids) => {
                let mut members = self.outermost(&ids)?;
                // Members keep their stacking order inside the group.
                members.sort_by_key(|&id| self.paint_rank(id));
                if let Some(&top) = members.last() {
                    let node = self.tree.create(self.parent_node(self.parent(top)))?;
                    self.tree.mov_after(node, self.nodes[&top])?;
                    let id = ObjectId::from_node(node);
                    let meta = self.tree.get_meta(node)?;
                    meta.insert(key::ID, id.to_string())?;
                    meta.insert(key::KIND, key::GROUP)?;
                    for (i, member) in members.iter().enumerate() {
                        self.tree
                            .mov_to(self.nodes[member], TreeParentId::Node(node), i)?;
                    }
                    applied.created.push(id);
                    // Members may have left groups of their own behind.
                    self.prune_empty_groups(TreeParentId::Root)?;
                }
            }
            Command::Ungroup(ids) => {
                self.check_exist(&ids)?;
                for id in ids {
                    let Some(group) = self.groups.get(&id) else {
                        continue;
                    };
                    applied.ungrouped.extend(&group.children);
                    let node = self.nodes[&id];
                    // All children, including kinds this version does not know.
                    let mut prev = node;
                    for child in self.tree.children(node).unwrap_or_default() {
                        self.tree.mov_after(child, prev)?;
                        prev = child;
                    }
                    self.tree.delete(node)?;
                }
            }
            Command::SetStyle { ids, change } => {
                self.check_exist(&ids)?;
                let mut done = HashSet::new();
                for &id in &ids {
                    for object in self.leaves(id) {
                        if done.insert(object.id) {
                            let meta = self.tree.get_meta(self.nodes[&object.id])?;
                            let change = change.for_kind(&object.data.kind);
                            write_style_change(&meta, &change, object.data.style)?;
                        }
                    }
                }
            }
            Command::SetText {
                id,
                text,
                spans,
                bounds,
            } => {
                if self.get(id).and_then(|o| o.data.kind.text()).is_none() {
                    return Err(Error::UnknownObject(id));
                }
                let meta = self.tree.get_meta(self.nodes[&id])?;
                meta.insert(key::TEXT, &*text)?;
                write_spans(&meta, &rich::fit_to(&spans, &text))?;
                write_bounds(&meta, bounds.abs())?;
            }
            Command::SetCrop { id, crop, bounds } => {
                if !matches!(
                    self.get(id).map(|o| &o.data.kind),
                    Some(ObjectKind::Image(_))
                ) {
                    return Err(Error::UnknownObject(id));
                }
                let meta = self.tree.get_meta(self.nodes[&id])?;
                write_crop(&meta, crop)?;
                write_bounds(&meta, bounds.abs())?;
            }
            Command::SetConnector { ids, change } => {
                self.check_exist(&ids)?;
                for id in ids {
                    let Some(ObjectKind::Connector(c)) = self.get(id).map(|o| &o.data.kind) else {
                        continue;
                    };
                    let meta = self.tree.get_meta(self.nodes[&id])?;
                    write_connector(&meta, &change.apply_to(c), &change)?;
                }
            }
            Command::SetBends { id, bends } => {
                if !matches!(
                    self.get(id).map(|o| &o.data.kind),
                    Some(ObjectKind::Connector(_))
                ) {
                    return Err(Error::UnknownObject(id));
                }
                write_bends(&self.tree.get_meta(self.nodes[&id])?, &bends)?;
            }
            Command::SetCell { id, row, col, cell } => {
                if !matches!(
                    self.get(id).map(|o| &o.data.kind),
                    Some(ObjectKind::Table(_))
                ) {
                    return Err(Error::UnknownObject(id));
                }
                let meta = self.tree.get_meta(self.nodes[&id])?;
                write_cell(&meta, &row, &col, &cell)?;
            }
            Command::SetTable {
                id,
                rows,
                cols,
                merges,
                bounds,
            } => {
                if !matches!(
                    self.get(id).map(|o| &o.data.kind),
                    Some(ObjectKind::Table(_))
                ) || rows.is_empty()
                    || cols.is_empty()
                {
                    return Err(Error::UnknownObject(id));
                }
                let meta = self.tree.get_meta(self.nodes[&id])?;
                meta.insert(key::ROWS, lines_to_string(&rows))?;
                meta.insert(key::COLS, lines_to_string(&cols))?;
                write_merges(&meta, &merges)?;
                write_bounds(&meta, bounds.abs())?;
            }
            Command::SetLabel { id, label } => {
                if !matches!(
                    self.get(id).map(|o| &o.data.kind),
                    Some(ObjectKind::Connector(_))
                ) {
                    return Err(Error::UnknownObject(id));
                }
                write_label(&self.tree.get_meta(self.nodes[&id])?, &label)?;
            }
            Command::SetLocked { ids, locked } => {
                self.check_exist(&ids)?;
                for id in ids {
                    self.tree
                        .get_meta(self.nodes[&id])?
                        .insert(key::LOCKED, locked)?;
                }
            }
        }
        self.doc.commit();
        self.refresh();
        Ok(applied)
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Starts merging the following commands into one undo step, e.g. while a slider is
    /// dragged. Ends with [`Document::end_undo_group`].
    pub fn begin_undo_group(&mut self) {
        if let Err(err) = self.undo.group_start() {
            // Already in a group: the commands still merge into it.
            log::debug!("undo group not started: {err}");
        }
    }

    pub fn end_undo_group(&mut self) {
        self.undo.group_end();
    }

    /// Reverts the last local command. Returns false if there was nothing to undo.
    pub fn undo(&mut self) -> Result<bool> {
        let done = self.undo.undo()?;
        if done {
            self.refresh();
        }
        Ok(done)
    }

    /// Re-applies the last undone command. Returns false if there was nothing to redo.
    pub fn redo(&mut self) -> Result<bool> {
        let done = self.undo.redo()?;
        if done {
            self.refresh();
        }
        Ok(done)
    }

    /// Creates `content` under `parent`. `made` collects created objects with the object
    /// each copies and their connector, for [`Self::attach_copies`].
    fn create(
        &self,
        parent: TreeParentId,
        content: &Content,
        made: &mut Vec<Made>,
    ) -> Result<ObjectId> {
        let node = self.tree.create(parent)?;
        let id = ObjectId::from_node(node);
        let meta = self.tree.get_meta(node)?;
        meta.insert(key::ID, id.to_string())?;
        let (from, data) = match content {
            Content::Object(data) => (None, data),
            Content::Copied { from, data } => (Some(*from), data),
            Content::Group(children) => {
                meta.insert(key::KIND, key::GROUP)?;
                for child in children {
                    self.create(TreeParentId::Node(node), child, made)?;
                }
                return Ok(id);
            }
        };
        write_data(&meta, data)?;
        let connector = match &data.kind {
            ObjectKind::Connector(c) => Some(c.clone()),
            _ => None,
        };
        made.push(Made {
            id,
            node,
            from,
            connector,
        });
        Ok(id)
    }

    /// Points copied connectors at the copies of their objects. Ends of copied connectors
    /// whose object was not copied come loose where they are.
    fn attach_copies(&self, made: &[Made]) -> Result<()> {
        let copies: HashMap<_, _> = made
            .iter()
            .filter_map(|m| m.from.map(|from| (from, m.id)))
            .collect();
        for m in made {
            let Some(c) = &m.connector else {
                continue;
            };
            let fix = |end: End| match end {
                End::Attached { object, anchor, at } => match copies.get(&object) {
                    Some(&copy) => End::Attached {
                        object: copy,
                        anchor,
                        at,
                    },
                    None if m.from.is_some() => End::Free(at),
                    None => end,
                },
                end => end,
            };
            let (start, end) = (fix(c.start), fix(c.end));
            if (start, end) != (c.start, c.end) {
                let meta = self.tree.get_meta(m.node)?;
                write_end(&meta, key::START, &start)?;
                write_end(&meta, key::END, &end)?;
            }
        }
        Ok(())
    }

    /// Before `deleted` (objects or groups) go, connectors attached to them that stay
    /// remember where their ends are, to stay there.
    fn remember_connector_ends(&self, deleted: &[ObjectId]) -> Result<()> {
        let gone: HashSet<_> = deleted
            .iter()
            .flat_map(|&id| self.leaves(id))
            .map(|o| o.id)
            .collect();
        for object in &self.objects {
            let ObjectKind::Connector(c) = &object.data.kind else {
                continue;
            };
            if gone.contains(&object.id) {
                continue;
            }
            let Some((s, e)) = self.connector_ends(object.id) else {
                continue;
            };
            let meta = self.tree.get_meta(self.nodes[&object.id])?;
            for (prefix, end, point) in [(key::START, c.start, s.point), (key::END, c.end, e.point)]
            {
                if end.object().is_some_and(|o| gone.contains(&o)) {
                    write_end(&meta, prefix, &with_position(end, point))?;
                }
            }
        }
        Ok(())
    }

    fn move_each(&self, moves: &[(ObjectId, Vec2)]) -> Result<()> {
        let ids: Vec<_> = moves.iter().map(|(id, _)| *id).collect();
        self.check_exist(&ids)?;
        let mut moved = HashSet::new();
        for &(id, delta) in moves {
            for object in self.leaves(id) {
                if moved.insert(object.id) {
                    let meta = self.tree.get_meta(self.nodes[&object.id])?;
                    let mut data = object.data.clone();
                    data.translate(delta);
                    // Free ends move; attached ones follow their objects (their last known
                    // position moves along, which is right when the objects move too).
                    if let ObjectKind::Connector(c) = &data.kind {
                        write_end(&meta, key::START, &c.start)?;
                        write_end(&meta, key::END, &c.end)?;
                        write_bends(&meta, &c.bends)?;
                    }
                    write_bounds(&meta, data.bounds)?;
                }
            }
        }
        Ok(())
    }

    fn reorder_siblings(
        &self,
        parent: Option<ObjectId>,
        ids: &[ObjectId],
        to: ZOrder,
    ) -> Result<()> {
        let current = match parent {
            None => &self.roots,
            Some(group) => &self.groups[&group].children,
        };
        let target = reorder(current, ids, to);
        // Walk the target order front to back, placing each moved object right after its new
        // predecessor; everything before it is then already final.
        for (i, id) in target.iter().enumerate() {
            if !ids.contains(id) {
                continue;
            }
            let node = self.nodes[id];
            match i {
                0 => self.tree.mov_to(node, self.parent_node(parent), 0)?,
                _ => self.tree.mov_after(node, self.nodes[&target[i - 1]])?,
            }
        }
        Ok(())
    }

    fn parent_node(&self, parent: Option<ObjectId>) -> TreeParentId {
        match parent {
            None => TreeParentId::Root,
            Some(group) => TreeParentId::Node(self.nodes[&group]),
        }
    }

    /// Paint position of the frontmost object making up `id`.
    fn paint_rank(&self, id: ObjectId) -> usize {
        self.leaves(id).last().map_or(0, |o| self.index[&o.id])
    }

    /// Validates up front so a failing command leaves the document untouched.
    fn check_exist(&self, ids: &[ObjectId]) -> Result<()> {
        match ids.iter().find(|&&id| !self.contains(id)) {
            Some(&id) => Err(Error::UnknownObject(id)),
            None => Ok(()),
        }
    }

    /// Checks that all `ids` exist, then drops duplicates and ids inside other listed groups.
    fn outermost(&self, ids: &[ObjectId]) -> Result<Vec<ObjectId>> {
        self.check_exist(ids)?;
        let listed: HashSet<_> = ids.iter().copied().collect();
        let mut seen = HashSet::new();
        Ok(ids
            .iter()
            .copied()
            .filter(|&id| {
                let mut ancestor = self.parent(id);
                while let Some(a) = ancestor {
                    if listed.contains(&a) {
                        return false;
                    }
                    ancestor = self.parent(a);
                }
                seen.insert(id)
            })
            .collect())
    }

    /// Deletes groups left without children. Returns whether `parent` still has children.
    fn prune_empty_groups(&self, parent: TreeParentId) -> Result<bool> {
        let mut any = false;
        for node in self.tree.children(parent).unwrap_or_default() {
            let is_group = is_group(&self.tree.get_meta(node)?);
            if is_group && !self.prune_empty_groups(TreeParentId::Node(node))? {
                self.tree.delete(node)?;
            } else {
                any = true;
            }
        }
        Ok(any)
    }

    /// Rebuilds the cache from the CRDT.
    ///
    /// TODO: update incrementally from Loro diff events once boards get large.
    fn refresh(&mut self) {
        self.objects.clear();
        self.index.clear();
        self.groups.clear();
        self.nodes.clear();
        let changes = self
            .changes
            .lock()
            .map(|mut c| std::mem::take(&mut *c))
            .unwrap_or(Changes {
                all: true,
                ..Changes::default()
            });
        if changes.all {
            self.node_cache.clear();
        } else {
            for node in &changes.nodes {
                self.node_cache.remove(node);
            }
        }
        self.roots = self.read_children(TreeParentId::Root, None);
        // Everything but connectors first: connectors route around it.
        let placed_objects = self
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| !matches!(o.data.kind, ObjectKind::Connector(_)))
            .map(|(i, o)| placed(i, &o.data))
            .collect();
        self.spatial = RTree::bulk_load(placed_objects);
        self.resolve_connectors();
        for (i, o) in self.objects.iter().enumerate() {
            if matches!(o.data.kind, ObjectKind::Connector(_)) {
                self.spatial.insert(placed(i, &o.data));
            }
        }
        self.revision += 1;
    }

    /// An id for a new table row or column, unique across replicas.
    pub fn new_line_id(&self) -> String {
        let n = self.line_ids.get() + 1;
        self.line_ids.set(n);
        format!("{:x}.{n}", self.doc.peer_id())
    }

    /// Where object `id` is in paint order (its index in [`Self::objects`]).
    pub fn paint_order(&self, id: ObjectId) -> Option<usize> {
        self.index.get(&id).copied()
    }

    /// Objects whose bounds (outline included) reach into `area`, back to front.
    pub fn objects_in(&self, area: Rect) -> Vec<&Object> {
        let mut slots: Vec<usize> = self
            .spatial
            .locate_in_envelope_intersecting(&envelope(area))
            .map(|p| p.slot)
            .collect();
        slots.sort_unstable();
        slots.into_iter().map(|i| &self.objects[i]).collect()
    }

    /// The bounds of the obstacles (see [`connector::obstacle`]) reaching into `area`.
    fn obstacles_in(&self, area: Rect) -> Vec<Rect> {
        self.objects_in(area)
            .into_iter()
            .filter_map(|o| connector::obstacle(&o.data))
            .collect()
    }

    /// Works out where connectors run now, and gives them bounds to match.
    fn resolve_connectors(&mut self) {
        self.connectors.clear();
        let lookup = |id| self.get(id).map(|o| &o.data);
        let resolved: Vec<_> = self
            .objects
            .iter()
            .enumerate()
            .filter_map(|(i, o)| match &o.data.kind {
                ObjectKind::Connector(c) => {
                    let width = o.data.style.stroke_width;
                    let (s, e, path) =
                        connector::layout(c, width, lookup, |area| self.obstacles_in(area));
                    let label = c.label.rect(&path);
                    Some((i, o.id, s, e, path, label))
                }
                _ => None,
            })
            .collect();
        for (i, id, start, end, path, label) in resolved {
            let pad = Label::PADDING;
            self.objects[i].data.bounds =
                label.map_or(path.bounds(), |r| path.bounds().union(r.inflate(pad, pad)));
            let resolved = Resolved {
                start,
                end,
                path,
                label,
            };
            self.connectors.insert(id, resolved);
        }
    }

    /// What moves and copies with frame `id`: the top-level objects and groups whose middle
    /// is inside it, including smaller frames (and so, through them, what those hold).
    /// Empty for anything but a frame.
    pub fn frame_members(&self, id: ObjectId) -> Vec<ObjectId> {
        let Some(frame) = self.get(id).filter(|o| o.data.kind.is_frame()) else {
            return Vec::new();
        };
        let area = frame.data.bounds;
        let size = area.area();
        let own = self.top_level(id);
        self.roots
            .iter()
            .copied()
            .filter(|&root| root != own)
            .filter(|&root| {
                let leaves = self.leaves(root);
                let Some(bounds) = leaves
                    .iter()
                    .map(|o| o.data.frame().aabb())
                    .reduce(|a, b| a.union(b))
                else {
                    return false;
                };
                // Frames hold only smaller frames, so two never hold each other.
                let fits = leaves
                    .iter()
                    .all(|o| !o.data.kind.is_frame() || o.data.bounds.area() < size);
                fits && area.contains(bounds.center())
            })
            .collect()
    }

    /// `ids` with the members of the frames among them, each once, in the given order
    /// followed by the members.
    pub fn with_frame_members(&self, ids: &[ObjectId]) -> Vec<ObjectId> {
        let mut out = ids.to_vec();
        let mut i = 0;
        while i < out.len() {
            let id = out[i];
            for member in self.frame_members(id) {
                if !out.contains(&member) {
                    out.push(member);
                }
            }
            i += 1;
        }
        out
    }

    /// Caches the subtree under `parent` and returns the ids of its direct children.
    fn read_children(&mut self, parent: TreeParentId, group: Option<ObjectId>) -> Vec<ObjectId> {
        let mut ids = Vec::new();
        for node in self.tree.children(parent).unwrap_or_default() {
            let Some(read) = self.node_data(node) else {
                continue;
            };
            let id = match &read {
                NodeData::Group { id, .. }
                | NodeData::Object { id, .. }
                | NodeData::Unknown { id } => *id,
            };
            // Two replicas undoing the same deletion can each restore the object; keep one.
            if self.nodes.contains_key(&id) {
                continue;
            }
            if let NodeData::Group { locked, .. } = read {
                self.nodes.insert(id, node);
                let children = self.read_children(TreeParentId::Node(node), Some(id));
                // A group emptied by a concurrent edit has nothing to show or select.
                if children.is_empty() {
                    continue;
                }
                self.groups.insert(
                    id,
                    Group {
                        id,
                        parent: group,
                        children,
                        locked,
                    },
                );
            } else {
                // Objects of kinds this version does not know are skipped, not dropped.
                let NodeData::Object { data, locked, .. } = read else {
                    continue;
                };
                self.nodes.insert(id, node);
                self.index.insert(id, self.objects.len());
                self.objects.push(Object {
                    id,
                    data: *data,
                    parent: group,
                    locked,
                });
            }
            ids.push(id);
        }
        ids
    }

    /// What `node` holds: cached, or read from the CRDT if it changed since.
    fn node_data(&mut self, node: TreeID) -> Option<NodeData> {
        if let Some(cached) = self.node_cache.get(&node) {
            return Some(cached.clone());
        }
        let meta = self.tree.get_meta(node).ok()?;
        let id = match meta.get(key::ID) {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => ObjectId::parse(&s),
            _ => None,
        }
        .unwrap_or_else(|| ObjectId::from_node(node));
        let locked = matches!(
            meta.get(key::LOCKED),
            Some(ValueOrContainer::Value(LoroValue::Bool(true)))
        );
        let read = if is_group(&meta) {
            NodeData::Group { id, locked }
        } else {
            match read_data(&meta) {
                Some(data) => NodeData::Object {
                    id,
                    data: Box::new(data),
                    locked,
                },
                None => NodeData::Unknown { id },
            }
        };
        self.node_cache.insert(node, read.clone());
        Some(read)
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

fn is_group(meta: &LoroMap) -> bool {
    matches!(
        meta.get(key::KIND),
        Some(ValueOrContainer::Value(LoroValue::String(s))) if *s == key::GROUP
    )
}

/// Writes inline styles, or removes them when there are none.
fn write_spans(meta: &LoroMap, spans: &[Span]) -> Result<()> {
    if spans.is_empty() {
        if meta.get(key::MARKS).is_some() {
            meta.delete(key::MARKS)?;
        }
        Ok(())
    } else {
        meta.insert(key::MARKS, rich::to_bytes(spans))?;
        Ok(())
    }
}

/// Inline styles stored with a text, fitted to it.
fn read_spans(meta: &LoroMap, text: &str) -> Spans {
    match meta.get(key::MARKS) {
        Some(ValueOrContainer::Value(LoroValue::Binary(bytes))) => {
            rich::fit_to(&rich::from_bytes(&bytes), text).into()
        }
        _ => Spans::default(),
    }
}

fn write_data(meta: &LoroMap, data: &ObjectData) -> Result<()> {
    meta.insert(key::KIND, data.kind.as_str())?;
    let spans = data.kind.spans();
    if !spans.is_empty() {
        write_spans(meta, spans)?;
    }
    match &data.kind {
        ObjectKind::Ink(ink) => meta.insert(key::POINTS, ink.to_bytes())?,
        ObjectKind::Text(text) => {
            meta.insert(key::TEXT, &*text.text)?;
            meta.insert(key::FONT_SIZE, text.size)?;
        }
        ObjectKind::Sticky(sticky) => meta.insert(key::TEXT, &*sticky.text)?,
        ObjectKind::Frame(frame) => meta.insert(key::TEXT, &*frame.title)?,
        ObjectKind::Table(table) => {
            meta.insert(key::ROWS, lines_to_string(&table.rows))?;
            meta.insert(key::COLS, lines_to_string(&table.cols))?;
            write_merges(meta, &table.merges)?;
            for ((row, col), cell) in table.cells.iter() {
                write_cell(meta, row, col, cell)?;
            }
        }
        ObjectKind::Image(image) => {
            meta.insert(key::ASSET, image.asset.to_string())?;
            write_crop(meta, image.crop)?;
        }
        ObjectKind::Connector(c) => {
            let all = crate::ConnectorChange {
                start: Some(c.start),
                end: Some(c.end),
                route: Some(c.route),
                start_cap: Some(c.start_cap),
                end_cap: Some(c.end_cap),
            };
            write_connector(meta, c, &all)?;
            write_label(meta, &c.label)?;
            write_bends(meta, &c.bends)?;
        }
        ObjectKind::Shape(shape) => {
            if !shape.text.is_empty() {
                meta.insert(key::TEXT, &*shape.text)?;
            }
        }
    }
    write_bounds(meta, data.bounds.abs())?;
    meta.insert(key::ROTATION, normalize_angle(data.rotation))?;
    let style = data.style;
    let all = StyleChange {
        fill: Some(style.fill),
        stroke: Some(style.stroke),
        stroke_width: Some(style.stroke_width),
        opacity: Some(style.opacity),
        text_color: style.text_color,
        font_size: style.font_size.map(Some),
        text_align: Some(style.text_align),
        text_valign: Some(style.text_valign),
    };
    write_style_change(meta, &all.for_kind(&data.kind), style)
}

/// Writes only the properties the change sets, each under its own key.
fn write_style_change(meta: &LoroMap, change: &StyleChange, current: Style) -> Result<()> {
    let style = change.apply_to(current);
    if change.fill.is_some() {
        meta.insert(key::FILL, i64::from(style.fill.0))?;
    }
    if change.stroke.is_some() {
        meta.insert(key::STROKE, i64::from(style.stroke.0))?;
    }
    if change.stroke_width.is_some() {
        meta.insert(key::STROKE_WIDTH, style.stroke_width)?;
    }
    if change.opacity.is_some() {
        meta.insert(key::OPACITY, style.opacity)?;
    }
    if let Some(color) = style.text_color
        && change.text_color.is_some()
    {
        meta.insert(key::TEXT_COLOR, i64::from(color.0))?;
    }
    if change.font_size.is_some() {
        match style.font_size {
            Some(size) => meta.insert(key::TEXT_SIZE, size)?,
            None => meta.delete(key::TEXT_SIZE)?,
        }
    }
    if change.text_align.is_some() {
        meta.insert(key::TEXT_ALIGN, style.text_align.as_str())?;
    }
    if change.text_valign.is_some() {
        meta.insert(key::TEXT_VALIGN, style.text_valign.as_str())?;
    }
    Ok(())
}

/// An end at `point`: a free end moves there, an attached one remembers it.
fn with_position(end: End, point: Point) -> End {
    match end {
        End::Free(_) => End::Free(point),
        End::Attached { object, anchor, .. } => End::Attached {
            object,
            anchor,
            at: point,
        },
    }
}

/// `p` placed the same way relative to `to` as it was relative to the rectangle `from`.
fn map_point(p: Point, from: Rect, to: pb_geom::Frame) -> Point {
    let rel = |v: f64, min: f64, size: f64| if size > 0.0 { (v - min) / size } else { 0.5 };
    let r = to.rect;
    let local = Point::new(
        r.x0 + rel(p.x, from.x0, from.width()) * r.width(),
        r.y0 + rel(p.y, from.y0, from.height()) * r.height(),
    );
    to.to_world(local)
}

fn route_name(route: Route) -> &'static str {
    match route {
        Route::Straight => "straight",
        Route::Elbow => "elbow",
        Route::Curved => "curved",
    }
}

fn cap_name(cap: Cap) -> &'static str {
    match cap {
        Cap::None => "none",
        Cap::Arrow => "arrow",
    }
}

/// Writes the properties `change` sets, taking their values from `c`.
fn write_connector(meta: &LoroMap, c: &Connector, change: &crate::ConnectorChange) -> Result<()> {
    if change.start.is_some() {
        write_end(meta, key::START, &c.start)?;
    }
    if change.end.is_some() {
        write_end(meta, key::END, &c.end)?;
    }
    if change.route.is_some() {
        meta.insert(key::ROUTE, route_name(c.route))?;
        write_bends(meta, &c.bends)?;
    }
    if change.start_cap.is_some() {
        meta.insert(key::START_CAP, cap_name(c.start_cap))?;
    }
    if change.end_cap.is_some() {
        meta.insert(key::END_CAP, cap_name(c.end_cap))?;
    }
    Ok(())
}

/// `id=weight;id=weight`, see `key::ROWS`.
fn lines_to_string(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|l| format!("{}={}", l.id, l.weight))
        .collect::<Vec<_>>()
        .join(";")
}

fn lines_from_string(s: &str) -> Vec<Line> {
    s.split(';')
        .filter_map(|part| {
            let (id, weight) = part.split_once('=')?;
            let weight: f64 = weight.parse().ok()?;
            (!id.is_empty() && weight.is_finite() && weight > 0.0).then(|| Line {
                id: id.into(),
                weight,
            })
        })
        .collect()
}

/// Writes a table's merged cells, or removes them when there are none.
fn write_merges(meta: &LoroMap, merges: &[crate::Merge]) -> Result<()> {
    if merges.is_empty() {
        if meta.get(key::MERGES).is_some() {
            meta.delete(key::MERGES)?;
        }
        return Ok(());
    }
    let text: Vec<String> = merges
        .iter()
        .map(|m| format!("{},{},{},{}", m.row0, m.col0, m.row1, m.col1))
        .collect();
    meta.insert(key::MERGES, text.join(";"))?;
    Ok(())
}

fn read_merges(meta: &LoroMap) -> Arc<[crate::Merge]> {
    let Some(text) = get_str(meta, key::MERGES) else {
        return Arc::new([]);
    };
    text.split(';')
        .filter_map(|part| {
            let mut ids = part.split(',').filter(|id| !id.is_empty()).map(Arc::from);
            let merge = crate::Merge {
                row0: ids.next()?,
                col0: ids.next()?,
                row1: ids.next()?,
                col1: ids.next()?,
            };
            ids.next().is_none().then_some(merge)
        })
        .collect()
}

/// Writes a table cell's text and styles; empty text removes them.
fn write_cell(meta: &LoroMap, row: &str, col: &str, cell: &crate::Cell) -> Result<()> {
    let text_key = format!("{}{row}:{col}", key::CELL);
    let marks_key = format!("{}{row}:{col}", key::CELL_MARKS);
    if cell.text.is_empty() {
        for k in [&text_key, &marks_key] {
            if meta.get(k).is_some() {
                meta.delete(k)?;
            }
        }
        return Ok(());
    }
    meta.insert(&text_key, &*cell.text)?;
    let spans = rich::fit_to(&cell.spans, &cell.text);
    if spans.is_empty() {
        if meta.get(&marks_key).is_some() {
            meta.delete(&marks_key)?;
        }
    } else {
        meta.insert(&marks_key, rich::to_bytes(&spans))?;
    }
    Ok(())
}

fn read_table(meta: &LoroMap) -> Option<crate::Table> {
    let rows = lines_from_string(&get_str(meta, key::ROWS)?);
    let cols = lines_from_string(&get_str(meta, key::COLS)?);
    if rows.is_empty() || cols.is_empty() {
        return None;
    }
    let mut texts = Vec::new();
    meta.for_each(|k, v| {
        if let (Some(ids), ValueOrContainer::Value(LoroValue::String(text))) =
            (k.strip_prefix(key::CELL), v)
            && let Some((row, col)) = ids.split_once(':')
        {
            texts.push((row.to_owned(), col.to_owned(), text.to_string()));
        }
    });
    let cells = texts
        .into_iter()
        .map(|(row, col, text)| {
            let marks_key = format!("{}{row}:{col}", key::CELL_MARKS);
            let spans: Spans = match meta.get(&marks_key) {
                Some(ValueOrContainer::Value(LoroValue::Binary(bytes))) => {
                    rich::fit_to(&rich::from_bytes(&bytes), &text).into()
                }
                _ => Spans::default(),
            };
            let cell = crate::Cell {
                text: text.into(),
                spans,
            };
            ((Arc::from(row), Arc::from(col)), cell)
        })
        .collect();
    Some(crate::Table {
        rows: rows.into(),
        cols: cols.into(),
        cells: Arc::new(cells),
        merges: read_merges(meta),
    })
}

/// Writes a connector's hand-set bends, or removes them when there are none.
fn write_bends(meta: &LoroMap, bends: &[Point]) -> Result<()> {
    if bends.is_empty() {
        if meta.get(key::BENDS).is_some() {
            meta.delete(key::BENDS)?;
        }
        return Ok(());
    }
    let mut bytes = Vec::with_capacity(bends.len() * 16);
    for p in bends {
        bytes.extend_from_slice(&p.x.to_le_bytes());
        bytes.extend_from_slice(&p.y.to_le_bytes());
    }
    meta.insert(key::BENDS, bytes)?;
    Ok(())
}

fn read_bends(meta: &LoroMap) -> Arc<[Point]> {
    let Some(ValueOrContainer::Value(LoroValue::Binary(bytes))) = meta.get(key::BENDS) else {
        return Arc::from([]);
    };
    let (chunks, _) = bytes.as_chunks::<16>();
    chunks
        .iter()
        .map(|c| {
            let f = |b: &[u8]| f64::from_le_bytes(b.try_into().expect("8 bytes"));
            Point::new(f(&c[0..8]), f(&c[8..16]))
        })
        .filter(|p| p.x.is_finite() && p.y.is_finite())
        .collect()
}

fn write_label(meta: &LoroMap, label: &Label) -> Result<()> {
    meta.insert(key::TEXT, &*label.text)?;
    meta.insert(key::LABEL_AT, label.at.clamp(0.0, 1.0))?;
    meta.insert(key::LABEL_W, label.size.width)?;
    meta.insert(key::LABEL_H, label.size.height)?;
    Ok(())
}

fn read_label(meta: &LoroMap) -> Label {
    let Some(text) = get_str(meta, key::TEXT) else {
        return Label::default();
    };
    let num = |k| get_num(meta, k).unwrap_or(0.0);
    Label {
        text: text.into(),
        at: get_num(meta, key::LABEL_AT).unwrap_or(0.5).clamp(0.0, 1.0),
        size: kurbo::Size::new(num(key::LABEL_W), num(key::LABEL_H)),
    }
}

fn write_end(meta: &LoroMap, prefix: &str, end: &End) -> Result<()> {
    let k = |suffix: &str| format!("{prefix}{suffix}");
    let p = end.point();
    meta.insert(&k(key::X_SUFFIX), p.x)?;
    meta.insert(&k(key::Y_SUFFIX), p.y)?;
    let anchor = match end {
        End::Free(_) => {
            meta.delete(&k(key::OBJECT_SUFFIX))?;
            None
        }
        End::Attached { object, anchor, .. } => {
            meta.insert(&k(key::OBJECT_SUFFIX), object.to_string())?;
            match anchor {
                Anchor::Auto => None,
                Anchor::Fixed(f) => Some(*f),
            }
        }
    };
    match anchor {
        Some(f) => {
            meta.insert(&k(key::ANCHOR_X_SUFFIX), f.x)?;
            meta.insert(&k(key::ANCHOR_Y_SUFFIX), f.y)?;
        }
        None => {
            meta.delete(&k(key::ANCHOR_X_SUFFIX))?;
            meta.delete(&k(key::ANCHOR_Y_SUFFIX))?;
        }
    }
    Ok(())
}

fn get_num(meta: &LoroMap, k: &str) -> Option<f64> {
    match meta.get(k) {
        Some(ValueOrContainer::Value(LoroValue::Double(v))) => Some(v),
        Some(ValueOrContainer::Value(LoroValue::I64(v))) => Some(v as f64),
        _ => None,
    }
}

fn get_str(meta: &LoroMap, k: &str) -> Option<String> {
    match meta.get(k) {
        Some(ValueOrContainer::Value(LoroValue::String(s))) => Some(s.to_string()),
        _ => None,
    }
}

fn read_end(meta: &LoroMap, prefix: &str) -> Option<End> {
    let k = |suffix: &str| format!("{prefix}{suffix}");
    let point = Point::new(
        get_num(meta, &k(key::X_SUFFIX))?,
        get_num(meta, &k(key::Y_SUFFIX))?,
    );
    let Some(object) = get_str(meta, &k(key::OBJECT_SUFFIX)) else {
        return Some(End::Free(point));
    };
    let anchor = match (
        get_num(meta, &k(key::ANCHOR_X_SUFFIX)),
        get_num(meta, &k(key::ANCHOR_Y_SUFFIX)),
    ) {
        (Some(u), Some(v)) => Anchor::Fixed(Point::new(u, v)),
        _ => Anchor::Auto,
    };
    Some(End::Attached {
        object: ObjectId::parse(&object)?,
        anchor,
        at: point,
    })
}

fn read_connector(meta: &LoroMap) -> Option<Connector> {
    let route = match get_str(meta, key::ROUTE).as_deref() {
        Some("elbow") => Route::Elbow,
        Some("curved") => Route::Curved,
        _ => Route::Straight,
    };
    let cap = |k| match get_str(meta, k).as_deref() {
        Some("arrow") => Cap::Arrow,
        _ => Cap::None,
    };
    Some(Connector {
        start: read_end(meta, key::START)?,
        end: read_end(meta, key::END)?,
        route,
        start_cap: cap(key::START_CAP),
        end_cap: cap(key::END_CAP),
        label: read_label(meta),
        bends: read_bends(meta),
    })
}

fn write_crop(meta: &LoroMap, crop: Rect) -> Result<()> {
    let c = crop.abs();
    meta.insert(key::CROP_X0, c.x0)?;
    meta.insert(key::CROP_Y0, c.y0)?;
    meta.insert(key::CROP_X1, c.x1)?;
    meta.insert(key::CROP_Y1, c.y1)?;
    Ok(())
}

/// Position and size are separate keys so concurrent move and resize both survive a merge.
fn write_bounds(meta: &LoroMap, bounds: Rect) -> Result<()> {
    meta.insert(key::X, bounds.x0)?;
    meta.insert(key::Y, bounds.y0)?;
    meta.insert(key::W, bounds.width())?;
    meta.insert(key::H, bounds.height())?;
    Ok(())
}

fn read_data(meta: &LoroMap) -> Option<ObjectData> {
    let kind = match meta.get(key::KIND)? {
        ValueOrContainer::Value(LoroValue::String(s)) if &**s == "ink" => {
            match meta.get(key::POINTS)? {
                ValueOrContainer::Value(LoroValue::Binary(bytes)) => {
                    ObjectKind::Ink(Ink::from_bytes(&bytes)?)
                }
                _ => return None,
            }
        }
        ValueOrContainer::Value(LoroValue::String(s)) if &**s == "text" => {
            let text = match meta.get(key::TEXT)? {
                ValueOrContainer::Value(LoroValue::String(t)) => Arc::from(&**t),
                _ => return None,
            };
            let size = match meta.get(key::FONT_SIZE) {
                Some(ValueOrContainer::Value(LoroValue::Double(v))) => v,
                _ => DEFAULT_FONT_SIZE,
            };
            let spans = read_spans(meta, &text);
            ObjectKind::Text(Text { text, spans, size })
        }
        ValueOrContainer::Value(LoroValue::String(s)) if &**s == "sticky" => {
            let text = match meta.get(key::TEXT) {
                Some(ValueOrContainer::Value(LoroValue::String(t))) => Arc::from(&**t),
                _ => Arc::from(""),
            };
            let spans = read_spans(meta, &text);
            ObjectKind::Sticky(Sticky { text, spans })
        }
        ValueOrContainer::Value(LoroValue::String(s)) if &**s == "table" => {
            ObjectKind::Table(read_table(meta)?)
        }
        ValueOrContainer::Value(LoroValue::String(s)) if &**s == "frame" => {
            let title = get_str(meta, key::TEXT).unwrap_or_default();
            ObjectKind::Frame(FrameData {
                title: title.into(),
            })
        }
        ValueOrContainer::Value(LoroValue::String(s)) if &**s == "connector" => {
            ObjectKind::Connector(read_connector(meta)?)
        }
        ValueOrContainer::Value(LoroValue::String(s)) if &**s == "image" => {
            let asset = match meta.get(key::ASSET)? {
                ValueOrContainer::Value(LoroValue::String(a)) => AssetId::parse(&a)?,
                _ => return None,
            };
            let num = |k| match meta.get(k) {
                Some(ValueOrContainer::Value(LoroValue::Double(v))) => Some(v),
                _ => None,
            };
            let crop = match (
                num(key::CROP_X0),
                num(key::CROP_Y0),
                num(key::CROP_X1),
                num(key::CROP_Y1),
            ) {
                (Some(x0), Some(y0), Some(x1), Some(y1)) => Rect::new(x0, y0, x1, y1),
                _ => Image::FULL,
            };
            ObjectKind::Image(Image { asset, crop })
        }
        ValueOrContainer::Value(LoroValue::String(s)) => {
            let text = match meta.get(key::TEXT) {
                Some(ValueOrContainer::Value(LoroValue::String(t))) => Arc::from(&**t),
                _ => Arc::from(""),
            };
            let spans = read_spans(meta, &text);
            ObjectKind::Shape(Shape {
                kind: ObjectKind::parse_shape(&s)?,
                text,
                spans,
            })
        }
        _ => return None,
    };
    let num = |k| match meta.get(k) {
        Some(ValueOrContainer::Value(LoroValue::Double(v))) => Some(v),
        Some(ValueOrContainer::Value(LoroValue::I64(v))) => Some(v as f64),
        _ => None,
    };
    let color = |k| num(k).map(|v| Rgba(v as u32));
    let defaults = Style::default();
    let (x, y) = (num(key::X)?, num(key::Y)?);
    Some(ObjectData {
        kind,
        bounds: Rect::new(x, y, x + num(key::W)?, y + num(key::H)?),
        rotation: num(key::ROTATION).unwrap_or(0.0),
        style: Style {
            fill: color(key::FILL).unwrap_or(defaults.fill),
            stroke: color(key::STROKE).unwrap_or(defaults.stroke),
            stroke_width: num(key::STROKE_WIDTH).unwrap_or(defaults.stroke_width),
            opacity: num(key::OPACITY).unwrap_or(defaults.opacity),
            text_color: color(key::TEXT_COLOR),
            font_size: num(key::TEXT_SIZE),
            text_align: get_str(meta, key::TEXT_ALIGN)
                .and_then(|s| HAlign::parse(&s))
                .unwrap_or_default(),
            text_valign: get_str(meta, key::TEXT_VALIGN)
                .and_then(|s| VAlign::parse(&s))
                .unwrap_or_default(),
        },
    })
}

#[cfg(test)]
mod tests {
    use pb_geom::Frame;

    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> ObjectData {
        ObjectData {
            kind: ShapeKind::Rectangle.into(),
            bounds: Rect::new(x, y, x + w, y + h),
            rotation: 0.0,
            style: Style::default(),
        }
    }

    fn create(doc: &mut Document, data: ObjectData) -> ObjectId {
        doc.apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0]
    }

    #[test]
    fn create_reads_back_and_stacks_on_top() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 20.0));
        let b = create(&mut doc, rect(5.0, 5.0, 10.0, 10.0));
        let ids: Vec<_> = doc.objects().iter().map(|o| o.id).collect();
        assert_eq!(ids, [a, b]);
        assert_eq!(doc.get(a).unwrap().data, rect(0.0, 0.0, 10.0, 20.0));
    }

    #[test]
    fn bounds_are_normalized() {
        let mut doc = Document::new();
        let mut data = rect(0.0, 0.0, 0.0, 0.0);
        data.bounds = Rect::new(10.0, 10.0, 0.0, 5.0);
        let id = create(&mut doc, data);
        assert_eq!(
            doc.get(id).unwrap().data.bounds,
            Rect::new(0.0, 5.0, 10.0, 10.0)
        );
    }

    #[test]
    fn move_delete_and_set_bounds() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        let b = create(&mut doc, rect(20.0, 0.0, 10.0, 10.0));
        doc.apply(Command::Move {
            ids: vec![a, b],
            delta: Vec2::new(1.0, 2.0),
        })
        .unwrap();
        assert_eq!(
            doc.get(b).unwrap().data.bounds,
            Rect::new(21.0, 2.0, 31.0, 12.0)
        );
        doc.apply(Command::SetGeometry(vec![(
            a,
            Frame::new(Rect::new(0.0, 0.0, 50.0, 50.0), 0.5),
        )]))
        .unwrap();
        assert_eq!(doc.get(a).unwrap().data.rotation, 0.5);
        assert_eq!(doc.get(a).unwrap().data.bounds.width(), 50.0);
        doc.apply(Command::Delete(vec![a])).unwrap();
        assert!(doc.get(a).is_none());
        assert_eq!(doc.objects().len(), 1);
    }

    #[test]
    fn failing_command_changes_nothing() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        let b = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        doc.apply(Command::Delete(vec![b])).unwrap();
        let revision = doc.revision();
        let err = doc
            .apply(Command::Move {
                ids: vec![a, b],
                delta: Vec2::new(5.0, 5.0),
            })
            .unwrap_err();
        assert!(matches!(err, Error::UnknownObject(id) if id == b));
        assert_eq!(doc.get(a).unwrap().data.bounds.x0, 0.0);
        assert_eq!(doc.revision(), revision);
    }

    #[test]
    fn undo_redo_step_per_command() {
        let mut doc = Document::new();
        assert!(!doc.can_undo());
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        doc.apply(Command::Move {
            ids: vec![a],
            delta: Vec2::new(100.0, 0.0),
        })
        .unwrap();

        assert!(doc.undo().unwrap());
        assert_eq!(doc.get(a).unwrap().data.bounds.x0, 0.0);
        assert!(doc.undo().unwrap());
        assert!(doc.objects().is_empty());
        assert!(!doc.undo().unwrap());

        assert!(doc.redo().unwrap());
        assert_eq!(doc.objects().len(), 1);
        assert!(doc.redo().unwrap());
        assert_eq!(doc.objects()[0].data.bounds.x0, 100.0);
        assert!(!doc.can_redo());
    }

    #[test]
    fn undo_restores_deleted_object_with_same_id() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        doc.apply(Command::Delete(vec![a])).unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.get(a).unwrap().data, rect(0.0, 0.0, 10.0, 10.0));
    }

    #[test]
    fn redo_of_create_keeps_id() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        doc.undo().unwrap();
        doc.redo().unwrap();
        assert!(doc.get(a).is_some());
        // The restored object is still editable through its original id.
        doc.apply(Command::Move {
            ids: vec![a],
            delta: Vec2::new(1.0, 0.0),
        })
        .unwrap();
        assert_eq!(doc.get(a).unwrap().data.bounds.x0, 1.0);
    }

    #[test]
    fn object_id_roundtrips_through_string() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 1.0, 1.0));
        assert_eq!(ObjectId::parse(&a.to_string()), Some(a));
        assert_eq!(ObjectId::parse("garbage"), None);
    }

    fn order(doc: &Document) -> Vec<ObjectId> {
        doc.objects().iter().map(|o| o.id).collect()
    }

    #[test]
    fn create_many_is_one_undo_step() {
        let mut doc = Document::new();
        let ids = doc
            .apply(Command::Create(vec![
                rect(0.0, 0.0, 1.0, 1.0).into(),
                rect(5.0, 0.0, 1.0, 1.0).into(),
            ]))
            .unwrap()
            .created;
        assert_eq!(order(&doc), ids);
        doc.undo().unwrap();
        assert!(doc.objects().is_empty());
    }

    #[test]
    fn reorder_all_directions() {
        let mut doc = Document::new();
        let ids: Vec<_> = (0..4)
            .map(|i| create(&mut doc, rect(i as f64, 0.0, 1.0, 1.0)))
            .collect();
        let [a, b, c, d] = [ids[0], ids[1], ids[2], ids[3]];
        let mut reorder = |ids: Vec<ObjectId>, to| {
            doc.apply(Command::Reorder { ids, to }).unwrap();
            order(&doc)
        };
        assert_eq!(reorder(vec![b, a], ZOrder::Front), [c, d, a, b]);
        assert_eq!(reorder(vec![b, d], ZOrder::Back), [d, b, c, a]);
        assert_eq!(reorder(vec![b], ZOrder::Forward), [d, c, b, a]);
        assert_eq!(reorder(vec![b, a], ZOrder::Backward), [d, b, a, c]);
        // Already at the edge: nothing changes.
        assert_eq!(reorder(vec![d], ZOrder::Back), [d, b, a, c]);
    }

    #[test]
    fn reorder_undo_restores_order_and_ids() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 1.0, 1.0));
        let b = create(&mut doc, rect(0.0, 0.0, 1.0, 1.0));
        doc.apply(Command::Reorder {
            ids: vec![a],
            to: ZOrder::Front,
        })
        .unwrap();
        assert_eq!(order(&doc), [b, a]);
        doc.undo().unwrap();
        assert_eq!(order(&doc), [a, b]);
    }

    #[test]
    fn rotation_is_stored_normalized() {
        let mut doc = Document::new();
        let mut data = rect(0.0, 0.0, 10.0, 10.0);
        data.rotation = 3.0 * std::f64::consts::PI;
        let id = create(&mut doc, data);
        let stored = doc.get(id).unwrap().data.rotation;
        assert!((stored - std::f64::consts::PI).abs() < 1e-9);
    }

    #[test]
    fn snapshot_roundtrip() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(1.0, 2.0, 3.0, 4.0));
        let b = create(&mut doc, rect(5.0, 6.0, 7.0, 8.0));
        let c = create(&mut doc, rect(9.0, 0.0, 1.0, 1.0));
        let g = group(&mut doc, &[b, c]);
        lock(&mut doc, &[g], true);
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.objects(), doc.objects());
        assert_eq!(loaded.roots(), [a, g]);
        assert_eq!(loaded.group(g), doc.group(g));
        assert!(loaded.is_locked(c));
        assert!(!loaded.can_undo());
    }

    #[test]
    fn rejects_newer_schema() {
        let doc = Document::new();
        doc.doc
            .get_map(key::META)
            .insert(key::SCHEMA, SCHEMA_VERSION + 1)
            .unwrap();
        doc.doc.commit();
        assert!(matches!(
            Document::from_snapshot(&doc.snapshot()),
            Err(Error::UnsupportedSchema(_))
        ));
    }

    // Groups and locking

    fn group(doc: &mut Document, ids: &[ObjectId]) -> ObjectId {
        doc.apply(Command::Group(ids.to_vec())).unwrap().created[0]
    }

    fn lock(doc: &mut Document, ids: &[ObjectId], locked: bool) {
        doc.apply(Command::SetLocked {
            ids: ids.to_vec(),
            locked,
        })
        .unwrap();
    }

    fn four(doc: &mut Document) -> [ObjectId; 4] {
        [0.0, 10.0, 20.0, 30.0].map(|x| create(doc, rect(x, 0.0, 5.0, 5.0)))
    }

    #[test]
    fn group_takes_place_of_topmost_member() {
        let mut doc = Document::new();
        let [a, b, c, d] = four(&mut doc);
        // Listed out of order: members still stack as before.
        let g = group(&mut doc, &[c, a]);
        assert_eq!(doc.roots(), [b, g, d]);
        assert_eq!(doc.group(g).unwrap().children, [a, c]);
        assert_eq!(order(&doc), [b, a, c, d]);
        assert_eq!(doc.parent(a), Some(g));
        assert_eq!(doc.top_level(c), g);
        let leaves: Vec<_> = doc.leaves(g).iter().map(|o| o.id).collect();
        assert_eq!(leaves, [a, c]);
    }

    #[test]
    fn group_undo_and_redo_keep_ids() {
        let mut doc = Document::new();
        let [a, b, c, d] = four(&mut doc);
        let g = group(&mut doc, &[b, c]);
        doc.undo().unwrap();
        assert_eq!(doc.roots(), [a, b, c, d]);
        assert!(doc.group(g).is_none());
        doc.redo().unwrap();
        assert_eq!(doc.roots(), [a, g, d]);
        assert_eq!(doc.group(g).unwrap().children, [b, c]);
    }

    #[test]
    fn nested_groups() {
        let mut doc = Document::new();
        let [a, b, c, _] = four(&mut doc);
        let inner = group(&mut doc, &[a, b]);
        let outer = group(&mut doc, &[inner, c]);
        assert_eq!(doc.group(outer).unwrap().children, [inner, c]);
        assert_eq!(doc.top_level(a), outer);
        assert_eq!(doc.leaves(outer).len(), 3);
    }

    #[test]
    fn move_group_moves_members() {
        let mut doc = Document::new();
        let [a, b, ..] = four(&mut doc);
        let g = group(&mut doc, &[a, b]);
        // Listing a member next to its group must not move it twice.
        doc.apply(Command::Move {
            ids: vec![g, a],
            delta: Vec2::new(0.0, 7.0),
        })
        .unwrap();
        assert_eq!(doc.get(a).unwrap().data.bounds.y0, 7.0);
        assert_eq!(doc.get(b).unwrap().data.bounds.y0, 7.0);
        doc.apply(Command::MoveEach(vec![(g, Vec2::new(1.0, 0.0))]))
            .unwrap();
        assert_eq!(doc.get(b).unwrap().data.bounds.x0, 11.0);
    }

    #[test]
    fn delete_group_and_undo() {
        let mut doc = Document::new();
        let [a, b, c, d] = four(&mut doc);
        let g = group(&mut doc, &[b, c]);
        doc.apply(Command::Delete(vec![g])).unwrap();
        assert_eq!(order(&doc), [a, d]);
        doc.undo().unwrap();
        assert_eq!(doc.roots(), [a, g, d]);
        assert_eq!(doc.group(g).unwrap().children, [b, c]);
    }

    #[test]
    fn deleting_all_members_removes_group() {
        let mut doc = Document::new();
        let [a, b, ..] = four(&mut doc);
        let g = group(&mut doc, &[a, b]);
        doc.apply(Command::Delete(vec![a])).unwrap();
        assert_eq!(doc.group(g).unwrap().children, [b]);
        doc.apply(Command::Delete(vec![b])).unwrap();
        assert!(!doc.contains(g));
        assert_eq!(doc.tree.children(TreeParentId::Root).unwrap().len(), 2);
    }

    #[test]
    fn regrouping_drops_emptied_group() {
        let mut doc = Document::new();
        let [a, b, c, _] = four(&mut doc);
        let inner = group(&mut doc, &[a]);
        let outer = group(&mut doc, &[a, b, c]);
        assert!(!doc.contains(inner));
        assert_eq!(doc.group(outer).unwrap().children, [a, b, c]);
    }

    #[test]
    fn ungroup_puts_children_in_place_and_undo_regroups() {
        let mut doc = Document::new();
        let [a, b, c, d] = four(&mut doc);
        let g = group(&mut doc, &[b, c]);
        let applied = doc.apply(Command::Ungroup(vec![g, a])).unwrap();
        assert_eq!(applied.ungrouped, [b, c]);
        assert_eq!(doc.roots(), [a, b, c, d]);
        assert!(!doc.contains(g));
        assert_eq!(doc.parent(b), None);
        doc.undo().unwrap();
        assert_eq!(doc.roots(), [a, g, d]);
        assert_eq!(doc.group(g).unwrap().children, [b, c]);
    }

    #[test]
    fn reorder_inside_group() {
        let mut doc = Document::new();
        let [a, b, c, d] = four(&mut doc);
        let g = group(&mut doc, &[a, b, c]);
        doc.apply(Command::Reorder {
            ids: vec![a],
            to: ZOrder::Front,
        })
        .unwrap();
        assert_eq!(doc.group(g).unwrap().children, [b, c, a]);
        doc.apply(Command::Reorder {
            ids: vec![g],
            to: ZOrder::Front,
        })
        .unwrap();
        assert_eq!(doc.roots(), [d, g]);
    }

    #[test]
    fn lock_is_inherited_from_groups() {
        let mut doc = Document::new();
        let [a, b, c, _] = four(&mut doc);
        let g = group(&mut doc, &[a, b]);
        lock(&mut doc, &[g, c], true);
        assert!(doc.is_locked(a) && doc.is_locked(g) && doc.is_locked(c));
        assert!(!doc.get(a).unwrap().locked);
        lock(&mut doc, &[g], false);
        assert!(!doc.is_locked(a));
        doc.undo().unwrap();
        assert!(doc.is_locked(b));
    }

    #[test]
    fn content_recreates_group_structure() {
        let mut doc = Document::new();
        let [a, b, c, _] = four(&mut doc);
        let inner = group(&mut doc, &[a, b]);
        let outer = group(&mut doc, &[inner, c]);
        let content = doc.content(outer).unwrap();
        assert_eq!(content.shapes().len(), 3);
        let copy = doc.apply(Command::Create(vec![content])).unwrap().created[0];
        let copied = doc.group(copy).unwrap();
        assert_eq!(copied.children.len(), 2);
        assert!(doc.group(copied.children[0]).is_some());
        assert_eq!(doc.objects().len(), 7);
    }

    // Shapes and styles

    #[test]
    fn every_shape_kind_roundtrips() {
        let mut doc = Document::new();
        for shape in ShapeKind::ALL {
            let mut data = rect(0.0, 0.0, 10.0, 10.0);
            data.kind = shape.into();
            let id = create(&mut doc, data);
            assert_eq!(doc.get(id).unwrap().data.kind, shape.into());
        }
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.objects(), doc.objects());
    }

    #[test]
    fn set_style_changes_only_given_properties() {
        let mut doc = Document::new();
        let [a, b, c, _] = four(&mut doc);
        let g = group(&mut doc, &[a, b]);
        let red = Rgba::rgb(0xFF, 0, 0);
        doc.apply(Command::SetStyle {
            ids: vec![g, c],
            change: StyleChange {
                fill: Some(red),
                opacity: Some(7.0),
                ..StyleChange::default()
            },
        })
        .unwrap();
        for id in [a, b, c] {
            let style = doc.get(id).unwrap().data.style;
            assert_eq!(style.fill, red);
            assert_eq!(style.opacity, 1.0);
            assert_eq!(style.stroke, Style::default().stroke);
        }
        doc.undo().unwrap();
        assert_eq!(doc.get(a).unwrap().data.style, Style::default());
    }

    #[test]
    fn concurrent_style_edits_of_different_properties_merge() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        let mut other = Document::from_snapshot(&doc.snapshot()).unwrap();
        let red = Rgba::rgb(0xFF, 0, 0);
        let set = |doc: &mut Document, change| {
            doc.apply(Command::SetStyle {
                ids: vec![a],
                change,
            })
            .unwrap();
        };
        set(
            &mut doc,
            StyleChange {
                fill: Some(red),
                ..StyleChange::default()
            },
        );
        set(
            &mut other,
            StyleChange {
                opacity: Some(0.5),
                ..StyleChange::default()
            },
        );
        doc.doc.import(&other.snapshot()).unwrap();
        doc.refresh();
        let style = doc.get(a).unwrap().data.style;
        assert_eq!((style.fill, style.opacity), (red, 0.5));
    }

    /// The cached view after a mix of edits matches one read afresh from the CRDT.
    #[test]
    fn cache_matches_a_fresh_read() {
        let fresh = |doc: &Document| Document::from_snapshot(&doc.snapshot()).unwrap();
        let same = |doc: &Document| {
            let other = fresh(doc);
            assert_eq!(doc.objects(), other.objects());
            assert_eq!(doc.roots(), other.roots());
        };
        let mut doc = Document::new();
        let [a, b, c, d] = four(&mut doc);
        same(&doc);
        doc.apply(Command::Move {
            ids: vec![a, c],
            delta: Vec2::new(5.0, 5.0),
        })
        .unwrap();
        let g = group(&mut doc, &[b, c]);
        lock(&mut doc, &[d], true);
        same(&doc);
        doc.undo().unwrap();
        same(&doc);
        doc.redo().unwrap();
        doc.apply(Command::Ungroup(vec![g])).unwrap();
        doc.apply(Command::Delete(vec![a])).unwrap();
        same(&doc);
        doc.undo().unwrap();
        same(&doc);
        // Edits from another replica, merged in.
        let mut other = fresh(&doc);
        other
            .apply(Command::Move {
                ids: vec![b],
                delta: Vec2::new(0.0, 100.0),
            })
            .unwrap();
        other.apply(Command::Delete(vec![d])).unwrap();
        create(&mut other, rect(500.0, 500.0, 10.0, 10.0));
        doc.doc.import(&other.snapshot()).unwrap();
        doc.refresh();
        same(&doc);
        assert_eq!(doc.objects().len(), other.objects().len());
        assert_eq!(
            doc.get(b).unwrap().data.bounds.y0,
            other.get(b).unwrap().data.bounds.y0
        );
    }

    #[test]
    fn ink_roundtrips_and_follows_its_bounds() {
        use kurbo::Point;
        let points = [
            (Point::new(10.0, 10.0), 1.0),
            (Point::new(30.0, 20.0), 0.5),
            (Point::new(50.0, 10.0), 0.25),
        ];
        let (ink, bounds) = Ink::from_world(&points).unwrap();
        assert_eq!(bounds, Rect::new(10.0, 10.0, 50.0, 20.0));
        let mut data = rect(0.0, 0.0, 0.0, 0.0);
        data.kind = ObjectKind::Ink(ink.clone());
        data.bounds = bounds;
        let mut doc = Document::new();
        let id = create(&mut doc, data);
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        let stored = &loaded.get(id).unwrap().data;
        assert_eq!(stored.kind, ObjectKind::Ink(ink.clone()));
        // Twice as wide: the middle point moves with the bounds.
        let wide = Rect::new(10.0, 10.0, 90.0, 20.0);
        assert_eq!(ink.points_in(wide)[1], (Point::new(50.0, 20.0), 0.5));
        assert!(ink.has_pressure());
    }

    #[test]
    fn text_roundtrips() {
        let mut doc = Document::new();
        let mut data = rect(0.0, 0.0, 100.0, 30.0);
        data.kind = ObjectKind::Text(Text {
            spans: Default::default(),
            text: "2 + 7 = 9, привет".into(),
            size: 18.0,
        });
        let id = create(&mut doc, data.clone());
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.get(id).unwrap().data, data);
    }

    #[test]
    fn set_text_replaces_text_and_bounds() {
        let mut doc = Document::new();
        let mut data = rect(0.0, 0.0, 10.0, 10.0);
        data.kind = ObjectKind::Text(Text {
            spans: Default::default(),
            text: "old".into(),
            size: 12.0,
        });
        let id = create(&mut doc, data);
        let mut stroke = rect(0.0, 0.0, 1.0, 1.0);
        let points = [(Point::new(0.0, 0.0), 1.0), (Point::new(1.0, 1.0), 1.0)];
        stroke.kind = ObjectKind::Ink(Ink::from_world(&points).unwrap().0);
        let stroke = create(&mut doc, stroke);
        let bounds = Rect::new(0.0, 0.0, 40.0, 15.0);
        doc.apply(Command::SetText {
            spans: Default::default(),
            id,
            text: "new text".into(),
            bounds,
        })
        .unwrap();
        let stored = &doc.get(id).unwrap().data;
        assert!(
            matches!(&stored.kind, ObjectKind::Text(t) if &*t.text == "new text" && t.size == 12.0)
        );
        assert_eq!(stored.bounds, bounds);
        assert!(
            doc.apply(Command::SetText {
                spans: Default::default(),
                id: stroke,
                text: "x".into(),
                bounds,
            })
            .is_err()
        );
        doc.undo().unwrap();
        assert!(
            matches!(&doc.get(id).unwrap().data.kind, ObjectKind::Text(t) if &*t.text == "old")
        );
    }

    #[test]
    fn sticky_roundtrips_and_takes_text() {
        let mut doc = Document::new();
        let mut data = rect(0.0, 0.0, 200.0, 200.0);
        data.kind = ObjectKind::Sticky(Sticky {
            spans: Default::default(),
            text: "".into(),
        });
        let id = create(&mut doc, data.clone());
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.get(id).unwrap().data, data);
        doc.apply(Command::SetText {
            spans: Default::default(),
            id,
            text: "идея".into(),
            bounds: data.bounds,
        })
        .unwrap();
        let stored = &doc.get(id).unwrap().data;
        assert_eq!(stored.kind.text(), Some("идея"));
        assert_eq!(stored.bounds, data.bounds);
    }

    #[test]
    fn shape_text_roundtrips_and_is_set() {
        let mut doc = Document::new();
        let data = rect(0.0, 0.0, 100.0, 60.0);
        let id = create(&mut doc, data.clone());
        assert_eq!(doc.get(id).unwrap().data.kind.text(), Some(""));
        doc.apply(Command::SetText {
            spans: Default::default(),
            id,
            text: "Start".into(),
            bounds: data.bounds,
        })
        .unwrap();
        let expected = ObjectKind::Shape(Shape {
            spans: Default::default(),
            kind: ShapeKind::Rectangle,
            text: "Start".into(),
        });
        assert_eq!(doc.get(id).unwrap().data.kind, expected);
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.get(id).unwrap().data.kind, expected);
        doc.undo().unwrap();
        assert_eq!(doc.get(id).unwrap().data.kind.text(), Some(""));
    }

    #[test]
    fn text_style_applies_to_shapes_only() {
        let mut doc = Document::new();
        let shape = create(&mut doc, rect(0.0, 0.0, 100.0, 60.0));
        let mut note = rect(200.0, 0.0, 100.0, 100.0);
        note.kind = ObjectKind::Sticky(Sticky {
            spans: Default::default(),
            text: "".into(),
        });
        let note = create(&mut doc, note);
        let red = Rgba::rgb(0xF2, 0x55, 0x55);
        doc.apply(Command::SetStyle {
            ids: vec![shape, note],
            change: StyleChange {
                text_color: Some(red),
                font_size: Some(Some(18.0)),
                text_align: Some(HAlign::Left),
                text_valign: Some(VAlign::Bottom),
                ..StyleChange::default()
            },
        })
        .unwrap();
        let style = |doc: &Document, id| doc.get(id).unwrap().data.style;
        assert_eq!(style(&doc, shape).text_color, Some(red));
        assert_eq!(style(&doc, shape).font_size, Some(18.0));
        assert_eq!(style(&doc, shape).text_align, HAlign::Left);
        assert_eq!(style(&doc, shape).text_valign, VAlign::Bottom);
        assert_eq!(style(&doc, note), Style::default());
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(style(&loaded, shape), style(&doc, shape));

        // Back to fitting; the color stays.
        doc.apply(Command::SetStyle {
            ids: vec![shape],
            change: StyleChange {
                font_size: Some(None),
                ..StyleChange::default()
            },
        })
        .unwrap();
        assert_eq!(style(&doc, shape).font_size, None);
        assert_eq!(style(&doc, shape).text_color, Some(red));
        doc.undo().unwrap();
        assert_eq!(style(&doc, shape).font_size, Some(18.0));
        doc.undo().unwrap();
        assert_eq!(style(&doc, shape), Style::default());
        // Copies keep the text style.
        doc.redo().unwrap();
        let copy = doc.content(shape).unwrap();
        let id = doc.apply(Command::Create(vec![copy])).unwrap().created[0];
        assert_eq!(style(&doc, id), style(&doc, shape));
    }

    fn frame(doc: &mut Document, bounds: Rect) -> ObjectId {
        let mut data = rect(bounds.x0, bounds.y0, bounds.width(), bounds.height());
        data.kind = ObjectKind::Frame(FrameData {
            title: "Plan".into(),
        });
        create(doc, data)
    }

    #[test]
    fn frames_roundtrip_and_hold_what_is_inside() {
        let mut doc = Document::new();
        let outer = frame(&mut doc, Rect::new(0.0, 0.0, 400.0, 300.0));
        let inner = frame(&mut doc, Rect::new(10.0, 10.0, 200.0, 200.0));
        let inside = create(&mut doc, rect(20.0, 20.0, 10.0, 10.0));
        // Its middle is outside, though it overlaps.
        let across = create(&mut doc, rect(350.0, 10.0, 200.0, 50.0));
        let pair = [
            create(&mut doc, rect(300.0, 200.0, 10.0, 10.0)),
            create(&mut doc, rect(320.0, 220.0, 10.0, 10.0)),
        ];
        let group = doc.apply(Command::Group(pair.to_vec())).unwrap().created[0];
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.get(outer).unwrap().data.kind.text(), Some("Plan"));

        assert_eq!(doc.frame_members(outer), vec![inner, inside, group]);
        assert_eq!(doc.frame_members(inner), vec![inside]);
        assert!(doc.frame_members(inside).is_empty());
        assert!(!doc.frame_members(outer).contains(&across));
        assert_eq!(
            doc.with_frame_members(&[inner, inside]),
            vec![inner, inside]
        );
        // Moving a frame by itself leaves the contents; the tools add them.
        doc.apply(Command::Move {
            ids: doc.with_frame_members(&[outer]),
            delta: Vec2::new(10.0, 0.0),
        })
        .unwrap();
        assert_eq!(doc.get(inside).unwrap().data.bounds.x0, 30.0);
        assert_eq!(doc.get(across).unwrap().data.bounds.x0, 350.0);
        // Connectors do not attach to frames.
        assert!(!connector::can_attach(&doc.get(outer).unwrap().data.kind));
    }

    #[test]
    fn inline_styles_roundtrip_and_follow_set_text() {
        let mut doc = Document::new();
        let mut data = rect(0.0, 0.0, 200.0, 200.0);
        let bold = crate::TextMarks {
            bold: true,
            ..Default::default()
        };
        let span = |start, end| crate::Span {
            start,
            end,
            marks: bold,
        };
        data.kind = ObjectKind::Sticky(Sticky {
            text: "один два".into(),
            spans: vec![span(0, 8)].into(),
        });
        let id = create(&mut doc, data.clone());
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.get(id).unwrap().data, data);
        // Spans past the new text are cut to it.
        doc.apply(Command::SetText {
            id,
            text: "од".into(),
            spans: vec![span(0, 8)].into(),
            bounds: data.bounds,
        })
        .unwrap();
        assert_eq!(doc.get(id).unwrap().data.kind.spans(), &[span(0, 4)]);
        doc.apply(Command::SetText {
            id,
            text: "plain".into(),
            spans: Spans::default(),
            bounds: data.bounds,
        })
        .unwrap();
        assert!(doc.get(id).unwrap().data.kind.spans().is_empty());
        doc.undo().unwrap();
        assert_eq!(doc.get(id).unwrap().data.kind.spans(), &[span(0, 4)]);
    }

    #[test]
    fn spatial_index_finds_objects_in_paint_order() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        let b = create(&mut doc, rect(5.0, 5.0, 10.0, 10.0));
        let far = create(&mut doc, rect(1000.0, 1000.0, 10.0, 10.0));
        let line = create(
            &mut doc,
            connector(
                End::Free(Point::new(500.0, -50.0)),
                End::Free(Point::new(500.0, 50.0)),
            ),
        );
        let ids = |doc: &Document, r: Rect| -> Vec<ObjectId> {
            doc.objects_in(r).into_iter().map(|o| o.id).collect()
        };
        assert_eq!(ids(&doc, Rect::new(6.0, 6.0, 7.0, 7.0)), vec![a, b]);
        assert_eq!(ids(&doc, Rect::new(495.0, 0.0, 505.0, 1.0)), vec![line]);
        assert!(ids(&doc, Rect::new(200.0, 200.0, 300.0, 300.0)).is_empty());
        // It follows edits and undo.
        doc.apply(Command::Move {
            ids: vec![far],
            delta: Vec2::new(-1000.0, -1000.0),
        })
        .unwrap();
        assert_eq!(ids(&doc, Rect::new(6.0, 6.0, 7.0, 7.0)), vec![a, b, far]);
        doc.undo().unwrap();
        assert_eq!(ids(&doc, Rect::new(6.0, 6.0, 7.0, 7.0)), vec![a, b]);
        // Bringing one forward changes the order.
        doc.apply(Command::Reorder {
            ids: vec![a],
            to: ZOrder::Front,
        })
        .unwrap();
        assert_eq!(ids(&doc, Rect::new(6.0, 6.0, 7.0, 7.0)), vec![b, a]);
    }

    #[test]
    #[ignore = "timing, run by hand"]
    fn big_board_timing() {
        let mut doc = Document::new();
        let items: Vec<Content> = (0..20_000)
            .map(|i| rect((i % 200) as f64 * 30.0, (i / 200) as f64 * 30.0, 20.0, 20.0).into())
            .collect();
        let t = std::time::Instant::now();
        doc.apply(Command::Create(items)).unwrap();
        eprintln!("create 20k: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        for i in 0..1000 {
            let x = f64::from(i % 100) * 50.0;
            assert!(!doc.objects_in(Rect::new(x, x, x + 1.0, x + 1.0)).is_empty() || x > 0.0);
        }
        eprintln!("1000 point queries: {:?}", t.elapsed());
        let id = doc.objects()[0].id;
        let t = std::time::Instant::now();
        doc.apply(Command::Move {
            ids: vec![id],
            delta: Vec2::new(1.0, 1.0),
        })
        .unwrap();
        eprintln!("one move: {:?}", t.elapsed());
    }

    #[test]
    fn tables_store_cells_by_line_and_merge() {
        let mut doc = Document::new();
        let table = crate::Table::new(2, 2, || doc.new_line_id());
        let mut data = rect(0.0, 0.0, 200.0, 80.0);
        data.kind = ObjectKind::Table(table.clone());
        let id = create(&mut doc, data);
        let cell = |text: &str| crate::Cell {
            text: text.into(),
            spans: Spans::default(),
        };
        let set = |doc: &mut Document, r: usize, c: usize, text: &str| {
            doc.apply(Command::SetCell {
                id,
                row: table.rows[r].id.clone(),
                col: table.cols[c].id.clone(),
                cell: cell(text),
            })
            .unwrap();
        };
        let get = |doc: &Document, r: usize, c: usize| match &doc.get(id).unwrap().data.kind {
            ObjectKind::Table(t) => t.text(r, c).0.to_owned(),
            _ => panic!("not a table"),
        };
        set(&mut doc, 1, 1, "Done");
        assert_eq!(get(&doc, 1, 1), "Done");
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.get(id).unwrap().data, doc.get(id).unwrap().data);
        // A row added on top keeps the text with its row.
        let mut rows = table.rows.to_vec();
        rows.insert(0, crate::Line::new(doc.new_line_id()));
        doc.apply(Command::SetTable {
            id,
            rows: rows.into(),
            cols: table.cols.clone(),
            merges: table.with_merged(crate::CellRange::new((0, 0), (1, 1))),
            bounds: Rect::new(0.0, 0.0, 200.0, 120.0),
        })
        .unwrap();
        assert_eq!(get(&doc, 2, 1), "Done");
        // Merged cells are named by line ids, so they stay on the old rows and survive a
        // reload.
        let merged = |doc: &Document| match &doc.get(id).unwrap().data.kind {
            ObjectKind::Table(t) => t.merged(),
            _ => panic!("not a table"),
        };
        assert_eq!(merged(&doc), vec![crate::CellRange::new((1, 0), (2, 1))]);
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(merged(&loaded), merged(&doc));
        doc.undo().unwrap();
        assert_eq!(get(&doc, 1, 1), "Done");
        assert!(merged(&doc).is_empty());
        // Two replicas typing in different cells both keep their text.
        let mut other = Document::from_snapshot(&doc.snapshot()).unwrap();
        set(&mut doc, 0, 0, "A");
        set(&mut other, 0, 1, "B");
        doc.doc.import(&other.snapshot()).unwrap();
        doc.refresh();
        assert_eq!((get(&doc, 0, 0), get(&doc, 0, 1)), ("A".into(), "B".into()));
        // Clearing a cell removes its text.
        set(&mut doc, 0, 0, "");
        assert_eq!(get(&doc, 0, 0), "");
    }

    #[test]
    fn image_roundtrips_and_lists_its_asset() {
        let mut doc = Document::new();
        let asset = doc.add_asset(b"fake png".to_vec());
        let mut data = rect(0.0, 0.0, 40.0, 30.0);
        data.kind = ObjectKind::Image(Image {
            asset,
            crop: Rect::new(0.25, 0.0, 1.0, 0.5),
        });
        let id = create(&mut doc, data.clone());
        create(&mut doc, data.clone());
        create(&mut doc, rect(0.0, 0.0, 1.0, 1.0));
        assert_eq!(doc.used_assets(), vec![asset]);
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.get(id).unwrap().data, data);
        let crop = Rect::new(0.5, 0.5, 1.0, 1.0);
        let bounds = Rect::new(5.0, 5.0, 25.0, 20.0);
        doc.apply(Command::SetCrop { id, crop, bounds }).unwrap();
        let stored = &doc.get(id).unwrap().data;
        assert_eq!(stored.kind, ObjectKind::Image(Image { asset, crop }));
        assert_eq!(stored.bounds, bounds);
        let square = doc.roots()[2];
        assert!(
            doc.apply(Command::SetCrop {
                id: square,
                crop,
                bounds
            })
            .is_err()
        );
        // Deleting an image keeps its file for undo.
        doc.apply(Command::Delete(doc.roots().to_vec())).unwrap();
        assert!(doc.used_assets().is_empty());
        assert!(doc.assets().contains(asset));
    }

    fn connector(start: End, end: End) -> ObjectData {
        ObjectData {
            kind: ObjectKind::Connector(Connector {
                bends: Default::default(),
                start,
                end,
                route: Route::Straight,
                start_cap: Cap::None,
                end_cap: Cap::Arrow,
                label: Label::default(),
            }),
            bounds: Rect::ZERO,
            rotation: 0.0,
            style: Style::default(),
        }
    }

    fn auto(object: ObjectId) -> End {
        End::Attached {
            object,
            anchor: Anchor::Auto,
            at: Point::ZERO,
        }
    }

    fn ends(doc: &Document, id: ObjectId) -> (Point, Point) {
        let (s, e) = doc.connector_ends(id).unwrap();
        (s.point, e.point)
    }

    fn close(a: Point, b: Point) -> bool {
        (a - b).hypot() < 1e-6
    }

    #[test]
    fn connector_follows_its_objects_and_roundtrips() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 100.0, 100.0));
        let b = create(&mut doc, rect(300.0, 0.0, 100.0, 100.0));
        let c = create(&mut doc, connector(auto(a), auto(b)));
        let (s, e) = ends(&doc, c);
        assert!(close(s, Point::new(100.0, 50.0)) && close(e, Point::new(300.0, 50.0)));
        // Bounds come from the path, arrowhead included.
        assert!(doc.get(c).unwrap().data.bounds.x1 >= 300.0 - 1e-9);
        doc.apply(Command::Move {
            ids: vec![b],
            delta: Vec2::new(0.0, 200.0),
        })
        .unwrap();
        let (_, e) = ends(&doc, c);
        assert!(e.y > 200.0, "{e:?}");
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.get(c).unwrap().data, doc.get(c).unwrap().data);
    }

    #[test]
    fn deleting_an_object_leaves_the_connector_where_it_was() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 100.0, 100.0));
        let b = create(&mut doc, rect(300.0, 0.0, 100.0, 100.0));
        let c = create(&mut doc, connector(auto(a), auto(b)));
        doc.apply(Command::Move {
            ids: vec![b],
            delta: Vec2::new(0.0, 500.0),
        })
        .unwrap();
        let before = ends(&doc, c);
        doc.apply(Command::Delete(vec![b])).unwrap();
        assert!(close(ends(&doc, c).1, before.1));
        // Undo brings the object back, still attached.
        doc.undo().unwrap();
        assert!(doc.contains(b));
        let ObjectKind::Connector(k) = &doc.get(c).unwrap().data.kind else {
            panic!("not a connector");
        };
        assert_eq!(k.end.object(), Some(b));
    }

    #[test]
    fn copies_attach_to_copies_and_lone_copies_come_loose() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 100.0, 100.0));
        let b = create(&mut doc, rect(300.0, 0.0, 100.0, 100.0));
        let c = create(
            &mut doc,
            connector(auto(a), End::Free(Point::new(200.0, 300.0))),
        );
        let made = doc
            .apply(Command::Create(vec![
                doc.content(a).unwrap(),
                doc.content(c).unwrap(),
            ]))
            .unwrap()
            .created;
        let ObjectKind::Connector(copy) = &doc.get(made[1]).unwrap().data.kind else {
            panic!("not a connector");
        };
        assert_eq!(copy.start.object(), Some(made[0]));
        assert_eq!(copy.end, End::Free(Point::new(200.0, 300.0)));
        // Copied alone, the attached end stays where it is, loose.
        let lone = doc
            .apply(Command::Create(vec![doc.content(c).unwrap()]))
            .unwrap()
            .created[0];
        let ObjectKind::Connector(lone_c) = &doc.get(lone).unwrap().data.kind else {
            panic!("not a connector");
        };
        assert!(matches!(lone_c.start, End::Free(p) if close(p, ends(&doc, c).0)));
        // A new connector (not a copy) keeps its attachments.
        let fresh = create(&mut doc, connector(auto(a), auto(b)));
        assert_eq!(
            doc.get(fresh).unwrap().data.kind,
            connector(auto(a), auto(b)).kind
        );
    }

    #[test]
    fn set_connector_changes_given_properties_and_ends() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 100.0, 100.0));
        let c = create(
            &mut doc,
            connector(
                End::Free(Point::new(500.0, 0.0)),
                End::Free(Point::new(600.0, 0.0)),
            ),
        );
        doc.apply(Command::SetConnector {
            ids: vec![c],
            change: crate::ConnectorChange {
                route: Some(Route::Elbow),
                end: Some(End::Attached {
                    object: a,
                    anchor: Anchor::Fixed(Point::new(0.5, 1.0)),
                    at: Point::ZERO,
                }),
                ..Default::default()
            },
        })
        .unwrap();
        let ObjectKind::Connector(k) = &doc.get(c).unwrap().data.kind else {
            panic!("not a connector");
        };
        assert_eq!(k.route, Route::Elbow);
        assert_eq!(k.end_cap, Cap::Arrow);
        assert!(close(ends(&doc, c).1, Point::new(50.0, 100.0)));
        // Detaching removes the stored object and anchor.
        doc.apply(Command::SetConnector {
            ids: vec![c],
            change: crate::ConnectorChange {
                end: Some(End::Free(Point::new(1.0, 2.0))),
                ..Default::default()
            },
        })
        .unwrap();
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        let ObjectKind::Connector(k) = &loaded.get(c).unwrap().data.kind else {
            panic!("not a connector");
        };
        assert_eq!(k.end, End::Free(Point::new(1.0, 2.0)));
    }

    #[test]
    fn labels_sit_on_the_line_and_roundtrip() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        let c = create(
            &mut doc,
            connector(
                End::Free(Point::new(0.0, 100.0)),
                End::Free(Point::new(400.0, 100.0)),
            ),
        );
        assert_eq!(doc.connector_label(c), None);
        let label = Label {
            text: "yes".into(),
            at: 0.25,
            size: kurbo::Size::new(40.0, 20.0),
        };
        doc.apply(Command::SetLabel {
            id: c,
            label: label.clone(),
        })
        .unwrap();
        let r = doc.connector_label(c).unwrap();
        // A quarter of the line, which stops short of the arrowhead at its end.
        assert!(
            (r.center().x - 100.0).abs() < 5.0 && r.center().y == 100.0,
            "{r:?}"
        );
        // The bounds take in the label and the gap around it.
        let bounds = doc.get(c).unwrap().data.bounds;
        assert!(bounds.y0 <= 90.0 - Label::PADDING + 1e-9, "{bounds:?}");
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        let ObjectKind::Connector(k) = &loaded.get(c).unwrap().data.kind else {
            panic!("not a connector");
        };
        assert_eq!(k.label, label);
        // Copies take the label along; only connectors take labels.
        let copy = doc.content(c).unwrap();
        let made = doc.apply(Command::Create(vec![copy])).unwrap().created[0];
        assert!(doc.connector_label(made).is_some());
        assert!(doc.apply(Command::SetLabel { id: a, label }).is_err());
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.connector_label(c), None);
    }

    #[test]
    fn resizing_a_selection_moves_free_connector_ends() {
        let mut doc = Document::new();
        let c = create(
            &mut doc,
            connector(
                End::Free(Point::new(0.0, 0.0)),
                End::Free(Point::new(100.0, 0.0)),
            ),
        );
        let bounds = doc.get(c).unwrap().data.bounds;
        let wider = Rect::new(
            bounds.x0,
            bounds.y0,
            bounds.x0 + bounds.width() * 2.0,
            bounds.y1,
        );
        doc.apply(Command::SetGeometry(vec![(c, Frame::new(wider, 0.0))]))
            .unwrap();
        let (s, e) = ends(&doc, c);
        assert!(close(s, Point::new(0.0, 0.0)), "{s:?}");
        assert!((e.x - 200.0).abs() < 1.0, "{e:?}");
    }

    #[test]
    fn sticky_text_area_is_inset() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        let area = Sticky::text_area(bounds);
        assert_eq!(area, Rect::new(8.0, 8.0, 192.0, 92.0));
        assert_eq!(Sticky::max_font_size(bounds), 16.0);
        let flat = Sticky::text_area(Rect::new(0.0, 0.0, 10.0, 0.0));
        assert_eq!(flat.area(), 0.0);
    }

    #[test]
    fn flat_ink_stays_on_its_line() {
        use kurbo::Point;
        let (ink, bounds) =
            Ink::from_world(&[(Point::new(0.0, 5.0), 1.0), (Point::new(10.0, 5.0), 1.0)]).unwrap();
        assert_eq!(bounds.height(), 0.0);
        let taller = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(ink.points_in(taller)[0].0, Point::new(0.0, 5.0));
        assert!(Ink::from_world(&[]).is_none());
    }

    #[test]
    fn undo_group_merges_commands() {
        let mut doc = Document::new();
        let a = create(&mut doc, rect(0.0, 0.0, 10.0, 10.0));
        doc.begin_undo_group();
        for opacity in [0.8, 0.5, 0.2] {
            doc.apply(Command::SetStyle {
                ids: vec![a],
                change: StyleChange {
                    opacity: Some(opacity),
                    ..StyleChange::default()
                },
            })
            .unwrap();
        }
        doc.end_undo_group();
        assert_eq!(doc.get(a).unwrap().data.style.opacity, 0.2);
        doc.undo().unwrap();
        assert_eq!(doc.get(a).unwrap().data.style.opacity, 1.0);
        assert!(doc.get(a).is_some());
    }

    #[test]
    fn empty_created_group_is_dropped() {
        let mut doc = Document::new();
        doc.apply(Command::Create(vec![Content::Group(Vec::new())]))
            .unwrap();
        assert!(doc.roots().is_empty());
        assert!(doc.tree.children(TreeParentId::Root).unwrap().is_empty());
    }
}
