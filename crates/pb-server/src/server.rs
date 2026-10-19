//! HTTP and WebSocket handling: boards are made with `POST /boards` and shared over
//! `GET /ws`, where the first message is the client's `Hello`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{self, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use pb_core::AssetId;
use pb_sync::{ClientId, MAX_ASSET, Message, Outgoing, PROTOCOL, Room};
use tokio::sync::mpsc;

use crate::store::{Access, Store};

/// How long a new connection has to say which board it wants.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// Messages a connection may send per second on average, and at once: typing and
/// pointer moves stay well below; a client flooding the server is cut off.
const RATE: f64 = 200.0;
const BURST: f64 = 600.0;

/// Allows messages at [`RATE`] per second with bursts of up to [`BURST`].
struct Bucket {
    tokens: f64,
    at: std::time::Instant,
}

impl Bucket {
    fn new() -> Self {
        Self {
            tokens: BURST,
            at: std::time::Instant::now(),
        }
    }

    /// Takes one message; false if over the limit.
    fn take(&mut self) -> bool {
        let now = std::time::Instant::now();
        let refill = now.duration_since(self.at).as_secs_f64() * RATE;
        self.tokens = (self.tokens + refill).min(BURST);
        self.at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

/// How often changed boards are written to the database.
pub const SAVE_EVERY: Duration = Duration::from_secs(2);

/// Everything the server shares between connections.
pub struct Shared {
    store: Mutex<Store>,
    /// Boards someone is connected to, by id.
    rooms: Mutex<HashMap<String, Arc<Mutex<Live>>>>,
    next_client: AtomicU64,
    /// When set, making a board needs this in the `x-create-key` header.
    create_key: Option<String>,
    /// The built web version to serve, so links open the board in a browser.
    web: Option<std::path::PathBuf>,
    /// The most boards the server keeps; `None` for no limit.
    pub max_boards: Option<usize>,
}

/// A board in memory and the connections to it.
struct Live {
    room: Room,
    senders: HashMap<ClientId, mpsc::UnboundedSender<Message>>,
    /// Pictures already in the database.
    saved_assets: HashSet<AssetId>,
}

impl Shared {
    #[cfg(test)]
    pub fn new(store: Store, create_key: Option<String>) -> Arc<Self> {
        Self::with_web(store, create_key, None)
    }

    pub fn with_web(
        store: Store,
        create_key: Option<String>,
        web: Option<std::path::PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(store),
            rooms: Mutex::new(HashMap::new()),
            next_client: AtomicU64::new(1),
            create_key,
            web,
            max_boards: None,
        })
    }

    /// The board in memory, loaded from the database if needed.
    fn room(&self, board: &str) -> Result<Arc<Mutex<Live>>, String> {
        let mut rooms = self.rooms.lock().expect("rooms lock");
        if let Some(live) = rooms.get(board) {
            return Ok(live.clone());
        }
        let store = self.store.lock().expect("store lock");
        let (snapshot, assets) = store
            .load(board)
            .map_err(|e| e.to_string())?
            .ok_or("no such board")?;
        let saved_assets = assets.iter().map(|a| AssetId::of(a)).collect();
        let room = match snapshot {
            Some(snapshot) => Room::load(&snapshot, assets)?,
            None => Room::new(),
        };
        let live = Arc::new(Mutex::new(Live {
            room,
            senders: HashMap::new(),
            saved_assets,
        }));
        rooms.insert(board.to_owned(), live.clone());
        Ok(live)
    }

    /// Writes `live` if it changed.
    fn save(&self, board: &str, live: &mut Live) {
        if !live.room.take_changed() {
            return;
        }
        let snapshot = live.room.snapshot();
        let new: Vec<(AssetId, Arc<[u8]>)> = live
            .room
            .asset_ids()
            .filter(|id| !live.saved_assets.contains(id))
            .filter_map(|id| Some((id, live.room.asset(id)?.clone())))
            .collect();
        let assets: Vec<(AssetId, &[u8])> = new.iter().map(|(id, b)| (*id, &**b)).collect();
        let saved = self
            .store
            .lock()
            .expect("store lock")
            .save(board, &snapshot, &assets);
        match saved {
            Ok(()) => live.saved_assets.extend(new.iter().map(|(id, _)| *id)),
            Err(err) => log::error!("could not save board {board}: {err}"),
        }
    }

