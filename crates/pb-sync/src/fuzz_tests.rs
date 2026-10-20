//! What a server takes from anyone on the internet must never bring it down: random and
//! damaged messages are refused, not panicked on, and a room survives whatever a client
//! sends. `PROPTEST_CASES` runs more cases.

use pb_core::{Command, Document, ObjectData, Style};
use proptest::prelude::*;

use crate::{Message, Presence, Room};

/// A real board change, as a client would send it.
fn real_update() -> Vec<u8> {
    let mut doc = Document::new();
    doc.start_sharing();
    let data = ObjectData {
        kind: pb_geom::ShapeKind::Rectangle.into(),
        bounds: kurbo::Rect::new(0.0, 0.0, 10.0, 10.0),
        rotation: 0.0,
        style: Style::default(),
        tags: Default::default(),
        link: None,
    };
    doc.apply(Command::Create(vec![data.into()])).unwrap();
    doc.take_local_updates().concat()
}

/// Valid messages of every kind, encoded.
fn real_messages() -> Vec<Vec<u8>> {
    [
        Message::Hello {
            protocol: crate::PROTOCOL,
            board: "b1".into(),
            token: "t".into(),
            name: "Ann".into(),
            version: vec![1, 2],
            assets: vec![pb_core::AssetId::of(b"x")],
        },
        Message::Welcome {
            client: 1,
            can_edit: true,
            updates: real_update(),
            version: vec![],
            assets: vec![],
        },
        Message::Updates(real_update()),
        Message::Asset(vec![1, 2, 3]),
        Message::Presence(Presence {
            client: 0,
            name: "Bob".into(),
            color: 1,
            cursor: Some((1.0, 2.0)),
            selection: vec!["1@2".into()],
            view: Some([0.0, 0.0, 1.0, 1.0]),
            leading: false,
        }),
        Message::Left(3),
        Message::Error("x".into()),
    ]
    .iter()
    .map(Message::encode)
    .collect()
}

/// `bytes` with some of them changed, cut short, or added to.
fn damaged(bytes: &[u8], edits: &[(usize, u8)], cut: Option<usize>, extra: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for &(at, value) in edits {
        if !out.is_empty() {
            let i = at % out.len();
            out[i] = value;
        }
    }
    if let Some(cut) = cut {
        out.truncate(cut % (out.len() + 1));
    }
    out.extend_from_slice(extra);
    out
}

proptest! {
    #[test]
    fn random_bytes_are_refused_or_read_consistently(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        if let Ok(message) = Message::decode(&bytes) {
            // What was read writes back to something read the same way.
            let again = message.encode();
            let reread = Message::decode(&again).expect("an encoded message reads");
            prop_assert_eq!(reread.encode(), again);
        }
    }

    #[test]
    fn damaged_messages_never_panic(
        which in 0usize..7,
        edits in prop::collection::vec((any::<usize>(), any::<u8>()), 0..6),
        cut in prop::option::of(any::<usize>()),
        extra in prop::collection::vec(any::<u8>(), 0..8),
    ) {
        let messages = real_messages();
        let bytes = damaged(&messages[which], &edits, cut, &extra);
        let _ = Message::decode(&bytes);
    }

    #[test]
    fn a_room_survives_what_a_client_sends(
        sends in prop::collection::vec(
            (0usize..7, prop::collection::vec((any::<usize>(), any::<u8>()), 0..4), prop::option::of(any::<usize>())),
            1..12,
        ),
        version in prop::collection::vec(any::<u8>(), 0..24),
    ) {
        let mut room = Room::new();
        // A client with a made-up version vector is refused or let in, never a panic.
        let _ = room.join(1, true, &version, &[]);
        room.join(2, true, &[], &[]).expect("an empty version joins");
        let messages = real_messages();
        for (which, edits, cut) in sends {
            let bytes = damaged(&messages[which], &edits, cut, &[]);
            if let Ok(message) = Message::decode(&bytes) {
                let _ = room.receive(2, message);
            }
            // Loro updates are taken in whole, even damaged ones that still decode.
            let _ = room.receive(2, Message::Updates(damaged(&real_update(), &edits, cut, &[])));
        }
        // The room still works: it can be saved and loaded, and someone new can join.
        let snapshot = room.snapshot();
        let mut loaded = Room::load(&snapshot, Vec::new()).expect("a room's own snapshot loads");
        loaded.join(3, false, &[], &[]).expect("joins after the storm");
    }
}
