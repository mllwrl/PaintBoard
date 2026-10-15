use pb_core::{Document, ObjectData, ObjectId};
use pb_geom::Frame;

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

    /// The frame that resize and rotate handles attach to.
    pub fn frame(&self, doc: &Document) -> Option<Frame> {
        frame_of(
            self.ids
                .iter()
                .filter_map(|&id| doc.get(id))
                .map(|o| &o.data),
        )
    }
}

/// Selection frame of some objects: a single object's own (possibly rotated) frame, or the
/// axis-aligned box around several.
pub fn frame_of<'a>(mut objects: impl Iterator<Item = &'a ObjectData>) -> Option<Frame> {
    let first = objects.next()?;
    match objects.next() {
        None => Some(first.frame()),
        Some(second) => {
            let aabb = [first, second]
                .into_iter()
                .chain(objects)
                .map(|d| d.frame().aabb())
                .reduce(|a, b| a.union(b))?;
            Some(Frame::axis_aligned(aabb))
        }
    }
}
