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
}
