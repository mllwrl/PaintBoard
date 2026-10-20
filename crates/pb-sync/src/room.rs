//! A board on the server with everyone connected to it. No networking: the server feeds
//! it messages and sends out what it returns.
//!
//! The room does not understand the board: it merges the CRDT updates it is sent (which
//! also checks they are well formed), keeps pictures and files by hash, and passes
//! everything on to the others.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use loro::{ExportMode, LoroDoc, VersionVector};
use pb_core::{AssetId, Role};

use crate::message::{ClientId, Message, Presence};

/// The largest picture or file the room takes.
pub const MAX_ASSET: usize = 64 << 20;

/// The most all of a board's pictures and files may take together.
pub const MAX_ASSETS_TOTAL: usize = 512 << 20;

/// The most one message of remarks may take: comments are short.
pub const MAX_REMARKS: usize = 1 << 20;

/// Messages to send: to whom, and what.
pub type Outgoing = Vec<(ClientId, Message)>;

struct Member {
    role: Role,
    /// Peers whose changes came from this member as remarks: only they may add more.
    peers: HashSet<u64>,
    presence: Option<Presence>,
}

pub struct Room {
    doc: LoroDoc,
    assets: HashMap<AssetId, Arc<[u8]>>,
    members: BTreeMap<ClientId, Member>,
    /// Changed since the last [`Room::take_changed`].
    changed: bool,
}

impl Default for Room {
    fn default() -> Self {
        Self::new()
    }
}

impl Room {
    pub fn new() -> Self {
        Self {
            doc: LoroDoc::new(),
            assets: HashMap::new(),
            members: BTreeMap::new(),
            changed: false,
        }
    }

    /// The room for a board stored as [`Room::snapshot`] and its assets.
    pub fn load(
        snapshot: &[u8],
        assets: impl IntoIterator<Item = Vec<u8>>,
    ) -> Result<Self, String> {
        let doc = LoroDoc::from_snapshot(snapshot).map_err(|e| e.to_string())?;
        let mut room = Self::new();
        room.doc = doc;
        for bytes in assets {
            room.assets.insert(AssetId::of(&bytes), bytes.into());
        }
        Ok(room)
    }

    /// The whole board, history included, to store.
    pub fn snapshot(&self) -> Vec<u8> {
        self.doc
            .export(ExportMode::Snapshot)
            .expect("a snapshot of a loaded document")
    }

    pub fn asset(&self, id: AssetId) -> Option<&Arc<[u8]>> {
        self.assets.get(&id)
    }