    /// Copies the database to `dir` as `paintboard-<day>.db`, keeping the newest `keep`.
    pub fn back_up(&self, dir: &std::path::Path, keep: usize) {
        let day = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() / 86_400);
        let target = dir.join(format!("paintboard-{day}.db"));
        let made = std::fs::create_dir_all(dir)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                self.store
                    .lock()
                    .expect("store lock")
                    .back_up(&target)
                    .map_err(|e| e.to_string())
            });
        match made {
            Ok(()) => log::info!("backed up to {}", target.display()),
            Err(err) => log::error!("backup failed: {err}"),
        }
        let mut old: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("paintboard-") && n.ends_with(".db"))
            })
            .collect();
        old.sort();
        let extra = old.len().saturating_sub(keep);
        for path in &old[..extra] {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Writes every changed board (periodically, and before stopping).
    pub fn save_all(&self) {
        let rooms: Vec<(String, Arc<Mutex<Live>>)> = self
            .rooms
            .lock()
            .expect("rooms lock")
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (board, live) in rooms {
            let mut live = live.lock().expect("room lock");
            self.save(&board, &mut live);
        }
    }

    /// Writes and forgets the board once nobody is connected to it.
    fn unload_if_empty(&self, board: &str) {
        let mut rooms = self.rooms.lock().expect("rooms lock");
        let Some(live) = rooms.get(board).cloned() else {
            return;
        };
        let mut live = live.lock().expect("room lock");
        if live.room.is_empty() {
            self.save(board, &mut live);
            rooms.remove(board);
        }
    }
}

pub fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/boards", post(create_board).options(allow_other_sites))
        .route("/ws", get(connect))
        .route("/", get(|state| web_file(state, "index.html".to_owned())))
        .route("/board/{name}", get(board_page))
        .route("/{name}", get(|state, Path(name)| web_file(state, name)))
        .with_state(shared)
}

/// `/board/<id>` is the web version opening that board; its files (`/board/x.js`, as the
/// page asks for them relative to itself) are the web version's.
async fn board_page(state: State<Arc<Shared>>, Path(name): Path<String>) -> Response {
    let file = if name.contains('.') {
        name
    } else {
        "index.html".to_owned()
    };
    web_file(state, file).await
}

/// A file of the web version, if the server serves it.
async fn web_file(State(shared): State<Arc<Shared>>, name: String) -> Response {
    let Some(dir) = &shared.web else {
        return (
            StatusCode::NOT_FOUND,
            "this server does not serve the web version",
        )
            .into_response();
    };
    // Only plain names in the web version's folder.
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !name.starts_with('.');
    if !plain {
        return StatusCode::NOT_FOUND.into_response();
    }
    let kind = match name.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        Some("css") => "text/css",
        _ => "application/octet-stream",
    };
    match tokio::fs::read(dir.join(&name)).await {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, kind)], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The web version may be served from another site than the server: let browsers ask.
fn cors(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert("access-control-allow-origin", "*".parse().expect("valid"));
    headers.insert(
        "access-control-allow-methods",
        "POST".parse().expect("valid"),
    );
    headers.insert(
        "access-control-allow-headers",
        "x-create-key".parse().expect("valid"),
    );
    response
}

async fn allow_other_sites() -> Response {
    cors(StatusCode::NO_CONTENT.into_response())
}

async fn create_board(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    cors(make_board(&shared, &headers))
}

