//! A `.pboard` file is a short header, a Loro snapshot of the board, which keeps the full
//! edit history (for merging copies of a board later), and the files its images use.
//!
//! Layout (integers little-endian):
//! - `PBOARD\0\0`, format version `u32`
//! - v1: the snapshot fills the rest of the file
//! - v2: snapshot length `u64`, the snapshot, asset count `u32`, then per asset its BLAKE3
//!   hash (32 bytes), length `u64` and bytes
//!
//! The header version covers the container only; the layout of objects inside the snapshot
//! has its own schema version, checked by [`Document::from_snapshot`].
//!
//! Only assets that objects use are written; files of images deleted earlier are dropped.
//!
//! TODO: an append-friendly or zip container, so autosave does not rewrite every image.

use std::fmt;

use pb_core::{AssetId, Document};

pub const EXTENSION: &str = "pboard";

const MAGIC: &[u8; 8] = b"PBOARD\0\0";
/// Bump when the container layout changes, and keep reading the old one.
const FORMAT_VERSION: u32 = 2;
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
    prepare(doc).encode()
}

/// What a board's file is made of, gathered quickly so the slow part, [`Prepared::encode`],
/// can run on another thread.
pub struct Prepared {
    snapshot: pb_core::SnapshotSource,
    assets: Vec<(pb_core::AssetId, std::sync::Arc<[u8]>)>,
}

/// Gathers what [`encode`] writes: the board and the files of the images on it.
pub fn prepare(doc: &Document) -> Prepared {
    let assets = doc
        .used_assets()
        .into_iter()
        .filter_map(|id| doc.assets().get(id).map(|bytes| (id, bytes.clone())))
        .collect();
    Prepared {
        snapshot: doc.snapshot_source(),
        assets,
    }
}

impl Prepared {
    /// The file's bytes, as [`encode`] makes them.
    pub fn encode(self) -> Vec<u8> {
        write(&self.snapshot.export(), &self.assets)
    }
}

fn write(snapshot: &[u8], assets: &[(pb_core::AssetId, std::sync::Arc<[u8]>)]) -> Vec<u8> {
    let asset_len: usize = assets.iter().map(|(_, b)| 40 + b.len()).sum();
    let mut bytes = Vec::with_capacity(HEADER_LEN + 12 + snapshot.len() + asset_len);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(snapshot.len() as u64).to_le_bytes());
    bytes.extend_from_slice(snapshot);
    bytes.extend_from_slice(&(assets.len() as u32).to_le_bytes());
    for (id, data) in assets {
        bytes.extend_from_slice(id.as_bytes());
        bytes.extend_from_slice(&(data.len() as u64).to_le_bytes());
        bytes.extend_from_slice(data);
    }
    bytes
}

/// Reads the container's parts in order.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        if n > self.0.len() {
            return Err(LoadError::NotABoard);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u32(&mut self) -> Result<u32, LoadError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn len(&mut self) -> Result<usize, LoadError> {
        let v = u64::from_le_bytes(self.take(8)?.try_into().expect("8 bytes"));
        usize::try_from(v).map_err(|_| LoadError::NotABoard)
    }
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
    let mut reader = Reader(&bytes[HEADER_LEN..]);
    let snapshot = if version == 1 {
        reader.take(reader.0.len())?
    } else {
        let len = reader.len()?;
        reader.take(len)?
    };
    let mut doc = Document::from_snapshot(snapshot).map_err(|err| match err {
        pb_core::Error::Crdt(_) => LoadError::NotABoard,
        err => LoadError::Document(err),
    })?;
    if version >= 2 {
        for _ in 0..reader.u32()? {
            let id = AssetId::from_bytes(reader.take(32)?.try_into().expect("32 bytes"));
            let len = reader.len()?;
            let data = reader.take(len)?;
            // A damaged file shows as a missing image rather than failing the whole board.
            if doc.add_asset(data) != id {
                log::warn!("asset {id} is damaged, skipping it");
            }
        }
    }
    Ok(doc)
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
            tags: Default::default(),
            link: None,
        };
        let ids = doc
            .apply(Command::Create(vec![data.clone().into(), data.into()]))
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
    fn prepared_on_another_thread_reads_back_the_same() {
        let mut doc = board();
        let asset = doc.add_asset(b"fake png".to_vec());
        let picture = ObjectData {
            kind: pb_core::ObjectKind::Image(pb_core::Image {
                asset,
                crop: Rect::new(0.0, 0.0, 1.0, 1.0),
                book: None,
            }),
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        doc.apply(Command::Create(vec![picture.into()])).unwrap();
        let prepared = prepare(&doc);
        let bytes = std::thread::spawn(move || prepared.encode())
            .join()
            .unwrap();
        let loaded = decode(&bytes).unwrap();
        assert_eq!(loaded.objects(), doc.objects());
        assert!(loaded.assets().get(asset).is_some());
    }

    proptest::proptest! {
        /// Damaged files, random bytes or real files cut or with bytes changed, are errors,
        /// never panics.
        #[test]
        fn any_bytes_are_read_without_panics(
            random in proptest::collection::vec(proptest::num::u8::ANY, 0..256),
            cut in 0..2000usize,
            flips in proptest::collection::vec((0..2000usize, proptest::num::u8::ANY), 0..8),
        ) {
            let _ = decode(&random);
            let good = encode(&board());
            let _ = decode(&good[..cut.min(good.len())]);
            let mut changed = good.clone();
            for (at, byte) in flips {
                let at = at % changed.len();
                changed[at] = byte;
            }
            let _ = decode(&changed);
        }
    }

    #[test]
    fn rejects_foreign_and_damaged_files() {
        assert!(matches!(decode(b"PNG..."), Err(LoadError::NotABoard)));
        let mut bytes = encode(&board());
        bytes.truncate(bytes.len() / 2);
        assert!(matches!(decode(&bytes), Err(LoadError::NotABoard)));
    }

    #[test]
    fn assets_of_images_are_kept() {
        use pb_core::{Image, ObjectKind};
        let mut doc = board();
        let used = doc.add_asset(b"used".to_vec());
        let unused = doc.add_asset(b"unused".to_vec());
        let data = ObjectData {
            kind: ObjectKind::Image(Image::new(used)),
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        doc.apply(Command::Create(vec![data.into()])).unwrap();
        let loaded = decode(&encode(&doc)).unwrap();
        assert_eq!(loaded.objects(), doc.objects());
        assert_eq!(&**loaded.assets().get(used).unwrap(), b"used");
        assert!(!loaded.assets().contains(unused));
    }

    #[test]
    fn reads_version_1() {
        let doc = board();
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&doc.snapshot());
        assert_eq!(decode(&bytes).unwrap().objects(), doc.objects());
    }

    #[test]
    fn rejects_newer_format() {
        let mut bytes = encode(&board());
        bytes[MAGIC.len()..HEADER_LEN].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        assert!(matches!(decode(&bytes), Err(LoadError::NewerFormat(_))));
    }
}
