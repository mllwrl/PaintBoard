//! What clients and the server say to each other, and its binary form.
//!
//! Each message is one WebSocket binary frame: a tag byte, then its fields in order.
//! Numbers are little-endian; byte strings and text carry a `u32` length first.

use pb_core::AssetId;

/// Bumped when messages change incompatibly; the server refuses other versions.
pub const PROTOCOL: u32 = 1;

/// One copy of the board on the server, for as long as it is connected.
pub type ClientId = u64;

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// Client → server, first: which board, with what right, and what the client has.
    Hello {
        protocol: u32,
        board: String,
        /// Says what the client may do (see the server's access rules).
        token: String,
        /// Shown to the others next to the cursor.
        name: String,
        /// The client's version vector (`Document::sync_version`).
        version: Vec<u8>,
        /// Pictures and files the client has.
        assets: Vec<AssetId>,
    },
    /// Server → client, the answer to `Hello`.
    Welcome {
        client: ClientId,
        /// Whether the client may change the board.
        can_edit: bool,
        /// Changes the client lacks.
        updates: Vec<u8>,
        /// The server's version vector: the client sends what the server lacks.
        version: Vec<u8>,
        /// Pictures and files the server has.
        assets: Vec<AssetId>,
    },
    /// Changes to the board (Loro updates), either way.
    Updates(Vec<u8>),
    /// A picture or file's bytes, either way; its id is their hash.
    Asset(Vec<u8>),
    /// Where someone is and what they have selected; never stored.
    Presence(Presence),
    /// Server → clients: someone left.
    Left(ClientId),
    /// Server → client: something went wrong; the server closes the connection after it
    /// unless it is only a warning.
    Error(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presence {
    /// Set by the server: clients send 0.
    pub client: ClientId,
    pub name: String,
    /// 0xRRGGBB.
    pub color: u32,
    /// The pointer on the board (world units), if it is over it.
    pub cursor: Option<(f64, f64)>,
    /// Selected objects, as `ObjectId`'s text form.
    pub selection: Vec<String>,
    /// What part of the board they look at: left, top, right, bottom (world units).
    pub view: Option<[f64; 4]>,
}

/// Why bytes are not a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError(pub &'static str);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad message: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

mod tag {
    pub const HELLO: u8 = 1;
    pub const WELCOME: u8 = 2;
    pub const UPDATES: u8 = 3;
    pub const ASSET: u8 = 4;
    pub const PRESENCE: u8 = 5;
    pub const LEFT: u8 = 6;
    pub const ERROR: u8 = 7;
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Message::Hello {
                protocol,
                board,
                token,
                name,
                version,
                assets,
            } => {
                w.u8(tag::HELLO);
                w.u32(*protocol);
                w.text(board);
                w.text(token);
                w.text(name);
                w.bytes(version);
                w.assets(assets);
            }
            Message::Welcome {
                client,
                can_edit,
                updates,
                version,
                assets,
            } => {
                w.u8(tag::WELCOME);
                w.u64(*client);
                w.u8(u8::from(*can_edit));
                w.bytes(updates);
                w.bytes(version);
                w.assets(assets);
            }
            Message::Updates(bytes) => {
                w.u8(tag::UPDATES);
                w.bytes(bytes);
            }
            Message::Asset(bytes) => {
                w.u8(tag::ASSET);
                w.bytes(bytes);
            }
            Message::Presence(p) => {
                w.u8(tag::PRESENCE);
                w.u64(p.client);
                w.text(&p.name);
                w.u32(p.color);
                match p.cursor {
                    Some((x, y)) => {
                        w.u8(1);
                        w.f64(x);
                        w.f64(y);
                    }
                    None => w.u8(0),
                }
                w.u32(p.selection.len() as u32);
                for id in &p.selection {
                    w.text(id);
                }
                match p.view {
                    Some(view) => {
                        w.u8(1);
                        view.iter().for_each(|v| w.f64(*v));
                    }
                    None => w.u8(0),
                }
            }
            Message::Left(client) => {
                w.u8(tag::LEFT);
                w.u64(*client);
            }
            Message::Error(text) => {
                w.u8(tag::ERROR);
                w.text(text);
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(bytes);
        let message = match r.u8()? {
            tag::HELLO => Message::Hello {
                protocol: r.u32()?,
                board: r.text()?,
                token: r.text()?,
                name: r.text()?,
                version: r.bytes()?,
                assets: r.assets()?,
            },
            tag::WELCOME => Message::Welcome {
                client: r.u64()?,
                can_edit: r.u8()? != 0,
                updates: r.bytes()?,
                version: r.bytes()?,
                assets: r.assets()?,
            },
            tag::UPDATES => Message::Updates(r.bytes()?),
            tag::ASSET => Message::Asset(r.bytes()?),
            tag::PRESENCE => {
                let client = r.u64()?;
                let name = r.text()?;
                let color = r.u32()?;
                let cursor = match r.u8()? {
                    0 => None,
                    _ => Some((r.f64()?, r.f64()?)),
                };
                let count = r.count(4)?;
                let selection = (0..count).map(|_| r.text()).collect::<Result<_, _>>()?;
                let view = match r.u8()? {
                    0 => None,
                    _ => Some([r.f64()?, r.f64()?, r.f64()?, r.f64()?]),
                };
                Message::Presence(Presence {
                    client,
                    name,
                    color,
                    cursor,
                    selection,
                    view,
                })
            }
            tag::LEFT => Message::Left(r.u64()?),
            tag::ERROR => Message::Error(r.text()?),
            _ => return Err(DecodeError("unknown kind")),
        };
        if !r.0.is_empty() {
            return Err(DecodeError("bytes left over"));
        }
        Ok(message)
    }
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
    }
    fn text(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
    fn assets(&mut self, ids: &[AssetId]) {
        self.u32(ids.len() as u32);
        for id in ids {
            self.0.extend_from_slice(id.as_bytes());
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], DecodeError> {
        if self.0.len() < n {
            return Err(DecodeError("cut short"));
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
    fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
    /// A count of items at least `size` bytes each, checked against what is left, so a
    /// bad count cannot make the reader allocate much.
    fn count(&mut self, size: usize) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(size) > self.0.len() {
            return Err(DecodeError("count too large"));
        }
        Ok(n)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n = self.count(1)?;
        Ok(self.take(n)?.to_vec())
    }
    fn text(&mut self) -> Result<String, DecodeError> {
        String::from_utf8(self.bytes()?).map_err(|_| DecodeError("text is not UTF-8"))
    }
    fn assets(&mut self) -> Result<Vec<AssetId>, DecodeError> {
        let n = self.count(32)?;
        (0..n)
            .map(|_| {
                let bytes: [u8; 32] = self.take(32)?.try_into().expect("32 bytes");
                Ok(AssetId::from_bytes(bytes))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(message: Message) {
        let bytes = message.encode();
        assert_eq!(Message::decode(&bytes), Ok(message));
        // Every shorter prefix is refused, never misread.
        for end in 0..bytes.len() {
            assert!(Message::decode(&bytes[..end]).is_err());
        }
    }

    #[test]
    fn messages_survive_encoding() {
        let asset = AssetId::of(b"picture");
        round_trip(Message::Hello {
            protocol: PROTOCOL,
            board: "b1".into(),
            token: "секрет".into(),
            name: "Ann".into(),
            version: vec![1, 2, 3],
            assets: vec![asset],
        });
        round_trip(Message::Welcome {
            client: 7,
            can_edit: true,
            updates: vec![9; 100],
            version: vec![],
            assets: vec![asset, asset],
        });
        round_trip(Message::Updates(vec![0, 255]));
        round_trip(Message::Asset(b"png".to_vec()));
        round_trip(Message::Presence(Presence {
            client: 3,
            name: "Bob".into(),
            color: 0x3B82F6,
            cursor: Some((1.5, -2.0)),
            selection: vec!["1@2".into(), "3@4".into()],
            view: Some([0.0, 0.0, 800.0, 600.0]),
        }));
        round_trip(Message::Presence(Presence::default()));
        round_trip(Message::Left(42));
        round_trip(Message::Error("no such board".into()));
    }

    #[test]
    fn garbage_is_refused() {
        assert!(Message::decode(&[]).is_err());
        assert!(Message::decode(&[99]).is_err());
        // A huge length with nothing behind it.
        assert!(Message::decode(&[tag::UPDATES, 255, 255, 255, 255]).is_err());
        let mut extra = Message::Left(1).encode();
        extra.push(0);
        assert!(Message::decode(&extra).is_err());
    }
}
