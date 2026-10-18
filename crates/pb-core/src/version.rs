//! Saved versions of the board: named points in its history to look back at and restore.

use std::sync::Arc;

use crate::{AssetId, Document, ObjectId};

/// A saved version.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub id: Arc<str>,
    pub name: Arc<str>,
    pub author: Arc<str>,
    /// When it was saved, in milliseconds since the Unix epoch.
    pub at_ms: i64,
    /// The point in the board's history (encoded CRDT frontiers).
    pub frontiers: Arc<[u8]>,
    /// Files its images used, kept in the board's file while the version is.
    pub assets: Arc<[AssetId]>,
}

/// The stored form's version.
const FORMAT: u8 = 1;

impl Version {
    /// The stored form: format, name and author (length-prefixed UTF-8), frontiers
    /// (length-prefixed), time, then the asset hashes (count-prefixed), all little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![FORMAT];
        for bytes in [
            self.name.as_bytes(),
            self.author.as_bytes(),
            &self.frontiers,
        ] {
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        out.extend_from_slice(&self.at_ms.to_le_bytes());
        out.extend_from_slice(&(self.assets.len() as u32).to_le_bytes());
        for asset in self.assets.iter() {
            out.extend_from_slice(asset.as_bytes());
        }
        out
    }

    /// Reads [`Self::to_bytes`]; `None` if it is not that.
    pub fn from_bytes(id: &str, bytes: &[u8]) -> Option<Self> {
        let mut rest = bytes;
        let mut take = |n: usize| -> Option<&[u8]> {
            let (head, tail) = rest.split_at_checked(n)?;
            rest = tail;
            Some(head)
        };
        if take(1)? != [FORMAT] {
            return None;
        }
        let mut field = || -> Option<Vec<u8>> {
            let len = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
            take(len).map(<[u8]>::to_vec)
        };
        let name = String::from_utf8(field()?).ok()?;
        let author = String::from_utf8(field()?).ok()?;
        let frontiers = field()?;
        let at_ms = i64::from_le_bytes(take(8)?.try_into().ok()?);
        let count = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        let assets = (0..count)
            .map(|_| Some(AssetId::from_bytes(take(32)?.try_into().ok()?)))
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            id: id.into(),
            name: name.into(),
            author: author.into(),
            at_ms,
            frontiers: frontiers.into(),
            assets: assets.into(),
        })
    }
}

/// How the board changed between two versions of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    /// In both, but different now.
    pub changed: Vec<ObjectId>,
    /// Only in the earlier one.
    pub removed: Vec<ObjectId>,
    /// Only in the later one.
    pub added: Vec<ObjectId>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty() && self.added.is_empty()
    }
}

/// What changed on the board from `then` to `now`, object by object, in `then`'s paint
/// order (added ones in `now`'s).
pub fn compare(then: &Document, now: &Document) -> Changes {
    let mut changes = Changes::default();
    for object in then.objects() {
        match now.get(object.id) {
            Some(current) if current.data != object.data => changes.changed.push(object.id),
            Some(_) => {}
            None => changes.removed.push(object.id),
        }
    }
    changes.added = now
        .objects()
        .iter()
        .filter(|o| then.get(o.id).is_none())
        .map(|o| o.id)
        .collect();
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_form_round_trips() {
        let version = Version {
            id: "v1".into(),
            name: "Перед ревью".into(),
            author: "Alex".into(),
            at_ms: 1_760_000_000_000,
            frontiers: vec![1, 2, 3].into(),
            assets: vec![AssetId::of(b"a"), AssetId::of(b"b")].into(),
        };
        let bytes = version.to_bytes();
        assert_eq!(Version::from_bytes("v1", &bytes), Some(version));
        assert_eq!(Version::from_bytes("v1", &bytes[..bytes.len() - 1]), None);
        assert_eq!(Version::from_bytes("v1", b""), None);
    }
}