fn make_board(shared: &Shared, headers: &HeaderMap) -> Response {
    if let Some(key) = &shared.create_key {
        let given = headers.get("x-create-key").and_then(|v| v.to_str().ok());
        if given != Some(key.as_str()) {
            return (StatusCode::FORBIDDEN, "a key is needed to make boards").into_response();
        }
    }
    let store = shared.store.lock().expect("store lock");
    if let Some(max) = shared.max_boards
        && store.board_count().unwrap_or(0) >= max
    {
        return (
            StatusCode::INSUFFICIENT_STORAGE,
            "the server has no room for more boards",
        )
            .into_response();
    }
    let made = store.create_board();
    drop(store);
    match made {
        Ok(new) => {
            log::info!("board {} made", new.board);
            Json(std::collections::BTreeMap::from([
                ("board", new.board),
                ("edit", new.edit),
                ("view", new.view),
            ]))
            .into_response()
        }
        Err(err) => {
            log::error!("could not make a board: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "could not make a board").into_response()
        }
    }
}

async fn connect(State(shared): State<Arc<Shared>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .max_message_size(MAX_ASSET + (1 << 20))
        .on_upgrade(move |socket| serve(socket, shared))
}

async fn send(socket: &mut WebSocket, message: &Message) -> bool {
    socket
        .send(ws::Message::Binary(message.encode().into()))
        .await
        .is_ok()
}

/// The next message from the client, or `None` once it is gone.
async fn receive(socket: &mut WebSocket) -> Option<Result<Message, String>> {
    loop {
        match socket.recv().await? {
            Ok(ws::Message::Binary(bytes)) => {
                return Some(Message::decode(&bytes).map_err(|e| e.to_string()));
            }
            Ok(ws::Message::Close(_)) | Err(_) => return None,
            // Pings are answered by axum; text is not part of the protocol.
            Ok(_) => {}
        }
    }
}

async fn refuse(mut socket: WebSocket, why: &str) {
    send(&mut socket, &Message::Error(why.to_owned())).await;
    let _ = socket.send(ws::Message::Close(None)).await;
}

async fn serve(mut socket: WebSocket, shared: Arc<Shared>) {
    let hello = tokio::time::timeout(HELLO_TIMEOUT, receive(&mut socket)).await;
    let Ok(Some(Ok(Message::Hello {
        protocol,
        board,
        token,
        name,
        version,
        assets,
    }))) = hello
    else {
        return refuse(socket, "expected a greeting").await;
    };
    if protocol != PROTOCOL {
        return refuse(
            socket,
            "this server speaks another version: update PaintBoard",
        )
        .await;
    }
    let access = shared
        .store
        .lock()
        .expect("store lock")
        .access(&board, &token);
    let can_edit = match access {
        Ok(Some(access)) => access == Access::Edit,
        Ok(None) => return refuse(socket, "no such board, or the link is wrong").await,
        Err(err) => {
            log::error!("access check failed: {err}");
            return refuse(socket, "the server could not check the link").await;
        }
    };
    let live = match shared.room(&board) {
        Ok(live) => live,
        Err(err) => return refuse(socket, &err).await,
    };
    let client = shared.next_client.fetch_add(1, Ordering::Relaxed);
    let (outbox, mut inbox) = mpsc::unbounded_channel();
    let joined = {
        let mut live = live.lock().expect("room lock");
        live.senders.insert(client, outbox);
        let joined = live.room.join(client, can_edit, &version, &assets);
        match joined {
            Ok(out) => {
                dispatch(&live, out);
                Ok(())
            }
            Err(err) => {
                live.senders.remove(&client);
                Err(err)
            }
        }
    };
    if let Err(err) = joined {
        shared.unload_if_empty(&board);
        return refuse(socket, &err).await;
    }
    log::info!("{name} ({client}) joined board {board}");
    let mut bucket = Bucket::new();

    loop {
        tokio::select! {
            incoming = receive(&mut socket) => {
                let Some(incoming) = incoming else { break };
                let incoming = if bucket.take() {
                    incoming
                } else {
                    Err("too many messages: slow down".to_owned())
                };
                let result = incoming.and_then(|message| {
                    let mut live = live.lock().expect("room lock");
                    let out = live.room.receive(client, message)?;
                    dispatch(&live, out);
                    Ok(())
                });
                if let Err(err) = result {
                    log::info!("{client} on {board}: {err}");
                    send(&mut socket, &Message::Error(err)).await;
                    break;
                }
            }
            outgoing = inbox.recv() => {
                let Some(message) = outgoing else { break };
                if !send(&mut socket, &message).await {
                    break;
                }
            }
        }
    }

    {
        let mut live = live.lock().expect("room lock");
        live.senders.remove(&client);
        let out = live.room.leave(client);
        dispatch(&live, out);
    }
    log::info!("{client} left board {board}");
    shared.unload_if_empty(&board);
}

fn dispatch(live: &Live, out: Outgoing) {
    for (to, message) in out {
        if let Some(sender) = live.senders.get(&to) {
            let _ = sender.send(message);
        }
    }
}
