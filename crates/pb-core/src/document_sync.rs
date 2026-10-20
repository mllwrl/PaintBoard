//! Exchanging changes with other copies of the board (see `pb-sync`).
//!
//! Every copy that edits at the same time needs its own peer id. Each local edit becomes
//! an update to send to the others ([`Document::take_local_updates`]); updates from them
//! are merged in with [`Document::import_updates`]. Version vectors say which changes a
//! copy has, so two copies that meet send each other only what the other lacks.

use std::sync::{Arc, Mutex};

use loro::{ExportMode, VersionVector};

use super::{Document, Error, Result};

/// Local changes not yet sent, collected while the board is shared.
pub(super) struct Outbox {
    updates: Arc<Mutex<Vec<Vec<u8>>>>,
    _subscription: loro::Subscription,
}

impl Document {
    /// Makes this copy's changes those of `peer`. Two copies editing at the same time must
    /// not share one, so pick it at random each time the board is opened for sharing.
    pub fn set_peer(&mut self, peer: u64) -> Result<()> {
        Ok(self.doc.set_peer_id(peer)?)
    }

    pub fn peer(&self) -> u64 {
        self.doc.peer_id()
    }

    /// Which changes this copy has, to ask another copy for the rest.
    pub fn sync_version(&self) -> Vec<u8> {
        self.doc.oplog_vv().encode()
    }

    /// The changes this copy has that a copy at `version` (from [`Document::sync_version`])
    /// lacks; empty `version` means all of them.
    pub fn updates_since(&self, version: &[u8]) -> Result<Vec<u8>> {
        let from = if version.is_empty() {
            VersionVector::default()
        } else {
            VersionVector::decode(version)?
        };
        Ok(self.doc.export(ExportMode::updates(&from))?)
    }

    /// Merges in changes from another copy. Returns whether the board changed. They are
    /// not added to undo: Ctrl+Z takes back only this copy's own edits.
    pub fn import_updates(&mut self, bytes: &[u8]) -> Result<bool> {
        let before = self.doc.oplog_vv();
        self.doc.import(bytes)?;
        if self.doc.oplog_vv() == before {
            return Ok(false);
        }
        self.refresh();
        Ok(true)
    }

    /// Starts collecting local changes for [`Document::take_local_updates`].
    pub fn start_sharing(&mut self) {
        if self.outbox.is_some() {
            return;
        }
        let updates = Arc::new(Mutex::new(Vec::new()));
        let sink = updates.clone();
        let subscription = self.doc.subscribe_local_update(Box::new(move |bytes| {
            if let Ok(mut updates) = sink.lock() {
                updates.push(bytes.clone());
            }
            true
        }));
        self.outbox = Some(Outbox {
            updates,
            _subscription: subscription,
        });
    }

    pub fn stop_sharing(&mut self) {
        self.outbox = None;
    }

    /// Local changes since the last call, to send to the other copies, oldest first.
    pub fn take_local_updates(&mut self) -> Vec<Vec<u8>> {
        self.outbox
            .as_ref()
            .and_then(|o| o.updates.lock().ok().map(|mut u| std::mem::take(&mut *u)))
            .unwrap_or_default()
    }
}

