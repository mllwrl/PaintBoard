//! A `.pboard` file is a short header followed by a Loro snapshot of the board, which keeps
//! the full edit history (for merging copies of a board later).
//!
//! The header version covers the container only; the layout of objects inside the snapshot
//! has its own schema version, checked by [`Document::from_snapshot`].
//!
//! TODO: move to a zip container once boards hold assets (images, PDFs).

use std::fmt;

use pb_core::Document;

pub const EXTENSION: &str = "pboard";

const MAGIC: &[u8; 8] = b"PBOARD\0\0";
/// Bump when the container layout changes, and keep reading the old one.
const FORMAT_VERSION: u32 = 1;
const HEADER_LEN: usize = MAGIC.len() + 4;

#[derive(Debug)]
pub enum LoadError {
    /// Not a PaintBoard file, or a truncated one.
    NotABoard,
    /// Written by a newer version of the app.
    NewerFormat(u32),
    Document(pb_core::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABoard => write!(f, "this is not a PaintBoard file, or it is damaged"),
            Self::NewerFormat(v) => write!(
                f,
                "the file was saved by a newer version of PaintBoard (format v{v})"
            ),
            Self::Document(pb_core::Error::UnsupportedSchema(_)) => {
                write!(f, "the board was saved by a newer version of PaintBoard")
            }
            Self::Document(err) => write!(f, "the board could not be read: {err}"),
        }
    }
}

impl std::error::Error for LoadError {}

pub fn encode(doc: &Document) -> Vec<u8> {
    let snapshot = doc.snapshot();
    let mut bytes = Vec::with_capacity(HEADER_LEN + snapshot.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&snapshot);
    bytes
}

pub fn decode(bytes: &[u8]) -> Result<Document, LoadError> {
    if bytes.len() < HEADER_LEN || &bytes[..MAGIC.len()] != MAGIC {
        return Err(LoadError::NotABoard);
    }
    let version = u32::from_le_bytes(
        bytes[MAGIC.len()..HEADER_LEN]
            .try_into()
            .expect("header slice has 4 bytes"),
    );
    if version > FORMAT_VERSION {
        return Err(LoadError::NewerFormat(version));
    }
    Document::from_snapshot(&bytes[HEADER_LEN..]).map_err(|err| match err {
        pb_core::Error::Crdt(_) => LoadError::NotABoard,
        err => LoadError::Document(err),
    })
}

#[cfg(test)]
mod tests {
    use pb_core::{Command, ObjectData, ShapeKind, Style};
    use pb_geom::kurbo::Rect;

    use super::*;

    fn board() -> Document {
        let mut doc = Document::new();
        let data = ObjectData {
            kind: ShapeKind::Star.into(),
            bounds: Rect::new(1.0, 2.0, 30.0, 40.0),
            rotation: 0.5,
            style: Style::default(),
        };
        let ids = doc
            .apply(Command::Create(vec![data.into(), data.into()]))
            .unwrap()
            .created;
        doc.apply(Command::Group(ids)).unwrap();
        doc
    }

    #[test]
    fn roundtrip_keeps_objects_and_groups() {
        let doc = board();
        let loaded = decode(&encode(&doc)).unwrap();
        assert_eq!(loaded.objects(), doc.objects());
        assert_eq!(loaded.roots(), doc.roots());
        assert!(loaded.group(loaded.roots()[0]).is_some());
    }

    #[test]
    fn rejects_foreign_and_damaged_files() {
        assert!(matches!(decode(b"PNG..."), Err(LoadError::NotABoard)));
        let mut bytes = encode(&board());
        bytes.truncate(bytes.len() / 2);
        assert!(matches!(decode(&bytes), Err(LoadError::NotABoard)));
    }

    #[test]
    fn rejects_newer_format() {
        let mut bytes = encode(&board());
        bytes[MAGIC.len()..HEADER_LEN].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        assert!(matches!(decode(&bytes), Err(LoadError::NewerFormat(_))));
    }
}
