//! A board's side of the conversation with the server: no networking, only what to send
//! and what to do with what arrives (see [`crate::Transport`] for the wire).

use std::collections::HashSet;

use pb_core::{AssetId, Document};

use crate::message::{ClientId, Message, PROTOCOL, Presence};

/// Something the app should know about after [`Client::receive`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The server took the client in.
    Joined {
        client: ClientId,
        can_edit: bool,
    },
    /// The board changed: redraw (and save).
    Changed,
    /// Someone's pointer, selection or view.
    Presence(Presence),
    Left(ClientId),
    /// The server's complaint.
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// `Hello` sent, no `Welcome` yet.
    Joining,
    Joined {
        can_edit: bool,
    },
}

/// One board shared through a server.
pub struct Client {
    board: String,
    token: String,
    name: String,
    state: Option<State>,
    /// Pictures and files the server has, or was sent.
    server_assets: HashSet<AssetId>,
}

impl Client {
    pub fn new(
        board: impl Into<String>,
        token: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            board: board.into(),
            token: token.into(),
            name: name.into(),
            state: None,
            server_assets: HashSet::new(),
        }
    }

    pub fn board(&self) -> &str {
        &self.board
    }

    /// Joined, and with what right.
    pub fn joined(&self) -> Option<bool> {
        match self.state {
            Some(State::Joined { can_edit }) => Some(can_edit),
            _ => None,
        }
    }

    /// The first message on each new connection (also after reconnecting).
    pub fn hello(&mut self, doc: &mut Document) -> Message {
        doc.start_sharing();
        self.state = Some(State::Joining);
        Message::Hello {
            protocol: PROTOCOL,
            board: self.board.clone(),
            token: self.token.clone(),
            name: self.name.clone(),
            version: doc.sync_version(),
            assets: doc.assets().ids().collect(),
        }
    }

    /// The connection is gone; [`Client::hello`] starts over on the next one.
    pub fn disconnected(&mut self) {
        self.state = None;
    }

    /// Takes in a message from the server; replies go to `out`.
    pub fn receive(
        &mut self,
        message: Message,
        doc: &mut Document,
        out: &mut Vec<Message>,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        match message {
            Message::Welcome {
                client,
                can_edit,
                updates,
                version,
                assets,
            } => {
                self.state = Some(State::Joined { can_edit });
                events.push(Event::Joined { client, can_edit });
                match doc.import_updates(&updates) {
                    Ok(true) => events.push(Event::Changed),
                    Ok(false) => {}
                    Err(err) => events.push(Event::Error(err.to_string())),
                }
                // What the server lacks goes in one piece; edits queued while joining
                // are part of it.
                doc.take_local_updates();
                self.server_assets = assets.into_iter().collect();
                if can_edit {
                    match doc.updates_since(&version) {
                        Ok(missing) if !missing.is_empty() => out.push(Message::Updates(missing)),
                        Ok(_) => {}
                        Err(err) => events.push(Event::Error(err.to_string())),
                    }
                    self.send_assets(doc, out);
                }
            }
            Message::Updates(updates) => match doc.import_updates(&updates) {
                Ok(true) => events.push(Event::Changed),
                Ok(false) => {}
                Err(err) => events.push(Event::Error(err.to_string())),
            },
            Message::Asset(bytes) => {
                let id = doc.add_asset(bytes);
                self.server_assets.insert(id);
                events.push(Event::Changed);
            }
            Message::Presence(presence) => events.push(Event::Presence(presence)),
            Message::Left(client) => events.push(Event::Left(client)),
            Message::Error(text) => events.push(Event::Error(text)),
            Message::Hello { .. } => {
                events.push(Event::Error("the server sent a client's greeting".into()));
            }
        }
        events
    }

    /// Local edits and new pictures to send, once joined with the right to edit.
    pub fn outgoing(&mut self, doc: &mut Document) -> Vec<Message> {
        let mut out = Vec::new();
        match self.state {
            Some(State::Joined { can_edit: true }) => {
                out.extend(doc.take_local_updates().into_iter().map(Message::Updates));
                self.send_assets(doc, &mut out);
            }
            Some(State::Joined { can_edit: false }) => {
                // Looking only: local changes (none, normally) stay local.
                doc.take_local_updates();
            }
            // Kept until `Welcome`, which sends them in one piece.
            Some(State::Joining) | None => {}
        }
        out
    }

    fn send_assets(&mut self, doc: &Document, out: &mut Vec<Message>) {
        let assets = doc.assets();
        for id in assets.ids().collect::<Vec<_>>() {
            if self.server_assets.insert(id)
                && let Some(bytes) = assets.get(id)
            {
                out.push(Message::Asset(bytes.to_vec()));
            }
        }
    }
}
