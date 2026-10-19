//! Binary assets (image files) addressed by their content hash.
//!
//! Assets are kept beside the CRDT rather than in it: they never change once added, so they
//! need no merging, and later they can travel between replicas on their own, fetched by hash
//! only when missing.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// BLAKE3 hash of an asset's bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId([u8; 32]);

impl AssetId {
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Reads the stored (hex) form.
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() != 64 || !s.is_ascii() {
            return None;
        }
        let mut bytes = [0; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(bytes))
    }
}

/// Lowercase hex; this is also the stored form.
impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetId({})", &self.to_string()[..12])
    }
}

/// Asset bytes by id. Adding the same bytes twice keeps one copy.
#[derive(Clone, Default)]
pub struct Assets {
    map: HashMap<AssetId, Arc<[u8]>>,
}

impl Assets {
    pub fn insert(&mut self, bytes: impl Into<Arc<[u8]>>) -> AssetId {
        let bytes = bytes.into();
        let id = AssetId::of(&bytes);
        self.map.entry(id).or_insert(bytes);
        id
    }

    pub fn get(&self, id: AssetId) -> Option<&Arc<[u8]>> {
        self.map.get(&id)
    }

    pub fn contains(&self, id: AssetId) -> bool {
        self.map.contains_key(&id)
    }

    pub fn ids(&self) -> impl Iterator<Item = AssetId> + '_ {
        self.map.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl fmt::Debug for Assets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.map.keys()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrips_through_hex() {
        let id = AssetId::of(b"picture");
        let hex = id.to_string();
        assert_eq!(hex.len(), 64);
        assert_eq!(AssetId::parse(&hex), Some(id));
        assert_eq!(AssetId::parse("xyz"), None);
        assert_eq!(AssetId::parse(&"g".repeat(64)), None);
    }

    #[test]
    fn same_bytes_are_stored_once() {
        let mut assets = Assets::default();
        let a = assets.insert(b"one".to_vec());
        let b = assets.insert(b"one".to_vec());
        let c = assets.insert(b"two".to_vec());
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(assets.len(), 2);
        assert_eq!(&**assets.get(c).unwrap(), b"two");
    }
}
