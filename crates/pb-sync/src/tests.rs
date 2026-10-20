//! Boards shared through a room over in-memory connections.

use std::collections::BTreeMap;

use kurbo::{Rect, Vec2};
use pb_core::{
    Command, Comment, Document, ObjectData, ObjectId, Role, ShapeKind, Style, ThreadAnchor,
};

use crate::{Client, ClientId, Event, Memory, Message, Room, Transport};

/// A room and the server ends of its connections.
struct Server {
    room: Room,
    links: BTreeMap<ClientId, Memory>,
    next: ClientId,
}

impl Server {
    fn new() -> Self {
        Self {
            room: Room::new(),
            links: BTreeMap::new(),
            next: 1,
        }
    }

    /// Opens a connection; the board's end is returned.
    fn connect(&mut self) -> Memory {
        let (server, client) = Memory::pair();
        self.links.insert(self.next, server);
        self.next += 1;
        client
    }

    /// Handles everything that arrived. Tokens: "edit" may edit, "comment" may comment,
    /// others only look.
    fn pump(&mut self) {
        let ids: Vec<ClientId> = self.links.keys().copied().collect();
        for id in ids {
            while let Some(message) = self.links.get_mut(&id).and_then(Memory::try_recv) {
                let out = match message {
                    Message::Hello {
                        token,
                        version,
                        assets,
                        ..
                    } => {
                        let role = match token.as_str() {
                            "edit" => Role::Edit,
                            "comment" => Role::Comment,
                            _ => Role::View,
                        };
                        self.room.join(id, role, &version, &assets)
                    }
                    other => self.room.receive(id, other),
                };
                match out {
                    Ok(out) => self.send(out),
                    Err(err) => {
                        if let Some(link) = self.links.get_mut(&id) {
                            link.send(&Message::Error(err));
                        }
                    }
                }
            }
        }
    }

    fn drop_client(&mut self, id: ClientId) {
        if let Some(link) = self.links.remove(&id) {
            link.close();
        }
        let out = self.room.leave(id);
        self.send(out);
    }

    fn send(&mut self, out: crate::Outgoing) {
        for (to, message) in out {
            if let Some(link) = self.links.get_mut(&to) {
                link.send(&message);
            }
        }
    }
}

/// A board connected to the server.
struct Peer {
    doc: Document,
    client: Client,
    link: Memory,
    events: Vec<Event>,
}

impl Peer {
    fn join(server: &mut Server, doc: Document, token: &str) -> Self {
        let mut peer = Self {
            doc,
            client: Client::new("board", token, "someone"),
            link: server.connect(),
            events: Vec::new(),
        };
        let hello = peer.client.hello(&mut peer.doc);
        peer.link.send(&hello);
        peer
    }

    /// Sends local edits and handles what arrived.
    fn pump(&mut self) {
        for message in self.client.outgoing(&mut self.doc) {
            self.link.send(&message);
        }
        let mut replies = Vec::new();
        while let Some(message) = self.link.try_recv() {
            let events = self.client.receive(message, &mut self.doc, &mut replies);
            self.events.extend(events);
        }
        for message in replies {
            self.link.send(&message);
        }
    }

