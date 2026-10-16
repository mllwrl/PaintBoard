use std::sync::Arc;

use kurbo::{Rect, Vec2};
use pb_geom::Frame;

use crate::{ConnectorChange, Content, Label, ObjectId, StyleChange};

/// A user-level edit. Each applied command is one undo step and, later, one unit of sync.
///
/// Ids may name objects or groups unless noted; a group stands for everything inside it.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Adds content on top of all others, in the given order. Connectors copied along with
    /// the objects they attach to attach to the copies; copied ends attached to anything
    /// else come loose.
    Create(Vec<Content>),
    /// Groups left empty are deleted too.
    Delete(Vec<ObjectId>),
    Move {
        ids: Vec<ObjectId>,
        delta: Vec2,
    },
    /// Moves each object by its own offset (align, distribute).
    MoveEach(Vec<(ObjectId, Vec2)>),
    /// Sets position, size and rotation of several objects at once (resize, rotate).
    /// Takes objects only, not groups.
    SetGeometry(Vec<(ObjectId, Frame)>),
    /// Changes the stacking order of objects among their siblings.
    Reorder {
        ids: Vec<ObjectId>,
        to: ZOrder,
    },
    /// Wraps objects in a new group that takes the place of the topmost of them.
    Group(Vec<ObjectId>),
    /// Dissolves groups; their children take the group's place. Other ids are ignored.
    Ungroup(Vec<ObjectId>),
    /// Replaces the text of a text object, sticky note or shape and sets its bounds (text
    /// objects are fitted to their text). Takes no other objects.
    SetText {
        id: ObjectId,
        text: Arc<str>,
        bounds: Rect,
    },
    /// Shows another part of an image's picture (see `Image::crop`) over new bounds.
    /// Takes an image only.
    SetCrop {
        id: ObjectId,
        crop: Rect,
        bounds: Rect,
    },
    /// Changes connectors' ends, route or arrowheads; other ids are ignored.
    SetConnector {
        ids: Vec<ObjectId>,
        change: ConnectorChange,
    },
    /// Replaces a connector's label. Takes a connector only.
    SetLabel {
        id: ObjectId,
        label: Label,
    },
    /// Changes colors, outline or opacity; a group restyles everything inside it.
    SetStyle {
        ids: Vec<ObjectId>,
        change: StyleChange,
    },
    /// Locked objects cannot be moved, resized or deleted from the canvas.
    SetLocked {
        ids: Vec<ObjectId>,
        locked: bool,
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
    /// Top-level ids of created content or of a new group.
    pub created: Vec<ObjectId>,
    /// Former children of dissolved groups.
    pub ungrouped: Vec<ObjectId>,
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
