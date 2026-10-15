//! Document model, commands and undo/redo. Knows nothing about rendering, UI or the platform.

mod command;
mod document;
mod object;

pub use command::{Applied, Command, ZOrder};
pub use document::{Document, Error, Result};
pub use object::{Object, ObjectData, ObjectId, ObjectKind, Rgba, Style};