    fn add_rect(&mut self, x: f64) -> ObjectId {
        let data = ObjectData {
            kind: ShapeKind::Rectangle.into(),
            bounds: Rect::new(x, 0.0, x + 10.0, 10.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        self.doc.apply(Command::Create(vec![data.into()])).unwrap();
        self.doc.objects().last().unwrap().id
    }

    /// The board as plain data, to compare copies.
    fn view(&self) -> Vec<(String, Rect)> {
        let mut objects: Vec<_> = self
            .doc
            .objects()
            .iter()
            .map(|o| (o.id.to_string(), o.data.bounds))
            .collect();
        objects.sort_by(|a, b| a.0.cmp(&b.0));
        objects
    }
}

fn settle(server: &mut Server, peers: &mut [&mut Peer]) {
    for _ in 0..4 {
        for peer in peers.iter_mut() {
            peer.pump();
        }
        server.pump();
    }
}

fn shared_doc(peer: u64) -> Document {
    let mut doc = Document::new();
    doc.set_peer(peer).unwrap();
    doc
}

#[test]
fn edits_reach_everyone() {
    let mut server = Server::new();
    let mut a = Peer::join(&mut server, shared_doc(1), "edit");
    let mut b = Peer::join(&mut server, shared_doc(2), "edit");
    settle(&mut server, &mut [&mut a, &mut b]);
    assert!(a.events.contains(&Event::Joined {
        client: 1,
        role: Role::Edit
    }));

    let first = a.add_rect(0.0);
    let second = b.add_rect(100.0);
    settle(&mut server, &mut [&mut a, &mut b]);
    assert_eq!(a.view(), b.view());
    assert!(b.doc.get(first).is_some() && a.doc.get(second).is_some());

    // Both move the same object at once: they still agree.
    a.doc
        .apply(Command::Move {
            ids: vec![first],
            delta: Vec2::new(5.0, 0.0),
        })
        .unwrap();
    b.doc
        .apply(Command::Move {
            ids: vec![first],
            delta: Vec2::new(0.0, 7.0),
        })
        .unwrap();
    settle(&mut server, &mut [&mut a, &mut b]);
    assert_eq!(a.view(), b.view());
}

#[test]
fn a_late_joiner_gets_the_board_and_offline_edits_are_merged() {
    let mut server = Server::new();
    let mut a = Peer::join(&mut server, shared_doc(1), "edit");
    a.add_rect(0.0);
    settle(&mut server, &mut [&mut a]);

    // B starts from its own copy of an older file with an edit of its own.
    let mut b_doc = shared_doc(2);
    let mut offline = Peer {
        doc: Document::new(),
        client: Client::new("board", "edit", "b"),
        link: Memory::pair().0,
        events: Vec::new(),
    };
    offline.doc.set_peer(2).unwrap();
    let offline_id = offline.add_rect(50.0);
    b_doc
        .import_updates(&offline.doc.updates_since(&[]).unwrap())
        .unwrap();
    let mut b = Peer::join(&mut server, b_doc, "edit");
    settle(&mut server, &mut [&mut a, &mut b]);
    assert_eq!(a.view(), b.view());
    assert_eq!(a.view().len(), 2);
    assert!(a.doc.get(offline_id).is_some());

    // A drops off, edits, and comes back.
    server.drop_client(1);
    a.client.disconnected();
    a.add_rect(200.0);
    b.add_rect(300.0);
    settle(&mut server, &mut [&mut b]);
    assert!(b.events.contains(&Event::Left(1)));
    a.link = server.connect();
    let hello = a.client.hello(&mut a.doc);
    a.link.send(&hello);
    settle(&mut server, &mut [&mut a, &mut b]);
    assert_eq!(a.view(), b.view());
    assert_eq!(a.view().len(), 4);
}

#[test]
fn lookers_cannot_change_the_board() {
    let mut server = Server::new();
    let mut a = Peer::join(&mut server, shared_doc(1), "edit");
    let mut v = Peer::join(&mut server, shared_doc(2), "view");
    a.add_rect(0.0);
    settle(&mut server, &mut [&mut a, &mut v]);
    assert_eq!(v.view().len(), 1);
    assert_eq!(v.client.joined(), Some(Role::View));
    assert!(v.doc.view_only());

    // A looker's edit (the board would not allow one) stays with them.
    v.doc.set_role(Role::Edit);
    v.add_rect(100.0);
    settle(&mut server, &mut [&mut a, &mut v]);
    assert_eq!(a.view().len(), 1);

    // Sent anyway, it is refused.
    v.link
        .send(&Message::Updates(v.doc.updates_since(&[]).unwrap()));
    settle(&mut server, &mut [&mut a, &mut v]);
    assert_eq!(a.view().len(), 1);
    assert!(v.events.iter().any(|e| matches!(e, Event::Error(_))));
}

#[test]
fn pictures_are_shared_once() {
    let mut server = Server::new();
    let mut a = Peer::join(&mut server, shared_doc(1), "edit");
    let id = a.doc.add_asset(b"a picture".to_vec());
    settle(&mut server, &mut [&mut a]);
    assert!(server.room.asset(id).is_some());

    let mut b = Peer::join(&mut server, shared_doc(2), "edit");
    settle(&mut server, &mut [&mut a, &mut b]);
    assert!(b.doc.assets().contains(id));
    // Nothing is sent again.
    assert!(a.client.outgoing(&mut a.doc).is_empty());
    assert!(b.client.outgoing(&mut b.doc).is_empty());
}

#[test]
fn presence_goes_to_the_others_only() {
    let mut server = Server::new();
    let mut a = Peer::join(&mut server, shared_doc(1), "edit");
    let mut b = Peer::join(&mut server, shared_doc(2), "view");
    settle(&mut server, &mut [&mut a, &mut b]);
    a.link.send(&Message::Presence(crate::Presence {
        name: "Ann".into(),
        cursor: Some((1.0, 2.0)),
        ..Default::default()
    }));
    settle(&mut server, &mut [&mut a, &mut b]);
    let seen: Vec<_> = b
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Presence(p) => Some((p.client, p.name.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(seen, [(1, "Ann".to_owned())]);
    assert!(!a.events.iter().any(|e| matches!(e, Event::Presence(_))));
}

/// Many random edits by three copies, delivered in bursts: all end up the same.
#[test]
fn random_edits_converge() {
    let mut server = Server::new();
    let mut peers: Vec<Peer> = (1..=3)
        .map(|n| Peer::join(&mut server, shared_doc(n), "edit"))
        .collect();
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut random = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    for round in 0..200 {
        let who = random(3) as usize;
        let peer = &mut peers[who];
        let objects: Vec<ObjectId> = peer.doc.objects().iter().map(|o| o.id).collect();
        match random(4) {
            0 | 1 => {
                peer.add_rect(random(1000) as f64);
            }
            2 if !objects.is_empty() => {
                let id = objects[random(objects.len() as u64) as usize];
                let delta = Vec2::new(random(50) as f64, random(50) as f64);
                peer.doc
                    .apply(Command::Move {
                        ids: vec![id],
                        delta,
                    })
                    .unwrap();
            }
            3 if !objects.is_empty() => {
                let id = objects[random(objects.len() as u64) as usize];
                peer.doc.apply(Command::Delete(vec![id])).unwrap();
            }
            _ => {}
        }
        // Deliver only now and then, so edits cross each other.
        if round % 7 == 0 {
            let who = random(3) as usize;
            peers[who].pump();
            server.pump();
        }
    }
    let mut refs: Vec<&mut Peer> = peers.iter_mut().collect();
    settle(&mut server, &mut refs);
    assert_eq!(peers[0].view(), peers[1].view());
    assert_eq!(peers[1].view(), peers[2].view());
}

fn comment(id: &str) -> Comment {
    Comment {
        id: id.into(),
        author: "cat".into(),
        at_ms: 1,
        text: "looks good".into(),
    }
}

#[test]
fn commenters_comment_but_cannot_edit() {
    let mut server = Server::new();
    let mut a = Peer::join(&mut server, shared_doc(1), "edit");
    // The commenter's copy has a change of its own, which the server will never get.
    let mut own = shared_doc(2);
    own.apply(Command::SetTitle("mine".into())).unwrap();
    let mut c = Peer::join(&mut server, own, "comment");
    let rect = a.add_rect(0.0);
    settle(&mut server, &mut [&mut a, &mut c]);
    assert_eq!(c.client.joined(), Some(Role::Comment));
    assert_eq!(c.doc.title(), None);
    assert_eq!(c.view(), a.view());
    assert_eq!(c.doc.role(), Role::Comment);
    assert!(c.doc.apply(Command::Delete(vec![rect])).is_err());

    // Comments and reactions reach the others; the commenter took a new peer for them.
    assert_ne!(c.doc.peer(), 2);
    c.doc
        .apply(Command::AddComment {
            thread: "t".into(),
            anchor: Some(ThreadAnchor::Object(rect)),
            comment: comment("1"),
        })
        .unwrap();
    c.doc
        .apply(Command::React {
            id: rect,
            reaction: pb_core::Reaction::ALL[0],
            author: "cat".into(),
            on: true,
        })
        .unwrap();
    settle(&mut server, &mut [&mut a, &mut c]);
    assert!(
        !c.events.iter().any(|e| matches!(e, Event::Error(_))),
        "{:?}",
        c.events
    );
    assert_eq!(a.doc.threads().len(), 1);
    assert_eq!(a.doc.reactions(rect).len(), 1);
    // An editor's reply gets back to the commenter.
    a.doc
        .apply(Command::AddComment {
            thread: "t".into(),
            anchor: None,
            comment: comment("2"),
        })
        .unwrap();
    settle(&mut server, &mut [&mut a, &mut c]);
    assert_eq!(c.doc.threads()[0].comments.len(), 2);
    assert!(!c.events.iter().any(|e| matches!(e, Event::Error(_))));

    // An edit sneaked in (the board would not allow one) is refused as remarks too.
    c.doc.set_role(Role::Edit);
    c.add_rect(100.0);
    settle(&mut server, &mut [&mut a, &mut c]);
    assert_eq!(a.view().len(), 1);
    assert!(c.events.iter().any(|e| matches!(e, Event::Error(_))));
}

#[test]
fn remarks_cannot_pass_for_an_editors_changes() {
    let mut server = Server::new();
    let mut a = Peer::join(&mut server, shared_doc(1), "edit");
    let mut c = Peer::join(&mut server, shared_doc(2), "comment");
    let rect = a.add_rect(0.0);
    settle(&mut server, &mut [&mut a, &mut c]);
    // A comment written as the editor's peer, continuing the editor's changes.
    let mut forged = Document::from_snapshot(&a.doc.snapshot()).unwrap();
    forged.set_peer(1).unwrap();
    let from = a.doc.own_changes_json(0).unwrap().unwrap().1;
    forged
        .apply(Command::AddComment {
            thread: "t".into(),
            anchor: Some(ThreadAnchor::Object(rect)),
            comment: comment("1"),
        })
        .unwrap();
    let (json, _) = forged.own_changes_json(from).unwrap().unwrap();
    c.link.send(&Message::Remarks(json));
    settle(&mut server, &mut [&mut a, &mut c]);
    assert!(a.doc.threads().is_empty());
    assert!(c.events.iter().any(|e| matches!(e, Event::Error(_))));
}

#[test]
fn lookers_cannot_send_remarks() {
    let mut server = Server::new();
    let mut v = Peer::join(&mut server, shared_doc(2), "view");
    settle(&mut server, &mut [&mut v]);
    v.link.send(&Message::Remarks("{}".into()));
    settle(&mut server, &mut [&mut v]);
    assert!(v.events.iter().any(|e| matches!(e, Event::Error(_))));
}
