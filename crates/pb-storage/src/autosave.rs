use pb_platform::{Duration, Instant};

/// Writes are due this long after editing pauses...
const IDLE_DELAY: Duration = Duration::from_millis(1000);
/// ...or this long after the first unsaved edit, even if editing goes on.
const MAX_DELAY: Duration = Duration::from_secs(10);

/// Decides when to write the board after edits.
///
/// It only tracks document revisions and time; the caller does the writing, and tells it
/// with [`Autosave::saved`].
#[derive(Debug)]
pub struct Autosave {
    saved_revision: u64,
    seen_revision: u64,
    first_change: Option<Instant>,
    last_change: Option<Instant>,
}

impl Autosave {
    /// Starts out clean at `revision`.
    pub fn new(revision: u64) -> Self {
        Self {
            saved_revision: revision,
            seen_revision: revision,
            first_change: None,
            last_change: None,
        }
    }

    /// Call whenever the document may have changed.
    pub fn observe(&mut self, revision: u64, now: Instant) {
        if revision == self.seen_revision {
            return;
        }
        self.seen_revision = revision;
        if revision == self.saved_revision {
            // Undone back to the saved state.
            self.first_change = None;
            self.last_change = None;
            return;
        }
        self.first_change.get_or_insert(now);
        self.last_change = Some(now);
    }

    pub fn is_dirty(&self) -> bool {
        self.first_change.is_some()
    }

    /// When the next write is due, if anything is unsaved.
    pub fn deadline(&self) -> Option<Instant> {
        let idle = self.last_change? + IDLE_DELAY;
        let max = self.first_change? + MAX_DELAY;
        Some(idle.min(max))
    }

    pub fn is_due(&self, now: Instant) -> bool {
        self.deadline().is_some_and(|d| now >= d)
    }

    /// Call after writing the document at `revision`.
    pub fn saved(&mut self, revision: u64) {
        *self = Self::new(revision);
    }

    /// Call after a failed write: try again after the next pause, not in a tight loop.
    pub fn failed(&mut self, now: Instant) {
        if self.is_dirty() {
            self.first_change = Some(now);
            self.last_change = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn saves_after_editing_pauses() {
        let t = Instant::now();
        let mut a = Autosave::new(1);
        a.observe(1, t);
        assert!(!a.is_dirty() && a.deadline().is_none());
        a.observe(2, t);
        a.observe(3, t + ms(500));
        assert_eq!(a.deadline(), Some(t + ms(1500)));
        assert!(!a.is_due(t + ms(1400)));
        assert!(a.is_due(t + ms(1500)));
        a.saved(3);
        assert!(!a.is_dirty());
    }

    #[test]
    fn continuous_editing_still_saves() {
        let t = Instant::now();
        let mut a = Autosave::new(0);
        for i in 1..=30 {
            a.observe(i, t + ms(500 * i));
        }
        assert_eq!(a.deadline(), Some(t + ms(500) + MAX_DELAY));
    }

    #[test]
    fn undo_to_saved_state_is_clean() {
        let t = Instant::now();
        let mut a = Autosave::new(5);
        a.observe(6, t);
        assert!(a.is_dirty());
        a.observe(5, t);
        assert!(!a.is_dirty());
    }

    #[test]
    fn failure_backs_off() {
        let t = Instant::now();
        let mut a = Autosave::new(0);
        a.observe(1, t);
        a.failed(t + ms(2000));
        assert!(!a.is_due(t + ms(2500)));
        assert!(a.is_due(t + ms(3000)));
    }
}
