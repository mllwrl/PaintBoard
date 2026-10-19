//! The network: a WebSocket that never blocks the UI, and HTTP requests whose answer
//! comes back through a callback. On the desktop they run on their own threads; in the
//! browser, the browser runs them.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketState {
    Connecting,
    Open,
    /// Gone, and why.
    Closed(String),
}

/// `Send + Sync` on the desktop, where callbacks run on other threads; nothing in the
/// browser, where everything runs on one.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> MaybeSend for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// Called when something arrived or the state changed (from another thread on the
/// desktop).
#[cfg(not(target_arch = "wasm32"))]
type Waker = std::sync::Arc<dyn Fn() + Send + Sync>;
#[cfg(target_arch = "wasm32")]
type Waker = std::rc::Rc<dyn Fn()>;

/// A WebSocket carrying binary messages.
pub struct Socket {
    imp: imp::Socket,
}

impl Socket {
    /// Starts connecting to `url` (`ws://` or `wss://`). Messages sent before it is open
    /// wait for it.
    pub fn connect(url: &str, wake: impl Fn() + MaybeSend + 'static) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let wake: Waker = std::sync::Arc::new(wake);
        #[cfg(target_arch = "wasm32")]
        let wake: Waker = std::rc::Rc::new(wake);
        Self {
            imp: imp::Socket::connect(url, wake),
        }
    }

    pub fn send(&self, bytes: Vec<u8>) {
        self.imp.send(bytes);
    }

    /// The next message that arrived, if any.
    pub fn try_recv(&self) -> Option<Vec<u8>> {
        self.imp.try_recv()
    }

    pub fn state(&self) -> SocketState {
        self.imp.state()
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.imp.close();
    }
}

