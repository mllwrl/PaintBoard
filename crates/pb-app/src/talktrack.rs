//! Recording a talktrack (the view, the pointer and the voice) and playing one back.

use std::sync::Arc;

use pb_core::talktrack::Key;
use pb_core::{Document, Talktrack};
use pb_geom::Camera;
use pb_geom::kurbo::Point;
use pb_platform::audio::{Player, Recorder};
use pb_platform::{Duration, Instant};

/// How often the view is sampled while recording.
pub const KEY_EVERY: Duration = Duration::from_millis(50);

/// A talktrack being recorded.
pub struct Recording {
    started: Instant,
    mic: Option<Recorder>,
    keys: Vec<Key>,
    next_key: Instant,
    /// The seconds last shown.
    shown: u64,
}

impl Recording {
    /// Starts recording; without a microphone only the view is recorded.
    pub fn start(now: Instant) -> Self {
        let mic = Recorder::start()
            .map_err(|err| log::warn!("talktrack without sound: {err}"))
            .ok();
        Self {
            started: now,
            mic,
            keys: Vec::new(),
            next_key: now,
            shown: 0,
        }
    }

    /// Whether the seconds shown are out of date (and now taken as shown).
    pub fn second_passed(&mut self, now: Instant) -> bool {
        let seconds = self.seconds(now);
        let passed = seconds != self.shown;
        self.shown = seconds;
        passed
    }

    pub fn has_sound(&self) -> bool {
        self.mic.is_some()
    }

    /// Whole seconds recorded.
    pub fn seconds(&self, now: Instant) -> u64 {
        now.duration_since(self.started).as_secs()
    }

    /// When the view should next be sampled.
    pub fn next_key(&self) -> Instant {
        self.next_key
    }

    /// Notes where the view and the pointer are, if it is time to.
    pub fn sample(&mut self, now: Instant, center: Point, zoom: f64, pointer: Option<Point>) {
        if now < self.next_key {
            return;
        }
        self.next_key = now + KEY_EVERY;
        self.keys.push(Key {
            at: millis(now.duration_since(self.started)),
            center,
            zoom,
            pointer,
        });
    }

    /// Stops recording: the talktrack by `author`, its voice stored in `doc`'s assets.
    pub fn finish(
        self,
        now: Instant,
        doc: &mut Document,
        id: Arc<str>,
        title: String,
        author: &str,
    ) -> Talktrack {
        let duration = millis(now.duration_since(self.started));
        let audio = self.mic.map(Recorder::finish).and_then(|clip| {
            (!clip.samples.is_empty())
                .then(|| doc.add_asset(pb_io::audio::encode(&clip.samples, clip.rate)))
        });
        Talktrack {
            id,
            title: title.into(),
            author: author.into(),
            created: pb_platform::now_ms(),
            duration,
            audio,
            keys: self.keys.into(),
        }
    }
}

fn millis(d: Duration) -> u32 {
    d.as_millis().min(u128::from(u32::MAX)) as u32
}

/// A talktrack being played back.
pub struct Playback {
    pub track: Talktrack,
    /// Where the board was before, to go back to.
    pub saved: Camera,
    sound: Option<(Arc<[i16]>, u32)>,
    player: Option<Player>,
    /// The clock without sound: where playing started from, and when.
    from: (f64, Instant),
    /// Seconds in, while paused.
    paused: Option<f64>,
}

impl Playback {
    pub fn start(track: Talktrack, doc: &Document, saved: Camera, now: Instant) -> Self {
        let sound = track
            .audio
            .and_then(|id| doc.assets().get(id))
            .and_then(|bytes| pb_io::audio::decode(bytes))
            .map(|(samples, rate)| (Arc::<[i16]>::from(samples), rate));
        let mut playback = Self {
            track,
            saved,
            sound,
            player: None,
            from: (0.0, now),
            paused: None,
        };
        playback.play_from(0.0, now);
        playback
    }

    fn play_from(&mut self, seconds: f64, now: Instant) {
        self.from = (seconds, now);
        self.paused = None;
        match &self.player {
            Some(player) => {
                player.seek(seconds);
                player.set_playing(true);
            }
            None => {
                self.player = self.sound.as_ref().and_then(|(samples, rate)| {
                    Player::start(samples.clone(), *rate, seconds)
                        .map_err(|err| log::warn!("talktrack sound: {err}"))
                        .ok()
                });
            }
        }
    }

    /// Seconds the talktrack lasts.
    pub fn length(&self) -> f64 {
        f64::from(self.track.duration) / 1000.0
    }

    /// Seconds in: the sound's position when there is sound, else the clock's.
    pub fn position(&self, now: Instant) -> f64 {
        let at = match (self.paused, &self.player) {
            (Some(at), _) => at,
            (None, Some(player)) => player.position(),
            (None, None) => self.from.0 + now.duration_since(self.from.1).as_secs_f64(),
        };
        at.min(self.length())
    }

    pub fn is_paused(&self) -> bool {
        self.paused.is_some()
    }

    pub fn is_over(&self, now: Instant) -> bool {
        self.position(now) >= self.length()
    }

    /// Pauses, or plays on (from the start once it is over).
    pub fn toggle(&mut self, now: Instant) {
        match self.paused {
            Some(at) => {
                let at = if at >= self.length() { 0.0 } else { at };
                self.play_from(at, now);
            }
            None => {
                let at = self.position(now);
                if at >= self.length() {
                    self.play_from(0.0, now);
                    return;
                }
                if let Some(player) = &self.player {
                    player.set_playing(false);
                }
                self.paused = Some(at);
            }
        }
    }

    /// Goes to `seconds` in, keeping it paused if it is.
    pub fn seek(&mut self, seconds: f64, now: Instant) {
        let seconds = seconds.clamp(0.0, self.length());
        if self.paused.is_some() {
            self.paused = Some(seconds);
            if let Some(player) = &self.player {
                player.seek(seconds);
            }
        } else {
            self.play_from(seconds, now);
        }
    }

    /// The view and the pointer now.
    pub fn view(&self, now: Instant) -> Option<(Point, f64, Option<Point>)> {
        let at = (self.position(now) * 1000.0) as u32;
        self.track.view_at(at)
    }
}
