//! One running copy of the app on the desktop. The first copy listens on a local port
//! written to the app data folder, with a secret only programs that can read the folder
//! know. A copy started after it hands over the file it was started with (if any) and
//! quits; the first one opens it. Anything going wrong lets the later copy run on its own.
//! In the browser every tab is its own app, so these do nothing there.

use std::path::{Path, PathBuf};

/// Hands `file` (or just the wish to be shown) to the copy of the app already running;
/// returns whether one took it, in which case this copy should quit.
pub fn hand_over(file: Option<&Path>) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::fs::app_data_dir().is_ok_and(|dir| imp::hand_over(&dir, file))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = file;
        false
    }
}

/// Makes this copy the one later copies hand their files to: `on_start` gets each one, on a
/// background thread, as the file to open or `None` to just be shown.
pub fn listen(on_start: impl Fn(Option<PathBuf>) + Send + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    match crate::fs::app_data_dir() {
        Ok(dir) => {
            if let Err(err) = imp::listen(&dir, on_start) {
                log::warn!("later starts will open another window: {err}");
            }
        }
        Err(err) => log::warn!("later starts will open another window: {err}"),
    }
    #[cfg(target_arch = "wasm32")]
    let _ = on_start;
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::hash::{BuildHasher, Hasher};
    use std::io::{self, Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// The file in the app data folder that says where the running copy listens.
    const FILE: &str = "instance";
    /// What every message starts with, so stray connections are told apart.
    const HELLO: &str = "PaintBoard start 1";
    /// The running copy's answer once it has the message.
    const TAKEN: &[u8] = b"taken";
    /// Messages are a path at most; anything longer is not one of ours.
    const MAX_MESSAGE: u64 = 64 * 1024;
    /// How long either side waits for the other.
    const WAIT: Duration = Duration::from_secs(3);

    /// Where the running copy listens, as written in its file.
    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct Place {
        pub port: u16,
        pub secret: String,
        pub pid: u32,
    }

    impl Place {
        pub fn to_text(&self) -> String {
            format!("{} {} {}", self.port, self.secret, self.pid)
        }

        pub fn parse(text: &str) -> Option<Self> {
            let mut parts = text.split_whitespace();
            let place = Self {
                port: parts.next()?.parse().ok()?,
                secret: parts.next()?.to_owned(),
                pid: parts.next()?.parse().ok()?,
            };
            (parts.next().is_none() && !place.secret.is_empty()).then_some(place)
        }
    }

    /// The message a later copy sends: who it is, the secret, and the file to open.
    pub(super) fn message(secret: &str, file: Option<&str>) -> String {
        format!("{HELLO}\n{secret}\n{}", file.unwrap_or(""))
    }

    /// The file a message asks to open (`Some(None)` to just be shown), or `None` if it
    /// is not a message with the right secret.
    pub(super) fn read_message(text: &str, secret: &str) -> Option<Option<PathBuf>> {
        let rest = text.strip_prefix(HELLO)?.strip_prefix('\n')?;
        let (given, file) = rest.split_once('\n')?;
        (given == secret).then(|| (!file.is_empty()).then(|| PathBuf::from(file)))
    }

    /// A secret no other program guesses: 128 bits from the system's random hashing keys.
    fn new_secret() -> String {
        let half = || {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u128(crate::now_ms() as u128);
            hasher.finish()
        };
        format!("{:016x}{:016x}", half(), half())
    }

    pub fn hand_over(dir: &Path, file: Option<&Path>) -> bool {
        match try_hand_over(dir, file) {
            Ok(()) => true,
            Err(err) => {
                log::debug!("no running copy took the start: {err}");
                false
            }
        }
    }

    fn try_hand_over(dir: &Path, file: Option<&Path>) -> io::Result<()> {
        let text = std::fs::read_to_string(dir.join(FILE))?;
        let place = Place::parse(&text).ok_or_else(|| io::Error::other("unreadable file"))?;
        // The running copy's working folder may differ, so the path goes whole.
        let file = file.map(std::path::absolute).transpose()?;
        let file = match &file {
            Some(f) => Some(
                f.to_str()
                    .ok_or_else(|| io::Error::other("path not UTF-8"))?,
            ),
            None => None,
        };
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, place.port));
        let mut stream = TcpStream::connect_timeout(&address, WAIT)?;
        stream.set_read_timeout(Some(WAIT))?;
        stream.set_write_timeout(Some(WAIT))?;
        // The running copy may bring its window to the front: this one was started by
        // the person, so Windows lets it pass that on.
        allow_to_front(place.pid);
        stream.write_all(message(&place.secret, file).as_bytes())?;
        stream.shutdown(std::net::Shutdown::Write)?;
        let mut answer = Vec::new();
        stream.take(TAKEN.len() as u64).read_to_end(&mut answer)?;
        if answer == TAKEN {
            Ok(())
        } else {
            Err(io::Error::other("not taken"))
        }
    }

    pub fn listen(
        dir: &Path,
        on_start: impl Fn(Option<PathBuf>) + Send + 'static,
    ) -> io::Result<()> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let place = Place {
            port: listener.local_addr()?.port(),
            secret: new_secret(),
            pid: std::process::id(),
        };
        crate::fs::write_atomic(&dir.join(FILE), place.to_text().as_bytes())?;
        std::thread::Builder::new()
            .name("instance".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    if let Some(file) = receive(stream, &place.secret) {
                        on_start(file);
                    }
                }
            })?;
        Ok(())
    }

    /// Reads one later copy's message and answers it; `None` if it was not one.
    fn receive(mut stream: TcpStream, secret: &str) -> Option<Option<PathBuf>> {
        if !stream.peer_addr().ok()?.ip().is_loopback() {
            return None;
        }
        stream.set_read_timeout(Some(WAIT)).ok()?;
        stream.set_write_timeout(Some(WAIT)).ok()?;
        let mut bytes = Vec::new();
        (&mut stream)
            .take(MAX_MESSAGE)
            .read_to_end(&mut bytes)
            .ok()?;
        let file = read_message(std::str::from_utf8(&bytes).ok()?, secret)?;
        stream.write_all(TAKEN).ok()?;
        Some(file)
    }

    /// Lets process `pid` bring its window in front of the one in use.
    #[cfg(windows)]
    fn allow_to_front(pid: u32) {
        // SAFETY: the call takes a plain number and touches no memory of ours.
        #[allow(unsafe_code)]
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid) };
    }

    #[cfg(not(windows))]
    fn allow_to_front(_pid: u32) {}
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::imp::*;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pb-instance-{name}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn place_round_trips_and_rejects_junk() {
        let place = Place {
            port: 51234,
            secret: "abc123".into(),
            pid: 42,
        };
        assert_eq!(Place::parse(&place.to_text()), Some(place));
        assert_eq!(Place::parse(""), None);
        assert_eq!(Place::parse("80 secret"), None);
        assert_eq!(Place::parse("99999 secret 1"), None);
        assert_eq!(Place::parse("80 secret 1 extra"), None);
    }

    #[test]
    fn messages_need_the_secret() {
        let file = r"C:\Boards\Plan.pboard";
        assert_eq!(
            read_message(&message("s3", Some(file)), "s3"),
            Some(Some(PathBuf::from(file)))
        );
        assert_eq!(read_message(&message("s3", None), "s3"), Some(None));
        assert_eq!(read_message(&message("guess", Some(file)), "s3"), None);
        assert_eq!(read_message("GET / HTTP/1.1\r\n\r\n", "s3"), None);
        assert_eq!(read_message("", "s3"), None);
    }

    #[test]
    fn a_later_start_hands_its_file_to_the_running_copy() {
        let dir = scratch("handover");
        // Nothing runs yet: the start goes on by itself.
        assert!(!hand_over(&dir, None));

        let (sender, received) = mpsc::channel();
        listen(&dir, move |file| {
            let _ = sender.send(file);
        })
        .unwrap();
        let board = dir.join("Plan.pboard");
        assert!(hand_over(&dir, Some(&board)));
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            Some(board)
        );
        assert!(hand_over(&dir, None));
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), None);

        // A relative path arrives whole.
        assert!(hand_over(&dir, Some(std::path::Path::new("rel.pboard"))));
        let got = received
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(got.is_absolute() && got.ends_with("rel.pboard"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_secret_or_a_gone_copy_lets_the_start_go_on() {
        let dir = scratch("stale");
        let (sender, received) = mpsc::channel();
        listen(&dir, move |file| {
            let _ = sender.send(file);
        })
        .unwrap();
        // Someone else's secret is not taken.
        let text = std::fs::read_to_string(dir.join("instance")).unwrap();
        let mut place = Place::parse(&text).unwrap();
        place.secret = "wrong".into();
        std::fs::write(dir.join("instance"), place.to_text()).unwrap();
        assert!(!hand_over(&dir, None));
        assert!(received.recv_timeout(Duration::from_millis(200)).is_err());

        // Left behind by a copy that is gone: nobody listens on the port.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        place.port = free.local_addr().unwrap().port();
        drop(free);
        std::fs::write(dir.join("instance"), place.to_text()).unwrap();
        assert!(!hand_over(&dir, None));

        std::fs::write(dir.join("instance"), "garbage").unwrap();
        assert!(!hand_over(&dir, None));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