impl From<loro::LoroEncodeError> for Error {
    fn from(err: loro::LoroEncodeError) -> Self {
        Error::Crdt(loro::LoroError::DecodeError(err.to_string().into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, ObjectData, ObjectId, Style};

    /// Adds a rectangle; returns its id.
    fn rect(doc: &mut Document) -> ObjectId {
        let data = ObjectData {
            kind: pb_geom::ShapeKind::Rectangle.into(),
            bounds: kurbo::Rect::new(0.0, 0.0, 10.0, 10.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        doc.apply(Command::Create(vec![data.into()])).unwrap();
        doc.objects().last().unwrap().id
    }

    #[test]
    fn copies_exchange_only_what_the_other_lacks() {
        let mut a = Document::new();
        a.set_peer(1).unwrap();
        let mut b = Document::from_snapshot(&a.snapshot()).unwrap();
        b.set_peer(2).unwrap();
        a.start_sharing();
        let first = rect(&mut a);
        let sent = a.take_local_updates();
        assert_eq!(sent.len(), 1);
        for update in &sent {
            assert!(b.import_updates(update).unwrap());
        }
        assert!(b.get(first).is_some());
        // Importing again changes nothing.
        assert!(!b.import_updates(&sent[0]).unwrap());

        let second = rect(&mut b);
        let missing = b.updates_since(&a.sync_version()).unwrap();
        assert!(a.import_updates(&missing).unwrap());
        assert!(a.get(second).is_some());
        // A remote change is not this copy's to undo.
        assert!(a.take_local_updates().is_empty());
        assert!(a.undo().unwrap());
        assert!(a.get(first).is_none());
        assert!(a.get(second).is_some());
    }

    fn set_text(doc: &mut Document, id: ObjectId, text: &str) {
        let bounds = doc.get(id).unwrap().data.bounds;
        doc.apply(Command::SetText {
            id,
            text: text.into(),
            spans: Default::default(),
            bounds,
        })
        .unwrap();
    }

    fn text_of(doc: &Document, id: ObjectId) -> String {
        doc.get(id).unwrap().data.kind.text().unwrap().to_string()
    }

    #[test]
    fn edits_of_the_same_text_at_once_both_stay() {
        let mut a = Document::new();
        a.set_peer(1).unwrap();
        let id = rect(&mut a);
        set_text(&mut a, id, "Hello world");
        let mut b = Document::from_snapshot(&a.snapshot()).unwrap();
        b.set_peer(2).unwrap();

        set_text(&mut a, id, "Hello big world");
        set_text(&mut b, id, "Hello world!");
        let from_a = a.updates_since(&b.sync_version()).unwrap();
        let from_b = b.updates_since(&a.sync_version()).unwrap();
        a.import_updates(&from_b).unwrap();
        b.import_updates(&from_a).unwrap();
        assert_eq!(text_of(&a, id), "Hello big world!");
        assert_eq!(text_of(&b, id), "Hello big world!");
    }

    #[test]
    fn text_from_older_boards_reads_and_edits() {
        let mut doc = Document::new();
        let id = rect(&mut doc);
        // Boards before text containers kept a plain string.
        let meta = doc.tree.get_meta(doc.nodes[&id]).unwrap();
        meta.insert(super::super::key::TEXT, "old text").unwrap();
        doc.doc.commit();
        let mut doc = Document::from_snapshot(&doc.snapshot()).unwrap();
        assert_eq!(text_of(&doc, id), "old text");
        set_text(&mut doc, id, "new text");
        assert_eq!(text_of(&doc, id), "new text");
        assert!(doc.undo().unwrap());
        assert_eq!(text_of(&doc, id), "old text");
    }

    #[test]
    fn edits_of_the_same_cell_at_once_both_stay() {
        use crate::{Cell, ObjectKind, Table};

        let mut a = Document::new();
        a.set_peer(1).unwrap();
        let mut n = 0;
        let table = Table::new(2, 2, || {
            n += 1;
            format!("l{n}")
        });
        let (row, col) = (table.rows[0].id.clone(), table.cols[0].id.clone());
        let data = ObjectData {
            kind: ObjectKind::Table(table),
            bounds: kurbo::Rect::new(0.0, 0.0, 200.0, 100.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        a.apply(Command::Create(vec![data.into()])).unwrap();
        let id = a.objects().last().unwrap().id;
        let set = |doc: &mut Document, text: &str| {
            doc.apply(Command::SetCell {
                id,
                row: row.clone(),
                col: col.clone(),
                cell: Cell {
                    text: text.into(),
                    spans: Default::default(),
                },
            })
            .unwrap();
        };
        set(&mut a, "two words");
        let mut b = Document::from_snapshot(&a.snapshot()).unwrap();
        b.set_peer(2).unwrap();
        set(&mut a, "two short words");
        set(&mut b, "two words!");
        let from_a = a.updates_since(&b.sync_version()).unwrap();
        let from_b = b.updates_since(&a.sync_version()).unwrap();
        a.import_updates(&from_b).unwrap();
        b.import_updates(&from_a).unwrap();
        let cell = |doc: &Document| match &doc.get(id).unwrap().data.kind {
            ObjectKind::Table(t) => t.cell(0, 0).map(|c| c.text.to_string()),
            _ => None,
        };
        assert_eq!(cell(&a).as_deref(), Some("two short words!"));
        assert_eq!(cell(&b), cell(&a));
        // Emptied, the cell is gone from the table, as before.
        set(&mut a, "");
        assert_eq!(cell(&a), None);
    }

    /// Two copies of `doc`, as if opened by two people.
    fn two(doc: &Document) -> (Document, Document) {
        let mut a = Document::from_snapshot(&doc.snapshot()).unwrap();
        let mut b = Document::from_snapshot(&doc.snapshot()).unwrap();
        a.set_peer(11).unwrap();
        b.set_peer(12).unwrap();
        (a, b)
    }

    /// Each copy takes in the other's changes.
    fn exchange(a: &mut Document, b: &mut Document) {
        let from_a = a.updates_since(&b.sync_version()).unwrap();
        let from_b = b.updates_since(&a.sync_version()).unwrap();
        a.import_updates(&from_b).unwrap();
        b.import_updates(&from_a).unwrap();
        let ids = |d: &Document| d.objects().iter().map(|o| o.id).collect::<Vec<_>>();
        assert_eq!(ids(a), ids(b));
    }

    #[test]
    fn a_moved_object_someone_deleted_stays_deleted() {
        let mut doc = Document::new();
        let id = rect(&mut doc);
        let (mut a, mut b) = two(&doc);
        a.apply(Command::Delete(vec![id])).unwrap();
        b.apply(Command::Move {
            ids: vec![id],
            delta: kurbo::Vec2::new(10.0, 0.0),
        })
        .unwrap();
        set_text(&mut b, id, "edited meanwhile");
        exchange(&mut a, &mut b);
        assert!(a.get(id).is_none() && b.get(id).is_none());
    }

    #[test]
    fn a_connector_to_a_deleted_object_is_harmless() {
        use crate::{Anchor, Cap, Connector, End, Label, Route};

        let mut doc = Document::new();
        let first = rect(&mut doc);
        let second = rect(&mut doc);
        let (mut a, mut b) = two(&doc);
        a.apply(Command::Delete(vec![second])).unwrap();
        let end = |object| End::Attached {
            object,
            anchor: Anchor::Auto,
            at: kurbo::Point::ZERO,
        };
        let data = ObjectData {
            kind: crate::ObjectKind::Connector(Connector {
                bends: Default::default(),
                start: end(first),
                end: end(second),
                route: Route::Straight,
                start_cap: Cap::None,
                end_cap: Cap::Arrow,
                label: Label::default(),
            }),
            bounds: kurbo::Rect::ZERO,
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        b.apply(Command::Create(vec![data.into()])).unwrap();
        exchange(&mut a, &mut b);
        // Both copies agree on what is left and can still draw and save it.
        for doc in [&a, &b] {
            for object in doc.objects() {
                let _ = doc.connector_path(object.id);
            }
            Document::from_snapshot(&doc.snapshot()).unwrap();
        }
    }

    #[test]
    fn a_deleted_group_takes_what_someone_moved_in_it() {
        let mut doc = Document::new();
        let first = rect(&mut doc);
        let second = rect(&mut doc);
        doc.apply(Command::Group(vec![first, second])).unwrap();
        let group = doc.parent(first).unwrap();
        let (mut a, mut b) = two(&doc);
        a.apply(Command::Delete(vec![group])).unwrap();
        b.apply(Command::Move {
            ids: vec![first],
            delta: kurbo::Vec2::new(5.0, 5.0),
        })
        .unwrap();
        exchange(&mut a, &mut b);
        assert!(a.get(first).is_none() && a.get(second).is_none());
    }

    #[test]
    fn the_board_name_reaches_the_others() {
        let mut doc = Document::new();
        assert_eq!(doc.title(), None);
        let (mut a, mut b) = two(&doc);
        a.apply(Command::SetTitle("Roadmap".into())).unwrap();
        exchange(&mut a, &mut b);
        assert_eq!(b.title().as_deref(), Some("Roadmap"));
        doc.apply(Command::SetTitle("  ".into())).unwrap();
        assert_eq!(doc.title(), None);
    }

    #[test]
    fn edits_of_a_label_or_page_title_at_once_both_stay() {
        use crate::{Cap, Connector, Embed, End, Label, ObjectKind, Route};

        let mut doc = Document::new();
        let free = |x| End::Free(kurbo::Point::new(x, 0.0));
        let data = |kind| ObjectData {
            kind,
            bounds: kurbo::Rect::new(0.0, 0.0, 100.0, 10.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        let connector = ObjectKind::Connector(Connector {
            bends: Default::default(),
            start: free(0.0),
            end: free(100.0),
            route: Route::Straight,
            start_cap: Cap::None,
            end_cap: Cap::Arrow,
            label: Label::default(),
        });
        let page = ObjectKind::Embed(Embed {
            url: "https://example.com".into(),
            title: "Example page".into(),
            description: "".into(),
            site: "example.com".into(),
        });
        doc.apply(Command::Create(vec![
            data(connector).into(),
            data(page).into(),
        ]))
        .unwrap();
        let (line, card) = (doc.objects()[0].id, doc.objects()[1].id);
        let label = |text: &str| Label {
            text: text.into(),
            at: 0.5,
            size: kurbo::Size::new(40.0, 12.0),
        };
        doc.apply(Command::SetLabel {
            id: line,
            label: label("yes no"),
        })
        .unwrap();
        let (mut a, mut b) = two(&doc);
        a.apply(Command::SetLabel {
            id: line,
            label: label("yes or no"),
        })
        .unwrap();
        b.apply(Command::SetLabel {
            id: line,
            label: label("yes no?"),
        })
        .unwrap();
        set_text(&mut a, card, "An example page");
        set_text(&mut b, card, "Example web page");
        exchange(&mut a, &mut b);
        for doc in [&a, &b] {
            let ObjectKind::Connector(c) = &doc.get(line).unwrap().data.kind else {
                panic!("a connector");
            };
            assert_eq!(&*c.label.text, "yes or no?");
            assert_eq!(text_of(doc, card), "An example web page");
        }
    }
}
