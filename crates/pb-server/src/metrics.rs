//! Counts of what the server does, for monitoring: `GET /metrics` in Prometheus' text
//! format when `PB_METRICS` is set. Only numbers, nothing about what is on the boards.

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    /// People connected to boards now.
    pub connected: AtomicU64,
    pub joins: AtomicU64,
    /// Connections refused: wrong link, unknown board, bad greeting.
    pub refused: AtomicU64,
    pub messages: AtomicU64,
    /// Bytes of board changes and pictures taken in.
    pub bytes_in: AtomicU64,
    /// Connections cut for a bad message or sending too fast.
    pub cut_off: AtomicU64,
    pub boards_made: AtomicU64,
    /// Requests refused by the limits per address.
    pub limited: AtomicU64,
    pub previews: AtomicU64,
}

/// Adds one to `counter`.
pub fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

impl Metrics {
    /// The text Prometheus reads, with the board count and live rooms known elsewhere.
    pub fn render(&self, boards: u64, rooms: u64) -> String {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let rows: [(&str, &str, &str, u64); 11] = [
            ("boards", "gauge", "Boards kept", boards),
            ("rooms", "gauge", "Boards open in memory", rooms),
            (
                "connected",
                "gauge",
                "People connected to boards",
                get(&self.connected),
            ),
            (
                "joins_total",
                "counter",
                "Connections to boards",
                get(&self.joins),
            ),
            (
                "refused_total",
                "counter",
                "Connections refused",
                get(&self.refused),
            ),
            (
                "messages_total",
                "counter",
                "Messages taken in",
                get(&self.messages),
            ),
            (
                "received_bytes_total",
                "counter",
                "Bytes of changes and pictures",
                get(&self.bytes_in),
            ),
            (
                "cut_off_total",
                "counter",
                "Connections cut for bad or too many messages",
                get(&self.cut_off),
            ),
            (
                "boards_made_total",
                "counter",
                "Boards made",
                get(&self.boards_made),
            ),
            (
                "limited_total",
                "counter",
                "Requests over the limits per address",
                get(&self.limited),
            ),
            (
                "previews_total",
                "counter",
                "Link previews fetched",
                get(&self.previews),
            ),
        ];
        let mut out = String::new();
        for (name, kind, help, value) in rows {
            let _ = writeln!(out, "# HELP paintboard_{name} {help}.");
            let _ = writeln!(out, "# TYPE paintboard_{name} {kind}");
            let _ = writeln!(out, "paintboard_{name} {value}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_as_prometheus_text() {
        let metrics = Metrics::default();
        bump(&metrics.joins);
        bump(&metrics.joins);
        let text = metrics.render(5, 1);
        assert!(text.contains("# TYPE paintboard_joins_total counter\npaintboard_joins_total 2\n"));
        assert!(text.contains("paintboard_boards 5\n"));
        assert!(
            text.lines()
                .all(|l| l.starts_with('#') || l.split(' ').count() == 2)
        );
    }
}
