//! Document model, commands and undo/redo. Knows nothing about rendering, UI or the platform.

mod command;
mod document;
mod object;

pub use command::{Applied, Command, ZOrder};
pub use document::{Document, Error, Result};
pub use object::{
    Content, Group, Ink, InkPoint, Object, ObjectData, ObjectId, ObjectKind, Rgba, Sticky, Style,
    StyleChange, Text,
};
pub use pb_geom::ShapeKind;
