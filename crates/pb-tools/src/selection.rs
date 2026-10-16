use pb_core::{Document, Object, ObjectId};
use pb_geom::Frame;

/// Selected top-level objects and groups in the order they were selected. View state, never
/// stored in the document.
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

    /// Drops ids that no longer exist (deleted, undone, removed remotely) and replaces
    /// objects that got grouped by their group.
    pub fn retain_existing(&mut self, doc: &Document) {
        let ids: Vec<_> = self
            .ids
            .iter()
            .filter(|&&id| doc.contains(id))
            .map(|&id| doc.top_level(id))
            .collect();
        self.set(ids);
    }

    /// Whether any selected object is locked.
    pub fn has_locked(&self, doc: &Document) -> bool {
        self.ids.iter().any(|&id| doc.is_locked(id))
    }

    /// The frame that resize and rotate handles attach to.
    pub fn frame(&self, doc: &Document) -> Option<Frame> {
        selection_frame(doc, &self.ids, |o| o.data.frame())
    }
}

/// Frame of a selectable unit: an object's own (possibly rotated) frame, or the
/// axis-aligned box around a group. `frame` gives each object's current frame, so callers
/// can follow a gesture in progress.
pub fn unit_frame(doc: &Document, id: ObjectId, frame: impl Fn(&Object) -> Frame) -> Option<Frame> {
    match doc.get(id) {
        Some(object) => Some(frame(object)),
        None => bounding_frame(doc.leaves(id).into_iter().map(frame)),
    }
}

/// Frame of a selection: a single unit's own frame, or the axis-aligned box around several.
pub fn selection_frame(
    doc: &Document,
    ids: &[ObjectId],
    frame: impl Fn(&Object) -> Frame,
) -> Option<Frame> {
    match ids {
        [] => None,
        [id] => unit_frame(doc, *id, frame),
        _ => bounding_frame(ids.iter().flat_map(|&id| doc.leaves(id)).map(frame)),
    }
}

fn bounding_frame(frames: impl Iterator<Item = Frame>) -> Option<Frame> {
    frames
        .map(|f| f.aabb())
        .reduce(|a, b| a.union(b))
        .map(Frame::axis_aligned)
}