/// POSTs to `url` with an empty body, `header` added (name, value), and hands the answer's
/// body (or why there is none) to `done`.
pub fn post(
    url: &str,
    header: Option<(&str, &str)>,
    done: impl FnOnce(Result<String, String>) + MaybeSend + 'static,
) {
    imp::post(url, header, Box::new(done));
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::collections::VecDeque;
    use std::io::{ErrorKind, Read};
    use std::net::TcpStream;
    use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, WebSocket};

    use super::{SocketState, Waker};

    /// How long a read waits before the thread looks for messages to send.
    const POLL: Duration = Duration::from_millis(30);

    #[derive(Default)]
    struct Inbox {
        messages: VecDeque<Vec<u8>>,
        state: Option<SocketState>,
    }

    pub struct Socket {
        outbox: Sender<Option<Vec<u8>>>,
        inbox: Arc<Mutex<Inbox>>,
    }

    impl Socket {
        pub fn connect(url: &str, wake: Waker) -> Self {
            let (outbox, outgoing) = channel();
            let inbox = Arc::new(Mutex::new(Inbox::default()));
            let shared = inbox.clone();
            let url = url.to_owned();
            let spawned = std::thread::Builder::new()
                .name("websocket".into())
                .spawn(move || {
                    let reason = match tungstenite::connect(&url) {
                        Ok((socket, _)) => run(socket, &outgoing, &shared, &wake),
                        Err(err) => err.to_string(),
                    };
                    if let Ok(mut inbox) = shared.lock() {
                        inbox.state = Some(SocketState::Closed(reason));
                    }
                    wake();
                });
            if let Err(err) = spawned
                && let Ok(mut inbox) = inbox.lock()
            {
                inbox.state = Some(SocketState::Closed(err.to_string()));
            }
            Self { outbox, inbox }
        }

        pub fn send(&self, bytes: Vec<u8>) {
            let _ = self.outbox.send(Some(bytes));
        }

        pub fn try_recv(&self) -> Option<Vec<u8>> {
            self.inbox.lock().ok()?.messages.pop_front()
        }

        pub fn state(&self) -> SocketState {
            self.inbox
                .lock()
                .ok()
                .and_then(|i| i.state.clone())
                .unwrap_or(SocketState::Connecting)
        }

        pub fn close(&self) {
            let _ = self.outbox.send(None);
        }
    }

    fn set_timeout(socket: &WebSocket<MaybeTlsStream<TcpStream>>) -> std::io::Result<()> {
        let tcp = match socket.get_ref() {
            MaybeTlsStream::Plain(tcp) => tcp,
            MaybeTlsStream::NativeTls(tls) => tls.get_ref(),
            _ => return Ok(()),
        };
        tcp.set_read_timeout(Some(POLL))
    }

    /// Passes messages both ways until the connection ends; returns why it did.
    fn run(
        mut socket: WebSocket<MaybeTlsStream<TcpStream>>,
        outgoing: &Receiver<Option<Vec<u8>>>,
        inbox: &Mutex<Inbox>,
        wake: &Waker,
    ) -> String {
        if let Err(err) = set_timeout(&socket) {
            return err.to_string();
        }
        if let Ok(mut inbox) = inbox.lock() {
            inbox.state = Some(SocketState::Open);
        }
        wake();
        loop {
            loop {
                match outgoing.try_recv() {
                    Ok(Some(bytes)) => {
                        if let Err(err) = socket.send(Message::Binary(bytes.into())) {
                            return err.to_string();
                        }
                    }
                    Ok(None) | Err(TryRecvError::Disconnected) => {
                        let _ = socket.close(None);
                        let _ = socket.flush();
                        return "closed".into();
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
            match socket.read() {
                Ok(Message::Binary(bytes)) => {
                    if let Ok(mut inbox) = inbox.lock() {
                        inbox.messages.push_back(bytes.to_vec());
                    }
                    wake();
                }
                Ok(Message::Close(frame)) => {
                    return frame.map_or_else(|| "closed".into(), |f| f.reason.to_string());
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(err))
                    if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(err) => return err.to_string(),
            }
        }
    }

    pub fn post(
        url: &str,
        header: Option<(&str, &str)>,
        done: Box<dyn FnOnce(Result<String, String>) + Send>,
    ) {
        let url = url.to_owned();
        let header = header.map(|(k, v)| (k.to_owned(), v.to_owned()));
        std::thread::spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(20)))
                .http_status_as_error(false)
                .build()
                .into();
            let mut request = agent.post(&url);
            if let Some((name, value)) = &header {
                request = request.header(name, value);
            }
            let result = request
                .send_empty()
                .map_err(|e| e.to_string())
                .and_then(|response| {
                    let status = response.status();
                    let mut body = String::new();
                    response
                        .into_body()
                        .into_reader()
                        .take(1 << 20)
                        .read_to_string(&mut body)
                        .map_err(|e| e.to_string())?;
                    if status.is_success() {
                        Ok(body)
                    } else {
                        Err(format!("{status}: {body}"))
                    }
                });
            done(result);
        });
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use js_sys::{ArrayBuffer, Uint8Array};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{BinaryType, CloseEvent, MessageEvent, RequestInit, Response, WebSocket};

    use super::{SocketState, Waker};

    #[derive(Default)]
    struct Shared {
        messages: VecDeque<Vec<u8>>,
        /// Sent before the socket opened.
        waiting: Vec<Vec<u8>>,
        state: Option<SocketState>,
    }

    pub struct Socket {
        socket: Option<WebSocket>,
        shared: Rc<RefCell<Shared>>,
        _handlers: Vec<Closure<dyn FnMut(JsValueEvent)>>,
    }

    type JsValueEvent = wasm_bindgen::JsValue;

    impl Socket {
        pub fn connect(url: &str, wake: Waker) -> Self {
            let shared = Rc::new(RefCell::new(Shared::default()));
            let socket = match WebSocket::new(url) {
                Ok(socket) => socket,
                Err(err) => {
                    shared.borrow_mut().state = Some(SocketState::Closed(format!("{err:?}")));
                    return Self {
                        socket: None,
                        shared,
                        _handlers: Vec::new(),
                    };
                }
            };
            socket.set_binary_type(BinaryType::Arraybuffer);
            let on_open = {
                let (shared, socket, wake) = (shared.clone(), socket.clone(), wake.clone());
                Closure::<dyn FnMut(JsValueEvent)>::new(move |_| {
                    let waiting = {
                        let mut shared = shared.borrow_mut();
                        shared.state = Some(SocketState::Open);
                        std::mem::take(&mut shared.waiting)
                    };
                    for bytes in waiting {
                        let _ = socket.send_with_u8_array(&bytes);
                    }
                    wake();
                })
            };
            let on_message = {
                let (shared, wake) = (shared.clone(), wake.clone());
                Closure::<dyn FnMut(JsValueEvent)>::new(move |event: JsValueEvent| {
                    let Some(event) = event.dyn_ref::<MessageEvent>() else {
                        return;
                    };
                    if let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() {
                        let bytes = Uint8Array::new(&buffer).to_vec();
                        shared.borrow_mut().messages.push_back(bytes);
                        wake();
                    }
                })
            };
            let on_close = {
                let (shared, wake) = (shared.clone(), wake.clone());
                Closure::<dyn FnMut(JsValueEvent)>::new(move |event: JsValueEvent| {
                    let reason = event
                        .dyn_ref::<CloseEvent>()
                        .map(CloseEvent::reason)
                        .filter(|r| !r.is_empty())
                        .unwrap_or_else(|| "the connection was lost".into());
                    shared.borrow_mut().state = Some(SocketState::Closed(reason));
                    wake();
                })
            };
            socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            Self {
                socket: Some(socket),
                shared,
                _handlers: vec![on_open, on_message, on_close],
            }
        }

        pub fn send(&self, bytes: Vec<u8>) {
            let open = self.shared.borrow().state == Some(SocketState::Open);
            match &self.socket {
                Some(socket) if open => {
                    let _ = socket.send_with_u8_array(&bytes);
                }
                _ => self.shared.borrow_mut().waiting.push(bytes),
            }
        }

        pub fn try_recv(&self) -> Option<Vec<u8>> {
            self.shared.borrow_mut().messages.pop_front()
        }

        pub fn state(&self) -> SocketState {
            self.shared
                .borrow()
                .state
                .clone()
                .unwrap_or(SocketState::Connecting)
        }

        pub fn close(&self) {
            if let Some(socket) = &self.socket {
                socket.set_onclose(None);
                socket.set_onmessage(None);
                socket.set_onopen(None);
                let _ = socket.close();
            }
        }
    }

    pub fn post(
        url: &str,
        header: Option<(&str, &str)>,
        done: Box<dyn FnOnce(Result<String, String>)>,
    ) {
        let init = RequestInit::new();
        init.set_method("POST");
        if let Some((name, value)) = header {
            let headers = web_sys::Headers::new().expect("empty headers");
            let _ = headers.set(name, value);
            init.set_headers(&headers);
        }
        let Some(window) = web_sys::window() else {
            done(Err("no window".into()));
            return;
        };
        let request = window.fetch_with_str_and_init(url, &init);
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let response: Response = JsFuture::from(request)
                    .await
                    .map_err(|e| format!("{e:?}"))?
                    .unchecked_into();
                let text = response.text().map_err(|e| format!("{e:?}"))?;
                let body = JsFuture::from(text)
                    .await
                    .map_err(|e| format!("{e:?}"))?
                    .as_string()
                    .unwrap_or_default();
                if response.ok() {
                    Ok(body)
                } else {
                    Err(format!("{}: {body}", response.status()))
                }
            }
            .await;
            done(result);
        });
    }
}
