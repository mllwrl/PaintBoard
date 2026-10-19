//! The server end to end: a real listener, HTTP and WebSockets, boards on both ends.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use kurbo::Rect;
use pb_core::{Command, Document, ObjectData, ShapeKind, Style};
use pb_sync::{Client, Event, Message};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite;

use crate::server::{Shared, router};
use crate::store::Store;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

async fn start(create_key: Option<&str>) -> (String, std::sync::Arc<Shared>) {
    let shared = Shared::new(Store::in_memory().unwrap(), create_key.map(Into::into));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = router(shared.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, shared)
}

/// `POST /boards`: the status line and body.
async fn make_board(addr: &str, key: Option<&str>) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let key = key.map_or(String::new(), |k| format!("x-create-key: {k}\r\n"));
    let request = format!(
        "POST /boards HTTP/1.1\r\nhost: {addr}\r\n{key}content-length: 0\r\nconnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    (head.lines().next().unwrap().to_owned(), body.to_owned())
}

/// The value of `"key":"value"` in a flat JSON object.
fn field(json: &str, key: &str) -> String {
    let start = json.find(&format!("\"{key}\":\"")).unwrap() + key.len() + 4;
    json[start..].split('"').next().unwrap().to_owned()
}

/// A board connected to the server.
struct Peer {
    doc: Document,
    client: Client,
    socket: Socket,
    events: Vec<Event>,
}

impl Peer {
    async fn join(addr: &str, board: &str, token: &str, doc: Document) -> Self {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .unwrap();
        let mut peer = Self {
            doc,
            client: Client::new(board, token, "tester"),
            socket,
            events: Vec::new(),
        };
        let hello = peer.client.hello(&mut peer.doc);
        peer.send(&hello).await;
        peer
    }

    async fn send(&mut self, message: &Message) {
        let frame = tungstenite::Message::Binary(message.encode().into());
        self.socket.send(frame).await.unwrap();
    }

    /// Sends local edits, then handles what arrives within a short while.
    async fn pump(&mut self) {
        for message in self.client.outgoing(&mut self.doc) {
            self.send(&message).await;
        }
        loop {
            let next = tokio::time::timeout(Duration::from_millis(150), self.socket.next()).await;
            let Ok(Some(Ok(tungstenite::Message::Binary(bytes)))) = next else {
                break;
            };
            let mut replies = Vec::new();
            let message = Message::decode(&bytes).unwrap();
            let events = self.client.receive(message, &mut self.doc, &mut replies);
            self.events.extend(events);
            for reply in replies {
                self.send(&reply).await;
            }
        }
    }

    fn add_rect(&mut self, x: f64) {
        let data = ObjectData {
            kind: ShapeKind::Rectangle.into(),
            bounds: Rect::new(x, 0.0, x + 10.0, 10.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
            link: None,
        };
        self.doc.apply(Command::Create(vec![data.into()])).unwrap();
    }

    fn count(&self) -> usize {
        self.doc.objects().len()
    }
}

fn doc(peer: u64) -> Document {
    let mut doc = Document::new();
    doc.set_peer(peer).unwrap();
    doc
}

#[tokio::test]
async fn boards_are_shared_and_kept() {
    let (addr, shared) = start(None).await;
    let (status, body) = make_board(&addr, None).await;
    assert!(status.contains("200"), "{status}");
    let (board, edit, view) = (
        field(&body, "board"),
        field(&body, "edit"),
        field(&body, "view"),
    );

    let mut a = Peer::join(&addr, &board, &edit, doc(1)).await;
    a.add_rect(0.0);
    a.doc.add_asset(b"a picture".to_vec());
    a.pump().await;
    let mut b = Peer::join(&addr, &board, &edit, doc(2)).await;
    b.pump().await;
    assert_eq!(b.count(), 1);
    assert_eq!(b.doc.assets().len(), 1);
    b.add_rect(50.0);
    b.pump().await;
    a.pump().await;
    assert_eq!(a.count(), 2);

    // Lookers see it but cannot change it.
    let mut v = Peer::join(&addr, &board, &view, doc(3)).await;
    v.pump().await;
    assert_eq!(v.count(), 2);
    assert_eq!(v.client.joined(), Some(false));

    // Everyone leaves: the board is written and unloaded, then loaded for the next one.
    drop((a, b, v));
    tokio::time::sleep(Duration::from_millis(300)).await;
    shared.save_all();
    let mut c = Peer::join(&addr, &board, &view, doc(4)).await;
    c.pump().await;
    assert_eq!(c.count(), 2);
    assert_eq!(c.doc.assets().len(), 1);
}

#[tokio::test]
async fn wrong_links_and_keys_are_refused() {
    let (addr, _) = start(Some("secret")).await;
    let (status, _) = make_board(&addr, None).await;
    assert!(status.contains("403"), "{status}");
    let (status, body) = make_board(&addr, Some("secret")).await;
    assert!(status.contains("200"), "{status}");
    let board = field(&body, "board");

    let mut intruder = Peer::join(&addr, &board, "guessed", doc(1)).await;
    intruder.pump().await;
    assert!(
        intruder
            .events
            .iter()
            .any(|e| matches!(e, Event::Error(text) if text.contains("link")))
    );
    assert_eq!(intruder.client.joined(), None);
}

/// The desktop app's network code (threads, tungstenite) against the real server.
#[tokio::test(flavor = "multi_thread")]
async fn the_apps_socket_and_requests_work() {
    use pb_platform::net::{Socket, SocketState, post};

    let (addr, _) = start(None).await;
    let (made, answer) = std::sync::mpsc::channel();
    post(&format!("http://{addr}/boards"), None, move |result| {
        let _ = made.send(result);
    });
    let body = tokio::task::spawn_blocking(move || answer.recv().unwrap())
        .await
        .unwrap()
        .unwrap();
    let (board, edit) = (field(&body, "board"), field(&body, "edit"));

    let socket = Socket::connect(&format!("ws://{addr}/ws"), || {});
    let mut doc = doc(1);
    let mut client = Client::new(board, edit, "app");
    socket.send(client.hello(&mut doc).encode());
    let mut joined = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        while let Some(bytes) = socket.try_recv() {
            let events =
                client.receive(Message::decode(&bytes).unwrap(), &mut doc, &mut Vec::new());
            joined |= events
                .iter()
                .any(|e| matches!(e, Event::Joined { can_edit: true, .. }));
        }
        if joined {
            break;
        }
    }
    assert!(joined);
    assert_eq!(socket.state(), SocketState::Open);
}
