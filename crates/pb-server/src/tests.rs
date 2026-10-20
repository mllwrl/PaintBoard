//! The server end to end: a real listener, HTTP and WebSockets, boards on both ends.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use kurbo::Rect;
use pb_core::{Command, Document, ObjectData, Role, ShapeKind, Style};
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
    let app = router(shared.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>();
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
        Self::join_as(addr, board, token, "", doc).await
    }

    /// Joins with the account of `session` too.
    async fn join_as(addr: &str, board: &str, token: &str, session: &str, doc: Document) -> Self {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .unwrap();
        let mut client = Client::new(board, token, "tester");
        client.set_session(session);
        let mut peer = Self {
            doc,
            client,
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
    assert_eq!(v.client.joined(), Some(Role::View));

    // Commenters comment; the editors see it.
    let mut c = Peer::join(&addr, &board, &field(&body, "comment"), doc(5)).await;
    c.pump().await;
    assert_eq!(c.client.joined(), Some(Role::Comment));
    c.doc
        .apply(Command::AddComment {
            thread: "t".into(),
            anchor: Some(pb_core::ThreadAnchor::Point(kurbo::Point::ZERO)),
            comment: pb_core::Comment {
                id: "1".into(),
                author: "cat".into(),
                at_ms: 1,
                text: "nice".into(),
            },
        })
        .unwrap();
    c.pump().await;
    a.pump().await;
    assert_eq!(a.doc.threads().len(), 1);
    assert!(!c.events.iter().any(|e| matches!(e, Event::Error(_))));

    // Everyone leaves: the board is written and unloaded, then loaded for the next one.
    drop((a, b, v, c));
    tokio::time::sleep(Duration::from_millis(300)).await;
    shared.save_all();
    let mut c = Peer::join(&addr, &board, &view, doc(4)).await;
    c.pump().await;
    assert_eq!(c.count(), 2);
    assert_eq!(c.doc.assets().len(), 1);
    // Counted for monitoring: five joins, the last one still connected.
    let metrics = shared.metrics.render(1, 1);
    for line in [
        "paintboard_joins_total 5\n",
        "paintboard_connected 1\n",
        "paintboard_boards_made_total 1\n",
    ] {
        assert!(metrics.contains(line), "{metrics}");
    }
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
            joined |= events.iter().any(|e| {
                matches!(
                    e,
                    Event::Joined {
                        role: Role::Edit,
                        ..
                    }
                )
            });
        }
        if joined {
            break;
        }
    }
    assert!(joined);
    assert_eq!(socket.state(), SocketState::Open);
}

/// `GET path`: the status line and body.
async fn get_page(addr: &str, path: &str) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    (head.lines().next().unwrap().to_owned(), body.to_owned())
}

