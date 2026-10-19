//! The board shared through a server: links, the connection (kept up, reconnecting when
//! it drops) and where the others are. The protocol itself is `pb_sync`'s.

use std::collections::BTreeMap;
use std::time::Duration;

use pb_core::{Document, ObjectId};
use pb_geom::kurbo::Point;
use pb_platform::Instant;
use pb_platform::net::{MaybeSend, Socket, SocketState};
use pb_sync::{Client, ClientId, Event, Message, Presence};

/// A link to a shared board: `https://host/board/<id>?key=<token>`. The key says what
/// its holder may do, so a link for looking only cannot be turned into one for editing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// `http(s)://host[:port]`, without a trailing slash.
    pub server: String,
    pub board: String,
    pub token: String,
}

impl Link {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (server, rest) = text.split_once("/board/")?;
        let (board, query) = rest.split_once('?')?;
        let token = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("key="))?
            .trim_end_matches('/');
        let server = server.trim_end_matches('/');
        let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
        if !(server.starts_with("http://") || server.starts_with("https://"))
            || !valid(board)
            || !valid(token)
        {
            return None;
        }
        Some(Self {
            server: server.to_owned(),
            board: board.to_owned(),
            token: token.to_owned(),
        })
    }

    /// The server's WebSocket address.
    fn socket_url(&self) -> String {
        let rest = self
            .server
            .strip_prefix("https://")
            .map(|r| format!("wss://{r}"))
            .or_else(|| {
                self.server
                    .strip_prefix("http://")
                    .map(|r| format!("ws://{r}"))
            })
            .unwrap_or_else(|| self.server.clone());
        format!("{rest}/ws")
    }
}

impl std::fmt::Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/board/{}?key={}", self.server, self.board, self.token)
    }
}

/// The server's address as typed: with `https://` added when no scheme is given, and no
/// trailing slash. `None` if it is not an address.
pub fn server_address(typed: &str) -> Option<String> {
    let typed = typed.trim().trim_end_matches('/');
    if typed.is_empty() || typed.contains(char::is_whitespace) {
        return None;
    }
    if typed.starts_with("http://") || typed.starts_with("https://") {
        Some(typed.to_owned())
    } else {
        Some(format!("https://{typed}"))
    }
}

/// `scheme://host[:port]` of a web address, if it is one (`http` or `https`).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn origin(address: &str) -> Option<String> {
    let (scheme, rest) = address.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|h| !h.is_empty())?;
    Some(format!("{scheme}://{host}"))
}

/// The links of a board just made on `server`, from the server's answer
/// (`{"board":…,"edit":…,"view":…}`): to edit, and to look only.
pub fn made_links(server: &str, answer: &str) -> Option<(Link, Link)> {
    let field = |key: &str| {
        let start = answer.find(&format!("\"{key}\":\""))? + key.len() + 4;
        answer[start..].split('"').next().map(str::to_owned)
    };
    let board = field("board")?;
    let link = |token| Link {
        server: server.to_owned(),
        board: board.clone(),
        token,
    };
    Some((link(field("edit")?), link(field("view")?)))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    /// Joined; with the right to edit or not.
    Online {
        can_edit: bool,
    },
    /// Not connected, and why; it tries again unless the server refused for good.
    Offline(String),
}

