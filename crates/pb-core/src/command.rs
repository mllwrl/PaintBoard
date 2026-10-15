use kurbo::Vec2;
use pb_geom::Frame;

use crate::{ObjectData, ObjectId};

/// A user-level edit. Each applied command is one undo step and, later, one unit of sync.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Adds objects on top of all others, in the given order.
    Create(Vec<ObjectData>),
    Delete(Vec<ObjectId>),
    Move {
        ids: Vec<ObjectId>,
        delta: Vec2,
    },
    /// Sets position, size and rotation of several objects at once (resize, rotate).
    SetGeometry(Vec<(ObjectId, Frame)>),
    /// Changes the stacking order of objects.
    Reorder {
        ids: Vec<ObjectId>,
        to: ZOrder,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZOrder {
    Front,
    Back,
    /// One step up, past the next object that is not being moved.
    Forward,
    Backward,
}

/// What a command produced, for callers that need to follow up (e.g. select new objects).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    pub created: Vec<ObjectId>,
}

/// The stacking order after moving `selected` objects, back to front.
pub(crate) fn reorder(current: &[ObjectId], selected: &[ObjectId], to: ZOrder) -> Vec<ObjectId> {
    let is_selected = |id: &ObjectId| selected.contains(id);
    let mut order = current.to_vec();
    match to {
        // Stable sorts keep the relative order within both groups.
        ZOrder::Front => order.sort_by_key(|id| is_selected(id)),
        ZOrder::Back => order.sort_by_key(|id| !is_selected(id)),
        ZOrder::Forward => {
            for i in (0..order.len().saturating_sub(1)).rev() {
                if is_selected(&order[i]) && !is_selected(&order[i + 1]) {
                    order.swap(i, i + 1);
                }
            }
        }
        ZOrder::Backward => {
            for i in 1..order.len() {
                if is_selected(&order[i]) && !is_selected(&order[i - 1]) {
                    order.swap(i, i - 1);
                }
            }
        }
    }
    order
}
