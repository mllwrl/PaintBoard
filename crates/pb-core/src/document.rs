use std::collections::{HashMap, HashSet};
use std::fmt;

use kurbo::{Rect, Vec2};
use loro::{
    ExportMode, LoroDoc, LoroError, LoroMap, LoroTree, LoroValue, TreeID, TreeParentId,
    UndoManager, ValueOrContainer,
};
#[cfg(test)]
use pb_geom::ShapeKind;
use pb_geom::normalize_angle;

use crate::command::reorder;
use crate::{
    Applied, Command, Content, Group, Object, ObjectData, ObjectId, ObjectKind, Rgba, Style,
    StyleChange, ZOrder,
};

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
    pub const LOCKED: &str = "locked";

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
        };
        document.refresh();
        document
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.doc
            .export(ExportMode::Snapshot)
            .expect("snapshot export of a committed document cannot fail")
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
            Some(object) => Some(Content::Object(object.data)),
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
                for content in &contents {
                    applied
                        .created
                        .push(self.create(TreeParentId::Root, content)?);
                }
                self.prune_empty_groups(TreeParentId::Root)?;
            }
            Command::Delete(ids) => {
                for id in self.outermost(&ids)? {
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
                            write_style_change(&meta, &change, object.data.style)?;
                        }
                    }
                }
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

    fn create(&self, parent: TreeParentId, content: &Content) -> Result<ObjectId> {
        let node = self.tree.create(parent)?;
        let id = ObjectId::from_node(node);
        let meta = self.tree.get_meta(node)?;
        meta.insert(key::ID, id.to_string())?;
        match content {
            Content::Object(data) => write_data(&meta, data)?,
            Content::Group(children) => {
                meta.insert(key::KIND, key::GROUP)?;
                for child in children {
                    self.create(TreeParentId::Node(node), child)?;
                }
            }
        }
        Ok(id)
    }

    fn move_each(&self, moves: &[(ObjectId, Vec2)]) -> Result<()> {
        let ids: Vec<_> = moves.iter().map(|(id, _)| *id).collect();
        self.check_exist(&ids)?;
        let mut moved = HashSet::new();
        for &(id, delta) in moves {
            for object in self.leaves(id) {
                if moved.insert(object.id) {
                    let meta = self.tree.get_meta(self.nodes[&object.id])?;
                    write_bounds(&meta, object.data.bounds + delta)?;
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
        self.roots = self.read_children(TreeParentId::Root, None);
        self.revision += 1;
    }

    /// Caches the subtree under `parent` and returns the ids of its direct children.
    fn read_children(&mut self, parent: TreeParentId, group: Option<ObjectId>) -> Vec<ObjectId> {
        let mut ids = Vec::new();
        for node in self.tree.children(parent).unwrap_or_default() {
            let Ok(meta) = self.tree.get_meta(node) else {
                continue;
            };
            let id = match meta.get(key::ID) {
                Some(ValueOrContainer::Value(LoroValue::String(s))) => ObjectId::parse(&s),
                _ => None,
            }
            .unwrap_or_else(|| ObjectId::from_node(node));
            // Two replicas undoing the same deletion can each restore the object; keep one.
            if self.nodes.contains_key(&id) {
                continue;
            }
            let locked = matches!(
                meta.get(key::LOCKED),
                Some(ValueOrContainer::Value(LoroValue::Bool(true)))
            );
            if is_group(&meta) {
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
                let Some(data) = read_data(&meta) else {
                    continue;
                };
                self.nodes.insert(id, node);
                self.index.insert(id, self.objects.len());
                self.objects.push(Object {
                    id,
                    data,
                    parent: group,
                    locked,
                });
            }
            ids.push(id);
        }
        ids
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

fn write_data(meta: &LoroMap, data: &ObjectData) -> Result<()> {
    meta.insert(key::KIND, data.kind.as_str())?;
    write_bounds(meta, data.bounds.abs())?;
    meta.insert(key::ROTATION, normalize_angle(data.rotation))?;
    let style = data.style;
    let all = StyleChange {
        fill: Some(style.fill),
        stroke: Some(style.stroke),
        stroke_width: Some(style.stroke_width),
        opacity: Some(style.opacity),
    };
    write_style_change(meta, &all, style)
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
        ValueOrContainer::Value(LoroValue::String(s)) => ObjectKind::parse(&s)?,
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
            assert_eq!(doc.get(id).unwrap().data.kind, ObjectKind::Shape(shape));
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

    #[test]
    fn empty_created_group_is_dropped() {
        let mut doc = Document::new();
        doc.apply(Command::Create(vec![Content::Group(Vec::new())]))
            .unwrap();
        assert!(doc.roots().is_empty());
        assert!(doc.tree.children(TreeParentId::Root).unwrap().is_empty());
    }
}
