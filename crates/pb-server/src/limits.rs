//! Limits per address, so one machine cannot take the server for itself: boards made per
//! hour, connections at once, link previews per minute. IPv6 addresses count by their
//! /64 network, which one machine usually has whole.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;

/// Addresses remembered before old ones are forgotten.
const REMEMBERED: usize = 4096;

pub struct Limits {
    /// Boards one address may make per hour.
    pub boards_per_hour: u32,
    /// Connections to boards one address may have at once.
    pub connections: u32,
    /// Link previews one address may ask for per minute.
    pub previews_per_minute: u32,
    /// The server is behind a proxy (Caddy) that puts the client's address last in
    /// `X-Forwarded-For`; otherwise the header could be made up and is ignored.
    pub trust_proxy: bool,
    seen: Arc<Mutex<HashMap<IpAddr, Seen>>>,
}

#[derive(Default)]
struct Seen {
    boards: Window,
    previews: Window,
    connections: u32,
}

/// Uses counted from the start of a period.
#[derive(Default)]
struct Window {
    start: Option<Instant>,
    count: u32,
}

impl Window {
    /// Counts one use if fewer than `max` were made in the period `length` running now.
    fn take(&mut self, length: Duration, max: u32, now: Instant) -> bool {
        if self.start.is_none_or(|s| now.duration_since(s) >= length) {
            *self = Window {
                start: Some(now),
                count: 0,
            };
        }
        if self.count >= max {
            return false;
        }
        self.count += 1;
        true
    }

    fn recent(&self, now: Instant) -> bool {
        self.start
            .is_some_and(|s| now.duration_since(s) < Duration::from_secs(3600))
    }
}

/// One connection of an address, counted until dropped.
pub struct Connection {
    seen: Arc<Mutex<HashMap<IpAddr, Seen>>>,
    ip: IpAddr,
}

impl Drop for Connection {
    fn drop(&mut self) {
        let mut seen = self.seen.lock().expect("limits lock");
        if let Some(entry) = seen.get_mut(&self.ip) {
            entry.connections = entry.connections.saturating_sub(1);
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            boards_per_hour: 20,
            connections: 16,
            previews_per_minute: 20,
            trust_proxy: false,
            seen: Arc::default(),
        }
    }
}

impl Limits {
    /// Who is asking: the peer, or the client the proxy names when it is trusted. IPv6
    /// addresses are cut to their /64 network.
    pub fn client(&self, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
        let forwarded = self
            .trust_proxy
            .then(|| headers.get("x-forwarded-for")?.to_str().ok())
            .flatten()
            .and_then(|list| list.rsplit(',').next()?.trim().parse().ok());
        match forwarded.unwrap_or(peer.ip()) {
            IpAddr::V6(v6) if v6.to_ipv4_mapped().is_none() => {
                let s = v6.segments();
                IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
            IpAddr::V6(v6) => IpAddr::V4(v6.to_ipv4_mapped().expect("mapped")),
            v4 => v4,
        }
    }

    /// Counts a board made by `ip`; false if it made too many lately.
    pub fn make_board(&self, ip: IpAddr) -> bool {
        let max = self.boards_per_hour;
        self.with(ip, |seen, now| {
            seen.boards.take(Duration::from_secs(3600), max, now)
        })
    }

    /// Counts a link preview asked by `ip`; false if it asked too many lately.
    pub fn preview(&self, ip: IpAddr) -> bool {
        let max = self.previews_per_minute;
        self.with(ip, |seen, now| {
            seen.previews.take(Duration::from_secs(60), max, now)
        })
    }

    /// A connection of `ip`, counted while it is kept; `None` if it has too many.
    pub fn connect(&self, ip: IpAddr) -> Option<Connection> {
        let max = self.connections;
        let allowed = self.with(ip, |seen, _| {
            if seen.connections >= max {
                return false;
            }
            seen.connections += 1;
            true
        });
        allowed.then(|| Connection {
            seen: self.seen.clone(),
            ip,
        })
    }

    fn with(&self, ip: IpAddr, f: impl FnOnce(&mut Seen, Instant) -> bool) -> bool {
        let now = Instant::now();
        let mut seen = self.seen.lock().expect("limits lock");
        if seen.len() >= REMEMBERED && !seen.contains_key(&ip) {
            seen.retain(|_, s| s.connections > 0 || s.boards.recent(now) || s.previews.recent(now));
        }
        f(seen.entry(ip).or_default(), now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn each_address_has_its_own_allowance() {
        let limits = Limits {
            boards_per_hour: 2,
            connections: 1,
            ..Limits::default()
        };
        let (a, b) = (ip("203.0.113.1"), ip("203.0.113.2"));
        assert!(limits.make_board(a) && limits.make_board(a));
        assert!(!limits.make_board(a));
        assert!(limits.make_board(b));
        let held = limits.connect(a).expect("first connection");
        assert!(limits.connect(a).is_none());
        drop(held);
        assert!(limits.connect(a).is_some());
    }

    #[test]
    fn clients_are_named_by_address_or_trusted_proxy() {
        let peer: SocketAddr = "10.0.0.5:4000".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 198.51.100.7".parse().unwrap());
        let open = Limits::default();
        assert_eq!(open.client(peer, &headers), ip("10.0.0.5"));
        let behind = Limits {
            trust_proxy: true,
            ..Limits::default()
        };
        assert_eq!(behind.client(peer, &headers), ip("198.51.100.7"));
        // One IPv6 network is one client.
        let v6: SocketAddr = "[2001:db8:1:2:3:4:5:6]:9".parse().unwrap();
        assert_eq!(open.client(v6, &HeaderMap::new()), ip("2001:db8:1:2::"));
    }
}
