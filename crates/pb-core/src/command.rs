use kurbo::{Rect, Vec2};

use crate::{ObjectData, ObjectId};

/// A user-level edit. Each applied command is one undo step and, later, one unit of sync.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Adds an object on top of all others.
    Create(ObjectData),
    Delete(Vec<ObjectId>),
    Move {
        ids: Vec<ObjectId>,
        delta: Vec2,
    },
    SetBounds {
        id: ObjectId,
        bounds: Rect,
    },
}

/// What a command produced, for callers that need to follow up (e.g. select new objects).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    pub created: Vec<ObjectId>,
}
