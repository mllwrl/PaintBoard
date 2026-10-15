use std::collections::HashMap;
use std::fmt;

use kurbo::Rect;
use loro::{
    ExportMode, LoroDoc, LoroError, LoroMap, LoroTree, LoroValue, TreeID, TreeParentId,
    UndoManager, ValueOrContainer,
};
use pb_geom::normalize_angle;

use crate::command::reorder;
use crate::{Applied, Command, Object, ObjectData, ObjectId, ObjectKind, Rgba, Style};

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
/// Objects are nodes of a Loro movable tree. Sibling order is the z-order (back to front)
/// and nesting will carry frames and groups. All edits go through [`Document::apply`].
pub struct Document {
    doc: LoroDoc,
    tree: LoroTree,
    undo: UndoManager,
    /// Cached objects in z-order, rebuilt after every change.
    objects: Vec<Object>,
    index: HashMap<ObjectId, usize>,
    /// Tree node currently holding each object.
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

    /// Objects in z-order, back to front.
    pub fn objects(&self) -> &[Object] {
        &self.objects
    }

    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.index.get(&id).map(|&i| &self.objects[i])
    }

    /// Increments on every change; lets views skip work when nothing changed.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn apply(&mut self, command: Command) -> Result<Applied> {
        let mut applied = Applied::default();
        match command {
            Command::Create(objects) => {
                for data in objects {
                    let node = self.tree.create(TreeParentId::Root)?;
                    let id = ObjectId::from_node(node);
                    let meta = self.tree.get_meta(node)?;
                    meta.insert(key::ID, id.to_string())?;
                    write_data(&meta, &data)?;
                    applied.created.push(id);
                }
            }
            Command::Delete(ids) => {
                self.check_exist(&ids)?;
                for id in ids {
                    self.tree.delete(self.nodes[&id])?;
                }
            }
            Command::Move { ids, delta } => {
                self.check_exist(&ids)?;
                for id in ids {
                    let bounds = self.objects[self.index[&id]].data.bounds + delta;
                    write_bounds(&self.tree.get_meta(self.nodes[&id])?, bounds)?;
                }
            }
            Command::SetGeometry(changes) => {
                let ids: Vec<_> = changes.iter().map(|(id, _)| *id).collect();
                self.check_exist(&ids)?;
                for (id, frame) in changes {
                    let meta = self.tree.get_meta(self.nodes[&id])?;
                    write_bounds(&meta, frame.rect.abs())?;
                    meta.insert(key::ROTATION, normalize_angle(frame.rotation))?;
                }
            }
            Command::Reorder { ids, to } => {
                self.check_exist(&ids)?;
                let current: Vec<_> = self.objects.iter().map(|o| o.id).collect();
                let target = reorder(&current, &ids, to);
                // Walk the target order front to back, placing each moved object right after
                // its new predecessor; everything before it is then already final.
                for (i, id) in target.iter().enumerate() {
                    if !ids.contains(id) {
                        continue;
                    }
                    let node = self.nodes[id];
                    match i {
                        0 => self.tree.mov_to(node, TreeParentId::Root, 0)?,
                        _ => self.tree.mov_after(node, self.nodes[&target[i - 1]])?,
                    }
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

    /// Validates up front so a failing command leaves the document untouched.
    fn check_exist(&self, ids: &[ObjectId]) -> Result<()> {
        match ids.iter().find(|id| !self.index.contains_key(id)) {
            Some(&id) => Err(Error::UnknownObject(id)),
            None => Ok(()),
        }
    }

    /// Rebuilds the cache from the CRDT.
    ///
    /// TODO: update incrementally from Loro diff events once boards get large.
    fn refresh(&mut self) {
        self.objects.clear();
        self.index.clear();
        self.nodes.clear();
        for node in self.tree.children(TreeParentId::Root).unwrap_or_default() {
            let Ok(meta) = self.tree.get_meta(node) else {
                continue;
            };
            // Objects of kinds this version does not know are skipped, not dropped.
            let Some(data) = read_data(&meta) else {
                continue;
            };
            let id = match meta.get(key::ID) {
                Some(ValueOrContainer::Value(LoroValue::String(s))) => ObjectId::parse(&s),
                _ => None,
            }
            .unwrap_or_else(|| ObjectId::from_node(node));
            // Two replicas undoing the same deletion can each restore the object; keep one.
            if self.nodes.insert(id, node).is_some() {
                continue;
            }
            self.index.insert(id, self.objects.len());
            self.objects.push(Object { id, data });
        }
        self.revision += 1;
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

fn write_data(meta: &LoroMap, data: &ObjectData) -> Result<()> {
    meta.insert(key::KIND, data.kind.as_str())?;
    write_bounds(meta, data.bounds.abs())?;
    meta.insert(key::ROTATION, normalize_angle(data.rotation))?;
    meta.insert(key::FILL, i64::from(data.style.fill.0))?;
    meta.insert(key::STROKE, i64::from(data.style.stroke.0))?;
    meta.insert(key::STROKE_WIDTH, data.style.stroke_width)?;
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
        },
    })
}

#[cfg(test)]
mod tests {
    use kurbo::Vec2;
    use pb_geom::Frame;

    use crate::ZOrder;

    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> ObjectData {
        ObjectData {
            kind: ObjectKind::Rectangle,
            bounds: Rect::new(x, y, x + w, y + h),
            rotation: 0.0,
            style: Style::default(),
        }
    }

    fn create(doc: &mut Document, data: ObjectData) -> ObjectId {
        doc.apply(Command::Create(vec![data])).unwrap().created[0]
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
                rect(0.0, 0.0, 1.0, 1.0),
                rect(5.0, 0.0, 1.0, 1.0),
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
        let loaded = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(loaded.objects(), doc.objects());
        assert_eq!(loaded.objects()[0].id, a);
        assert_eq!(loaded.objects()[1].id, b);
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
}