/// What the app should do after [`Sharing::tick`].
#[derive(Default)]
pub struct Ticked {
    /// The board changed: redraw.
    pub changed: bool,
    /// Something to tell the user.
    pub error: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
type Waker = std::sync::Arc<dyn Fn() + Send + Sync>;
#[cfg(target_arch = "wasm32")]
type Waker = std::rc::Rc<dyn Fn()>;

/// How long the first retry waits; each next waits twice as long, up to [`MAX_BACKOFF`].
const BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Pointer moves are sent at most this often.
const PRESENCE_EVERY: Duration = Duration::from_millis(50);

/// Colors people's pointers get, by their client id.
const COLORS: [u32; 8] = [
    0xE5484D, 0x3B82F6, 0x16A34A, 0xF59E0B, 0x8B5CF6, 0xEC4899, 0x06B6D4, 0x84CC16,
];

pub fn color_of(client: ClientId) -> u32 {
    COLORS[client as usize % COLORS.len()]
}

pub struct Sharing {
    /// The link this copy joined with.
    pub link: Link,
    /// The link for looking only, when this copy made the board.
    pub view_link: Option<Link>,
    pub status: Status,
    /// Everyone else on the board, by client.
    pub people: BTreeMap<ClientId, Presence>,
    client: Client,
    socket: Option<Socket>,
    hello_sent: bool,
    /// When to connect again, while offline.
    retry_at: Option<Instant>,
    backoff: Duration,
    /// This copy's pointer and selection, and whether the server has them.
    mine: Presence,
    mine_sent: bool,
    sent_at: Option<Instant>,
    wake: Waker,
}

impl Sharing {
    /// Starts sharing `doc` through `link`'s server; `wake` is called when something
    /// arrives.
    pub fn start(
        link: Link,
        view_link: Option<Link>,
        name: &str,
        wake: impl Fn() + MaybeSend + 'static,
    ) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let wake: Waker = std::sync::Arc::new(wake);
        #[cfg(target_arch = "wasm32")]
        let wake: Waker = std::rc::Rc::new(wake);
        let mut sharing = Self {
            client: Client::new(link.board.clone(), link.token.clone(), name),
            link,
            view_link,
            status: Status::Connecting,
            people: BTreeMap::new(),
            socket: None,
            hello_sent: false,
            retry_at: None,
            backoff: BACKOFF,
            mine: Presence {
                name: name.to_owned(),
                ..Presence::default()
            },
            mine_sent: false,
            sent_at: None,
            wake,
        };
        sharing.connect();
        sharing
    }

    fn connect(&mut self) {
        let wake = self.wake.clone();
        self.socket = Some(Socket::connect(&self.link.socket_url(), move || wake()));
        self.hello_sent = false;
        self.retry_at = None;
        self.status = Status::Connecting;
    }

    /// When [`Sharing::tick`] should run next without anything arriving.
    pub fn deadline(&self) -> Option<Instant> {
        let presence = (!self.mine_sent)
            .then(|| self.sent_at.map(|at| at + PRESENCE_EVERY))
            .flatten();
        [self.retry_at, presence].into_iter().flatten().min()
    }

    /// Sends and takes in what there is; connects again when due.
    pub fn tick(&mut self, doc: &mut Document, now: Instant) -> Ticked {
        let mut ticked = Ticked::default();
        if self.socket.is_none() && self.retry_at.is_some_and(|at| at <= now) {
            self.connect();
        }
        let Some(socket) = &self.socket else {
            return ticked;
        };
        match socket.state() {
            SocketState::Connecting => return ticked,
            SocketState::Closed(reason) => {
                self.went_offline(reason, now);
                return ticked;
            }
            SocketState::Open => {}
        }
        if !self.hello_sent {
            socket.send(self.client.hello(doc).encode());
            self.hello_sent = true;
            self.mine_sent = false;
        }
        let mut replies = Vec::new();
        while let Some(bytes) = socket.try_recv() {
            let message = match Message::decode(&bytes) {
                Ok(message) => message,
                Err(err) => {
                    log::warn!("from the server: {err}");
                    continue;
                }
            };
            for event in self.client.receive(message, doc, &mut replies) {
                match event {
                    Event::Joined { can_edit, .. } => {
                        self.status = Status::Online { can_edit };
                        self.backoff = BACKOFF;
                        doc.set_view_only(!can_edit);
                        ticked.changed = true;
                    }
                    Event::Changed => ticked.changed = true,
                    Event::Presence(presence) => {
                        self.people.insert(presence.client, presence);
                        ticked.changed = true;
                    }
                    Event::Left(client) => {
                        self.people.remove(&client);
                        ticked.changed = true;
                    }
                    Event::Error(text) => {
                        log::warn!("server: {text}");
                        ticked.error = Some(text.clone());
                        if self.client.joined().is_none() {
                            // Refused (no such board, wrong link): trying again won't help.
                            self.stop(text);
                            return ticked;
                        }
                    }
                }
            }
        }
        let outgoing = self.client.outgoing(doc);
        for message in replies.iter().chain(&outgoing) {
            socket.send(message.encode());
        }
        let due = self.sent_at.is_none_or(|at| now >= at + PRESENCE_EVERY);
        if !self.mine_sent && due && self.client.joined().is_some() {
            socket.send(Message::Presence(self.mine.clone()).encode());
            self.mine_sent = true;
            self.sent_at = Some(now);
        }
        ticked
    }

