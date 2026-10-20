//! A countdown on the board, which everyone on a shared board sees (see
//! [`crate::Command::SetTimer`]).
//!
//! Times are wall-clock milliseconds since the Unix epoch, so copies of a shared board on
//! different computers count down together.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timer {
    /// How long it counts down, in milliseconds.
    pub duration: u64,
    /// When it last started running; `None` while paused.
    pub started: Option<i64>,
    /// Milliseconds run before the last start.
    pub run: u64,
}

impl Timer {
    /// A countdown of `duration` milliseconds, running from `now`.
    pub fn new(duration: u64, now: i64) -> Self {
        Self {
            duration,
            started: Some(now),
            run: 0,
        }
    }

    pub fn running(&self) -> bool {
        self.started.is_some()
    }

    /// Milliseconds run by `now`; a clock behind the starter's counts no time yet.
    pub fn elapsed(&self, now: i64) -> u64 {
        self.run
            + self
                .started
                .map_or(0, |s| now.saturating_sub(s).max(0) as u64)
    }

    /// Milliseconds left at `now`; 0 once time is up.
    pub fn left(&self, now: i64) -> u64 {
        self.duration.saturating_sub(self.elapsed(now))
    }

    /// Paused at `now`, or running on from there.
    pub fn toggled(&self, now: i64) -> Self {
        match self.started {
            Some(_) => Self {
                run: self.elapsed(now),
                started: None,
                ..*self
            },
            None => Self {
                started: Some(now),
                ..*self
            },
        }
    }

    /// Stored as `duration;run;started`, `started` empty while paused.
    pub(crate) fn to_value(self) -> String {
        let started = self.started.map(|s| s.to_string()).unwrap_or_default();
        format!("{};{};{started}", self.duration, self.run)
    }

    pub(crate) fn from_value(s: &str) -> Option<Self> {
        let mut parts = s.split(';');
        let duration = parts.next()?.parse().ok()?;
        let run = parts.next()?.parse().ok()?;
        let started = match parts.next()? {
            "" => None,
            s => Some(s.parse().ok()?),
        };
        Some(Self {
            duration,
            started,
            run,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_down_pauses_and_runs_on() {
        let timer = Timer::new(60_000, 1_000);
        assert_eq!(timer.left(1_000), 60_000);
        assert_eq!(timer.left(11_000), 50_000);
        // A clock a little behind the one that started it.
        assert_eq!(timer.left(900), 60_000);
        let paused = timer.toggled(21_000);
        assert!(!paused.running());
        assert_eq!(paused.left(99_000), 40_000);
        let resumed = paused.toggled(100_000);
        assert_eq!(resumed.left(130_000), 10_000);
        assert_eq!(resumed.left(200_000), 0);
        for t in [timer, paused, resumed] {
            assert_eq!(Timer::from_value(&t.to_value()), Some(t));
        }
        assert_eq!(Timer::from_value("garbage"), None);
    }
}
