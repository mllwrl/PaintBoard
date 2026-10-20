//! Sharing a board through a server: the messages ([`Message`]), a board's side
//! ([`Client`]), the server's side of one board ([`Room`]) and the wire ([`Transport`]).
//!
//! The server does not understand boards: clients merge each other's changes themselves
//! (the board is a CRDT), so the room only stores and passes on CRDT updates, pictures
//! and where everyone is.

mod client;
mod message;
mod room;
mod transport;

pub use client::{Client, Event};
pub use message::{ClientId, DecodeError, Message, PROTOCOL, Presence};
pub use room::{MAX_ASSET, MAX_ASSETS_TOTAL, MAX_REMARKS, Outgoing, Room};
pub use transport::{LocalOnly, Memory, Status, Transport};

#[cfg(test)]
mod fuzz_tests;
#[cfg(test)]
mod tests;
