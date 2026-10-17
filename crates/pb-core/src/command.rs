use std::sync::Arc;

use kurbo::{Rect, Vec2};
use pb_geom::Frame;

use crate::{
    Card, Cell, Column, Comment, ConnectorChange, Content, Embed, Label, Layer, Line, Link, Merge,
    Node, ObjectId, Reaction, Spans, StyleChange, Tag, ThreadAnchor,
};

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
        /// Inline styles of `text`; a frame's title has none.
        spans: Spans,
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
    /// Sets the points a connector's line runs through; empty for its automatic path.
    /// Takes a connector only.
    SetBends {
        id: ObjectId,
        bends: Arc<[kurbo::Point]>,
    },
    /// Replaces the text of a table's cell, named by the ids of its row and column. Takes a
    /// table only.
    SetCell {
        id: ObjectId,
        row: Arc<str>,
        col: Arc<str>,
        cell: Cell,
    },
    /// Sets a table's rows and columns (added, removed, reordered or resized), merged
    /// cells and bounds. Cells of removed lines are kept, unseen, so undo brings them back.
    SetTable {
        id: ObjectId,
        rows: Arc<[Line]>,
        cols: Arc<[Line]>,
        merges: Arc<[Merge]>,
        bounds: Rect,
    },
    /// Adds a card to a kanban board, or changes the card with its id (text, column or
    /// place), and sets the board's bounds. Takes a kanban board only.
    SetCard {
        id: ObjectId,
        card: Card,
        bounds: Rect,
    },
    /// Removes a card from a kanban board and sets the board's bounds.
    RemoveCard {
        id: ObjectId,
        card: Arc<str>,
        bounds: Rect,
    },
    /// Sets a kanban board's columns (added, removed, reordered or renamed) and bounds.
    /// Cards of removed columns are kept, unseen, so undo brings them back.
    SetColumns {
        id: ObjectId,
        columns: Arc<[Column]>,
        bounds: Rect,
    },
    /// Puts tags `add` on objects and takes tags `remove` off them; a group tags
    /// everything inside it.
    SetTags {
        ids: Vec<ObjectId>,
        add: Vec<Arc<str>>,
        remove: Vec<Arc<str>>,
    },
    /// Replaces a web page card's address, title, description and site. Takes a card only.
    SetEmbed {
        id: ObjectId,
        embed: Embed,
    },
    /// Sets or (`None`) removes the link of objects; groups have none.
    SetLink {
        ids: Vec<ObjectId>,
        link: Option<Link>,
    },
    /// Adds `comment` to `thread`; a new thread needs its `anchor`.
    AddComment {
        thread: Arc<str>,
        anchor: Option<ThreadAnchor>,
        comment: Comment,
    },
    /// Removes a comment; a thread left without any is no longer listed.
    RemoveComment {
        thread: Arc<str>,
        comment: Arc<str>,
    },
    /// Marks a thread resolved, or open again.
    SetResolved {
        thread: Arc<str>,
        resolved: bool,
    },
    /// Removes a thread with all its comments.
    RemoveThread(Arc<str>),
    /// Starts a voting session with `limit` votes per person, clearing earlier votes.
    StartVoting {
        limit: u32,
    },
    /// Ends the voting session; the votes stay, to show the results.
    EndVoting,
    /// Removes the votes and the session.
    ClearVotes,
    /// Gives object `id` one of `author`'s votes (`add`), or takes one back.
    Vote {
        id: ObjectId,
        author: Arc<str>,
        add: bool,
    },
    /// Puts `author`'s `reaction` on object `id` (`on`), or takes it off.
    React {
        id: ObjectId,
        reaction: Reaction,
        author: Arc<str>,
        on: bool,
    },
    /// Adds a tag to the board, or renames or recolors the tag with its id.
    DefineTag(Tag),
    /// Removes a tag from the board. Objects keep its id, unseen, so undo brings it back.
    RemoveTag(Arc<str>),
    /// Adds a node to a mind map, or changes the node with its id (text, size, parent,
    /// place, collapsing), and sets the map's bounds. Takes a mind map only.
    SetNode {
        id: ObjectId,
        node: Node,
        bounds: Rect,
    },
    /// Removes nodes from a mind map and sets its bounds.
    RemoveNodes {
        id: ObjectId,
        nodes: Vec<Arc<str>>,
        bounds: Rect,
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
    /// Hidden objects are not drawn and cannot be picked, unless hidden ones are shown.
    SetHidden {
        ids: Vec<ObjectId>,
        hidden: bool,
    },
    /// Protects the board from changes, or lifts that. Not undoable.
    SetReadOnly(bool),
    /// Adds a layer, or renames, reorders, hides or locks the layer with its id.
    DefineLayer(Layer),
    /// Removes a layer with everything on it. The base layer stays.
    RemoveLayer(Arc<str>),
    /// Moves top-level objects and groups to a layer.
    SetLayer {
        ids: Vec<ObjectId>,
        layer: Arc<str>,
    },
}

impl Command {
    /// Whether it may change a read-only board: it does not touch the content, like
    /// comments, reactions and voting.
    pub fn allowed_read_only(&self) -> bool {
        matches!(
            self,
            Self::SetReadOnly(_)
                | Self::AddComment { .. }
                | Self::RemoveComment { .. }
                | Self::SetResolved { .. }
                | Self::RemoveThread(_)
                | Self::React { .. }
                | Self::StartVoting { .. }
                | Self::EndVoting
                | Self::ClearVotes
                | Self::Vote { .. }
        )
    }
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