#[tokio::test]
async fn links_open_the_web_version() {
    let web = std::env::temp_dir().join(format!("pb-web-{}", std::process::id()));
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "the page").unwrap();
    std::fs::write(web.join("paintboard.js"), "the code").unwrap();
    std::fs::write(web.join("paintboard.js.gz"), "squeezed").unwrap();
    let shared = Shared::with_web(Store::in_memory().unwrap(), None, Some(web.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = router(shared).into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    assert_eq!(get_page(&addr, "/").await.1, "the page");
    assert_eq!(get_page(&addr, "/board/a1b2?key=c3").await.1, "the page");
    assert_eq!(get_page(&addr, "/board/paintboard.js").await.1, "the code");
    assert_eq!(get_page(&addr, "/paintboard.js").await.1, "the code");
    // Browsers that take gzip get the compressed copy.
    let mut stream = TcpStream::connect(&addr).await.unwrap();
    let request = format!(
        "GET /paintboard.js HTTP/1.1\r\nhost: {addr}\r\naccept-encoding: gzip, br\r\nconnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.to_lowercase().contains("content-encoding: gzip"));
    assert!(response.ends_with("squeezed"));
    assert!(get_page(&addr, "/missing.js").await.0.contains("404"));
    assert!(get_page(&addr, "/..%2Fsecret").await.0.contains("404"));
    assert!(get_page(&addr, "/health").await.1.contains("ok"));
    std::fs::remove_dir_all(&web).unwrap();
}

#[tokio::test]
async fn floods_are_cut_off() {
    let (addr, _) = start(None).await;
    let body = make_board(&addr, None).await.1;
    let (board, edit) = (field(&body, "board"), field(&body, "edit"));
    let mut a = Peer::join(&addr, &board, &edit, doc(1)).await;
    a.pump().await;
    // The server cuts the connection, possibly while this is still sending.
    let mut cut = false;
    for _ in 0..3000 {
        let frame = Message::Presence(pb_sync::Presence::default()).encode();
        if a.socket
            .send(tungstenite::Message::Binary(frame.into()))
            .await
            .is_err()
        {
            cut = true;
            break;
        }
    }
    if !cut {
        a.pump().await;
    }
    let told = a
        .events
        .iter()
        .any(|e| matches!(e, Event::Error(text) if text.contains("too many")));
    assert!(cut || told);
}

#[tokio::test]
async fn the_number_of_boards_can_be_limited() {
    let mut shared = Shared::new(Store::in_memory().unwrap(), None);
    std::sync::Arc::get_mut(&mut shared).unwrap().max_boards = Some(1);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = router(shared).into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    assert!(make_board(&addr, None).await.0.contains("200"));
    assert!(make_board(&addr, None).await.0.contains("507"));
}

#[tokio::test]
async fn one_address_makes_only_so_many_boards() {
    let mut shared = Shared::new(Store::in_memory().unwrap(), None);
    std::sync::Arc::get_mut(&mut shared)
        .unwrap()
        .limits
        .boards_per_hour = 2;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = router(shared).into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    assert!(make_board(&addr, None).await.0.contains("200"));
    assert!(make_board(&addr, None).await.0.contains("200"));
    assert!(make_board(&addr, None).await.0.contains("429"));
}

#[test]
fn backups_keep_the_newest() {
    let dir = std::env::temp_dir().join(format!("pb-backup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for day in ["paintboard-1.db", "paintboard-2.db", "paintboard-3.db"] {
        std::fs::write(dir.join(day), "old").unwrap();
    }
    let store = Store::in_memory().unwrap();
    let made = store.create_board(None).unwrap();
    let shared = Shared::new(store, None);
    shared.back_up(&dir, 2);
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names.len(), 2);
    // Today's copy is a working database with the board in it.
    let copy = names
        .iter()
        .find(|n| !n.starts_with("paintboard-3"))
        .unwrap();
    let copy = Store::open(&dir.join(copy)).unwrap();
    assert!(copy.load(&made.board).unwrap().is_some());
    drop(copy);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A server with accounts whose emails are kept: the server's address and the emails.
async fn start_with_accounts() -> (
    String,
    std::sync::Arc<Shared>,
    std::sync::Arc<std::sync::Mutex<Vec<crate::mail::Mail>>>,
) {
    let mut shared = Shared::new(Store::in_memory().unwrap(), None);
    let kept = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    std::sync::Arc::get_mut(&mut shared).unwrap().mailer =
        Some(crate::mail::Mailer::Kept(kept.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = router(shared.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, shared, kept)
}

/// Any request: the status code and body.
async fn http(
    addr: &str,
    method: &str,
    path: &str,
    session: Option<&str>,
    body: &str,
) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let auth = session.map_or(String::new(), |s| format!("authorization: Bearer {s}\r\n"));
    let request = format!(
        "{method} {path} HTTP/1.1\r\nhost: {addr}\r\n{auth}content-type: application/json\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, body.to_owned())
}

/// Signs `email` in the way a person would: asks, follows the emailed link, confirms.
async fn sign_in(
    addr: &str,
    kept: &std::sync::Mutex<Vec<crate::mail::Mail>>,
    email: &str,
) -> String {
    let (status, body) = http(
        addr,
        "POST",
        "/auth/start",
        None,
        &format!(r#"{{"email":"{email}","lang":"ru"}}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (request, code) = (field(&body, "request"), field(&body, "code"));
    let mail = kept.lock().unwrap().pop().unwrap();
    assert_eq!(mail.to, email.to_lowercase());
    assert!(mail.subject.contains(&code), "{}", mail.subject);
    let link = mail
        .body
        .split_whitespace()
        .find(|w| w.contains("/auth/"))
        .unwrap()
        .to_owned();
    let path = &link[link.find("/auth/").unwrap()..];
    let poll = format!(r#"{{"request":"{request}"}}"#);
    assert_eq!(http(addr, "POST", "/auth/poll", None, &poll).await.0, 202);
    let (status, page) = http(addr, "GET", path, None, "").await;
    assert_eq!(status, 200);
    assert!(page.contains(&code) && page.contains("<form"), "{page}");
    assert_eq!(http(addr, "POST", path, None, "").await.0, 200);
    let (status, body) = http(addr, "POST", "/auth/poll", None, &poll).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(http(addr, "POST", "/auth/poll", None, &poll).await.0, 410);
    // The link is used up with the sign-in.
    assert_eq!(http(addr, "POST", path, None, "").await.0, 410);
    field(&body, "session")
}

#[tokio::test]
async fn accounts_sign_in_by_email_and_own_boards() {
    let (addr, shared, kept) = start_with_accounts().await;
    let ann = sign_in(&addr, &kept, "Ann@Example.com").await;
    let (status, body) = http(&addr, "GET", "/me", Some(&ann), "").await;
    assert_eq!(status, 200);
    assert!(
        body.contains(r#""email":"ann@example.com""#) && body.contains(r#""boards":[]"#),
        "{body}"
    );
    assert_eq!(http(&addr, "GET", "/me", Some("forged"), "").await.0, 401);
    assert_eq!(http(&addr, "GET", "/me", None, "").await.0, 401);

    // A board made while signed in is the account's: it opens without a link.
    let (status, made) = http(&addr, "POST", "/boards", Some(&ann), "").await;
    assert_eq!(status, 200, "{made}");
    let board = field(&made, "board");
    let mut a = Peer::join_as(&addr, &board, "", &ann, doc(1)).await;
    a.pump().await;
    assert_eq!(a.client.joined(), Some(Role::Edit));
    a.doc.apply(Command::SetTitle("Plans".into())).unwrap();
    a.pump().await;
    shared.save_all();
    let (_, body) = http(&addr, "GET", "/me", Some(&ann), "").await;
    assert!(
        body.contains(r#""title":"Plans""#) && body.contains(r#""role":"owner""#),
        "{body}"
    );

    // The owner lets Bob comment; Bob hears of it by email and finds the board in his list.
    let path = format!("/boards/{board}/members");
    let (status, _) = http(
        &addr,
        "POST",
        &path,
        Some(&ann),
        r#"{"email":"bob@example.com","role":"comment"}"#,
    )
    .await;
    assert_eq!(status, 204);
    let invite = kept.lock().unwrap().pop().unwrap();
    assert_eq!(invite.to, "bob@example.com");
    assert!(
        invite.body.contains(&format!("/board/{board}")),
        "{}",
        invite.body
    );
    let bob = sign_in(&addr, &kept, "bob@example.com").await;
    let (_, body) = http(&addr, "GET", "/me", Some(&bob), "").await;
    assert!(
        body.contains(r#""role":"comment""#) && body.contains(&board),
        "{body}"
    );
    let mut b = Peer::join_as(&addr, &board, "", &bob, doc(2)).await;
    b.pump().await;
    assert_eq!(b.client.joined(), Some(Role::Comment));
    // A link gives more if it allows more.
    let mut b2 = Peer::join_as(&addr, &board, &field(&made, "edit"), &bob, doc(3)).await;
    b2.pump().await;
    assert_eq!(b2.client.joined(), Some(Role::Edit));
    // Only the owner manages the board.
    assert_eq!(http(&addr, "GET", &path, Some(&bob), "").await.0, 403);
    let (status, members) = http(&addr, "GET", &path, Some(&ann), "").await;
    assert_eq!(status, 200);
    assert!(
        members.contains("bob@example.com") && members.contains(r#""role":"owner""#),
        "{members}"
    );
    // Signed out, the account gives nothing.
    assert_eq!(
        http(&addr, "POST", "/auth/logout", Some(&bob), "").await.0,
        204
    );
    let mut out = Peer::join_as(&addr, &board, "", &bob, doc(4)).await;
    out.pump().await;
    assert_eq!(out.client.joined(), None);

    // A new edit link: the old one stops working.
    let links = format!("/boards/{board}/links");
    let (status, body) = http(&addr, "POST", &links, Some(&ann), r#"{"role":"edit"}"#).await;
    assert_eq!(status, 200, "{body}");
    let mut old = Peer::join(&addr, &board, &field(&made, "edit"), doc(5)).await;
    old.pump().await;
    assert_eq!(old.client.joined(), None);
    let mut new = Peer::join(&addr, &board, &field(&body, "token"), doc(6)).await;
    new.pump().await;
    assert_eq!(new.client.joined(), Some(Role::Edit));

    // Deleting the account deletes its boards, and those on them are let go.
    assert_eq!(http(&addr, "DELETE", "/me", Some(&ann), "").await.0, 204);
    a.pump().await;
    assert!(
        a.events
            .iter()
            .any(|e| matches!(e, Event::Error(t) if t.contains("deleted")))
    );
    assert_eq!(http(&addr, "GET", "/me", Some(&ann), "").await.0, 401);
    let mut late = Peer::join(&addr, &board, &field(&body, "token"), doc(7)).await;
    late.pump().await;
    assert_eq!(late.client.joined(), None);
}

#[tokio::test]
async fn servers_without_mail_have_no_accounts() {
    let (addr, _) = start(None).await;
    let (status, _) = http(&addr, "POST", "/auth/start", None, r#"{"email":"a@b.cd"}"#).await;
    assert_eq!(status, 404);
    // A session given anyway is ignored, not refused.
    let (status, _) = http(&addr, "POST", "/boards", Some("whatever"), "").await;
    assert_eq!(status, 200);
}