    pub fn asset_ids(&self) -> impl Iterator<Item = AssetId> + '_ {
        self.assets.keys().copied()
    }

    /// Whether the board or its assets changed since the last call: time to store it.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn members(&self) -> usize {
        self.members.len()
    }

    /// Lets `client` in with `role`. `version` and `assets` say what it has; it is sent the rest and
    /// where the others are.
    pub fn join(
        &mut self,
        client: ClientId,
        role: Role,
        version: &[u8],
        assets: &[AssetId],
    ) -> Result<Outgoing, String> {
        let from = if version.is_empty() {
            VersionVector::default()
        } else {
            VersionVector::decode(version).map_err(|e| e.to_string())?
        };
        let updates = self
            .doc
            .export(ExportMode::updates(&from))
            .map_err(|e| e.to_string())?;
        let mut out = vec![(
            client,
            Message::Welcome {
                client,
                role,
                updates,
                version: self.doc.oplog_vv().encode(),
                assets: self.assets.keys().copied().collect(),
            },
        )];
        for (id, bytes) in &self.assets {
            if !assets.contains(id) {
                out.push((client, Message::Asset(bytes.to_vec())));
            }
        }
        for member in self.members.values() {
            if let Some(presence) = &member.presence {
                out.push((client, Message::Presence(presence.clone())));
            }
        }
        self.members.insert(
            client,
            Member {
                role,
                peers: HashSet::new(),
                presence: None,
            },
        );
        Ok(out)
    }

    /// Takes in a message from `client`. An error is for that client: the connection
    /// should be closed after telling it.
    pub fn receive(&mut self, client: ClientId, message: Message) -> Result<Outgoing, String> {
        let member = self.members.get_mut(&client).ok_or("not in the room")?;
        match message {
            Message::Updates(bytes) => {
                if member.role != Role::Edit {
                    return Err("this board is read-only for you".into());
                }
                let before = self.doc.oplog_vv();
                self.doc.import(&bytes).map_err(|e| e.to_string())?;
                if self.doc.oplog_vv() == before {
                    return Ok(Vec::new());
                }
                self.changed = true;
                Ok(self.to_others(client, &Message::Updates(bytes)))
            }
            Message::Remarks(json) => {
                if member.role == Role::View {
                    return Err("this board is read-only for you".into());
                }
                if json.len() > MAX_REMARKS {
                    return Err("too much at once".into());
                }
                let schema = loro::JsonSchema::try_from(json.as_str())
                    .map_err(|e| format!("unreadable remarks: {e}"))?;
                let before = self.doc.oplog_vv();
                let peers = check_remarks(&schema, &member.peers, &before)?;
                self.doc
                    .import_json_updates(schema)
                    .map_err(|e| e.to_string())?;
                member.peers.extend(peers);
                if self.doc.oplog_vv() == before {
                    return Ok(Vec::new());
                }
                self.changed = true;
                let updates = self
                    .doc
                    .export(ExportMode::updates(&before))
                    .map_err(|e| e.to_string())?;
                Ok(self.to_others(client, &Message::Updates(updates)))
            }
            Message::Asset(bytes) => {
                if member.role != Role::Edit {
                    return Err("this board is read-only for you".into());
                }
                if bytes.len() > MAX_ASSET {
                    return Err("the file is too large".into());
                }
                let total: usize = self.assets.values().map(|a| a.len()).sum();
                if total + bytes.len() > MAX_ASSETS_TOTAL {
                    return Err("the board has no room for more pictures and files".into());
                }
                let id = AssetId::of(&bytes);
                if self.assets.contains_key(&id) {
                    return Ok(Vec::new());
                }
                let message = Message::Asset(bytes.clone());
                self.assets.insert(id, bytes.into());
                self.changed = true;
                Ok(self.to_others(client, &message))
            }
            Message::Presence(mut presence) => {
                presence.client = client;
                member.presence = Some(presence.clone());
                Ok(self.to_others(client, &Message::Presence(presence)))
            }
            _ => Err("unexpected message".into()),
        }
    }

    /// `client` is gone; the others are told.
    pub fn leave(&mut self, client: ClientId) -> Outgoing {
        if self.members.remove(&client).is_none() {
            return Vec::new();
        }
        self.to_others(client, &Message::Left(client))
    }

    fn to_others(&self, client: ClientId, message: &Message) -> Outgoing {
        self.members
            .keys()
            .filter(|&&id| id != client)
            .map(|&id| (id, message.clone()))
            .collect()
    }
}

/// Checks that remarks only write what someone who may only comment may
/// ([`pb_core::commenting_writes`]), as plain values, and only as peers that are new to the
/// board or this member's own (`owned`), so they cannot pass for someone else's edits.
/// Returns the peers they use.
fn check_remarks(
    schema: &loro::JsonSchema,
    owned: &HashSet<u64>,
    have: &VersionVector,
) -> Result<HashSet<u64>, String> {
    if schema.peers.is_some() {
        return Err("remarks must name their peers in full".into());
    }
    let mut peers = HashSet::new();
    for change in &schema.changes {
        let peer = change.id.peer;
        if !owned.contains(&peer) && have.get(&peer).is_some_and(|&n| n > 0) {
            return Err("remarks may not continue someone else's changes".into());
        }
        peers.insert(peer);
        for op in &change.ops {
            let loro::ContainerID::Root {
                name,
                container_type: loro::ContainerType::Map,
            } = &op.container
            else {
                return Err("you may only comment on this board".into());
            };
            let allowed = match &op.content {
                loro::JsonOpContent::Map(loro::JsonMapOp::Insert { key, value }) => {
                    !matches!(value, loro::LoroValue::Container(_))
                        && pb_core::commenting_writes(name, key)
                }
                loro::JsonOpContent::Map(loro::JsonMapOp::Delete { key }) => {
                    pb_core::commenting_writes(name, key)
                }
                _ => false,
            };
            if !allowed {
                return Err("you may only comment on this board".into());
            }
        }
    }
    Ok(peers)
}
