use pb_core::{Document, ObjectId};
use pb_geom::kurbo::Rect;

/// Selected objects in the order they were selected. View state, never stored in the document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    ids: Vec<ObjectId>,
}

impl Selection {
    pub fn ids(&self) -> &[ObjectId] {
        &self.ids
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: ObjectId) -> bool {
        self.ids.contains(&id)
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }

    pub fn set(&mut self, ids: impl IntoIterator<Item = ObjectId>) {
        self.ids.clear();
        for id in ids {
            self.add(id);
        }
    }

    pub fn add(&mut self, id: ObjectId) {
        if !self.contains(id) {
            self.ids.push(id);
        }
    }

    pub fn remove(&mut self, id: ObjectId) {
        self.ids.retain(|&x| x != id);
    }

    /// Drops ids of objects that no longer exist (deleted, undone, removed remotely).
    pub fn retain_existing(&mut self, doc: &Document) {
        self.ids.retain(|&id| doc.get(id).is_some());
    }

    /// Union of the selected objects' bounds.
    pub fn bounds(&self, doc: &Document) -> Option<Rect> {
        self.ids
            .iter()
            .filter_map(|&id| doc.get(id))
            .map(|o| o.data.bounds)
            .reduce(|a, b| a.union(b))
    }
}
