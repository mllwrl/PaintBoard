//! Document model, commands and undo/redo. Knows nothing about rendering, UI or the platform.

mod asset;
mod command;
pub mod connector;
mod document;
pub mod kanban;
pub mod mindmap;
mod object;
pub mod rich;
pub mod search;
pub mod table;

pub use asset::{AssetId, Assets};
pub use command::{Applied, Command, ZOrder};
pub use connector::{Anchor, Cap, Connector, ConnectorChange, End, Label, Route};
pub use document::{Document, Error, Result};
pub use kanban::{Card, Column, Kanban, KanbanPart};
pub use mindmap::{MindMap, MindMapPart, Node};
pub use object::{
    Content, Filter, FrameData, Group, HAlign, Image, Ink, InkPoint, Link, Object, ObjectData,
    ObjectId, ObjectKind, Rgba, Shape, Sticky, Style, StyleChange, Tag, Tags, Text, TextBox,
    VAlign,
};
pub use pb_geom::ShapeKind;
pub use rich::{Span, Spans, TextMarks};
pub use table::{Cell, CellRange, Line, Merge, Table};
