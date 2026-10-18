//! Stored forms read back from anything, and links from any typed text, without panics.

use proptest::prelude::*;

proptest! {
    #[test]
    fn stored_forms_read_anything(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let _ = crate::Version::from_bytes("v", &bytes);
        let _ = crate::Talktrack::from_bytes("t", &bytes);
    }

    #[test]
    fn versions_round_trip(name in ".*", author in ".*", at_ms in any::<i64>(),
                           frontiers in prop::collection::vec(any::<u8>(), 0..64)) {
        let version = crate::Version {
            id: "v".into(),
            name: name.into(),
            author: author.into(),
            at_ms,
            frontiers: frontiers.into(),
            assets: vec![crate::AssetId::of(b"x")].into(),
        };
        prop_assert_eq!(crate::Version::from_bytes("v", &version.to_bytes()), Some(version));
    }

    #[test]
    fn links_come_from_any_text(text in ".*") {
        let _ = crate::Link::from_input(&text);
        let _ = crate::ObjectId::parse(&text);
        let _ = crate::AssetId::parse(&text);
    }
}
