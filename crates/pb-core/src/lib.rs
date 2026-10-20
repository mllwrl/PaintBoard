//! Document model, commands and undo/redo. Knows nothing about rendering, UI or the platform.

mod asset;
mod command;
pub mod comment;
pub mod connector;
mod document;
pub mod kanban;
mod layer;
pub mod mindmap;
mod object;
pub mod reaction;
pub mod rich;
pub mod search;
pub mod table;
pub mod talktrack;
pub mod timer;
pub mod version;

#[cfg(test)]
mod fuzz_tests;

pub use asset::{AssetId, Assets};
pub use command::{Applied, Command, Role, ZOrder};
pub use comment::{Comment, Thread, ThreadAnchor};
pub use connector::{Anchor, Cap, Connector, ConnectorChange, End, Label, Route};
pub use document::{
    Document, Error, Result, SnapshotSource, Voting, board_title, commenting_writes,
};
pub use kanban::{Card, Column, Kanban, KanbanPart};
pub use layer::Layer;
pub use mindmap::{MindMap, MindMapPart, Node};
pub use object::{
    Book, Content, Doc, Embed, Filter, FrameData, Group, HAlign, Image, Ink, InkPoint, Link,
    Object, ObjectData, ObjectId, ObjectKind, Rgba, Shape, Sticky, Style, StyleChange, Tag, Tags,
    Text, TextBox, VAlign, shape_name,
};
pub use pb_geom::ShapeKind;
pub use reaction::Reaction;
pub use rich::{Span, Spans, TextMarks};
pub use table::{Cell, CellRange, Line, Merge, Table};
pub use talktrack::Talktrack;
pub use timer::Timer;
pub use version::Version;
