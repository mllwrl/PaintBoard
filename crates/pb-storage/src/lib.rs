//! The `.pboard` file format, autosave, assets and schema migrations.

mod autosave;
mod format;

pub use autosave::Autosave;
pub use format::{EXTENSION, LoadError, decode, encode};
