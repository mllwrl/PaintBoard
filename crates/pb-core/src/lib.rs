//! Document model, commands and undo/redo. Knows nothing about rendering, UI or the platform.

mod asset;
mod command;
pub mod connector;
mod document;
mod object;

pub use asset::{AssetId, Assets};
pub use command::{Applied, Command, ZOrder};
pub use connector::{Anchor, Cap, Connector, ConnectorChange, End, Label, Route};
pub use document::{Document, Error, Result};
pub use object::{
    Content, FrameData, Group, HAlign, Image, Ink, InkPoint, Object, ObjectData, ObjectId,
    ObjectKind, Rgba, Shape, Sticky, Style, StyleChange, Text, TextBox, VAlign,
};
pub use pb_geom::ShapeKind;