    fn went_offline(&mut self, reason: String, now: Instant) {
        self.socket = None;
        self.client.disconnected();
        self.people.clear();
        self.status = Status::Offline(reason);
        self.retry_at = Some(now + self.backoff);
        self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
    }

    /// Gives up: the server refused.
    fn stop(&mut self, reason: String) {
        self.socket = None;
        self.client.disconnected();
        self.people.clear();
        self.status = Status::Offline(reason);
        self.retry_at = None;
    }

    /// Shows this copy under `name` from now on.
    pub fn set_name(&mut self, name: &str) {
        self.mine.name = name.to_owned();
        self.mine_sent = false;
        (self.wake)();
    }

    /// Connects again now (the user asked).
    pub fn retry(&mut self) {
        if self.socket.is_none() {
            self.backoff = BACKOFF;
            self.connect();
        }
    }

    /// Tells the others where this copy's pointer is (on the board, world units) and
    /// what it has selected; sent with the next tick when it changed.
    pub fn set_presence(&mut self, cursor: Option<Point>, selection: &[ObjectId], view: [f64; 4]) {
        let cursor = cursor.map(|p| (p.x, p.y));
        let selection: Vec<String> = selection.iter().map(ToString::to_string).collect();
        // Those following see the view move, so it is sent when it moves noticeably.
        let moved = self.mine.view.is_none_or(|old| {
            let size = (view[2] - view[0])
                .abs()
                .max((view[3] - view[1]).abs())
                .max(1.0);
            old.iter()
                .zip(&view)
                .any(|(a, b)| (a - b).abs() > size * 0.002)
        });
        if self.mine.cursor != cursor || self.mine.selection != selection || moved {
            self.mine.cursor = cursor;
            self.mine.selection = selection;
            self.mine.view = Some(view);
            self.mine_sent = false;
            (self.wake)();
        }
    }

    /// Whether this copy asks everyone to follow its view.
    pub fn leading(&self) -> bool {
        self.mine.leading
    }

    pub fn set_leading(&mut self, leading: bool) {
        if self.mine.leading != leading {
            self.mine.leading = leading;
            self.mine_sent = false;
            (self.wake)();
        }
    }

    /// Someone else asking everyone to follow them, if anyone is.
    pub fn leader(&self) -> Option<ClientId> {
        self.people
            .iter()
            .find(|(_, p)| p.leading && p.view.is_some())
            .map(|(client, _)| *client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_round_trip() {
        let link = Link {
            server: "https://boards.example.com".into(),
            board: "a1b2".into(),
            token: "ff00".into(),
        };
        let text = link.to_string();
        assert_eq!(text, "https://boards.example.com/board/a1b2?key=ff00");
        assert_eq!(Link::parse(&text), Some(link.clone()));
        assert_eq!(Link::parse(&format!("  {text}\n")), Some(link.clone()));
        assert_eq!(link.socket_url(), "wss://boards.example.com/ws");
        let local = Link::parse("http://localhost:8080/board/x?key=y").unwrap();
        assert_eq!(local.socket_url(), "ws://localhost:8080/ws");
        assert_eq!(Link::parse("https://example.com/page"), None);
        assert_eq!(Link::parse("ftp://h/board/x?key=y"), None);
        assert_eq!(Link::parse("https://h/board/x?key=y z"), None);
    }

    #[test]
    fn typed_server_addresses() {
        assert_eq!(
            server_address("boards.example.com/").as_deref(),
            Some("https://boards.example.com")
        );
        assert_eq!(
            server_address("http://localhost:8080").as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(server_address("  "), None);
    }

    #[test]
    fn origins_of_addresses() {
        assert_eq!(
            origin("http://127.0.0.1:8080/board/x?key=y").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(origin("https://h").as_deref(), Some("https://h"));
        assert_eq!(origin("file:///c/x.html"), None);
    }

    #[test]
    fn links_from_the_server_answer() {
        let (edit, view) =
            made_links("http://h", r#"{"board":"b1","edit":"e2","view":"v3"}"#).unwrap();
        assert_eq!(edit.to_string(), "http://h/board/b1?key=e2");
        assert_eq!(view.to_string(), "http://h/board/b1?key=v3");
        assert!(made_links("http://h", "forbidden").is_none());
    }
}
