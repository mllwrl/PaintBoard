//! Talktracks: a walk through the board recorded with the presenter's voice, replayed as
//! it was, the view moving and the pointer showing where they pointed.

use std::sync::Arc;

use kurbo::Point;

use crate::AssetId;

/// Where the view and the pointer were at a moment of a talktrack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    /// Milliseconds from the start.
    pub at: u32,
    /// The middle of the view (world).
    pub center: Point,
    pub zoom: f64,
    /// The pointer (world), when it was over the board.
    pub pointer: Option<Point>,
}

/// A recorded talktrack.
#[derive(Clone, Debug, PartialEq)]
pub struct Talktrack {
    pub id: Arc<str>,
    pub title: Arc<str>,
    pub author: Arc<str>,
    /// When it was recorded, in milliseconds since the Unix epoch.
    pub created: i64,
    /// How long it lasts, in milliseconds.
    pub duration: u32,
    /// The voice (see `pb_io::audio`), if a microphone was there.
    pub audio: Option<AssetId>,
    /// The view over time, by `at`.
    pub keys: Arc<[Key]>,
}

/// The stored form's version.
const VERSION: u8 = 1;

impl Talktrack {
    /// The view and pointer at `at` milliseconds: in between keys the view moves on
    /// smoothly (zoom on a log scale), the pointer jumps.
    pub fn view_at(&self, at: u32) -> Option<(Point, f64, Option<Point>)> {
        let i = self.keys.partition_point(|k| k.at <= at);
        let Some(before) = i.checked_sub(1).map(|i| self.keys[i]) else {
            return self.keys.first().map(|k| (k.center, k.zoom, k.pointer));
        };
        let Some(after) = self.keys.get(i) else {
            return Some((before.center, before.zoom, before.pointer));
        };
        let t = f64::from(at - before.at) / f64::from((after.at - before.at).max(1));
        let center = before.center.lerp(after.center, t);
        let zoom = (before.zoom.ln() * (1.0 - t) + after.zoom.ln() * t).exp();
        Some((center, zoom, before.pointer))
    }

    /// The stored form, see [`Self::from_bytes`].
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![VERSION];
        for text in [&self.title, &self.author] {
            out.extend_from_slice(&(text.len() as u32).to_le_bytes());
            out.extend_from_slice(text.as_bytes());
        }
        out.extend_from_slice(&self.created.to_le_bytes());
        out.extend_from_slice(&self.duration.to_le_bytes());
        match self.audio {
            Some(id) => {
                out.push(1);
                out.extend_from_slice(id.as_bytes());
            }
            None => out.push(0),
        }
        out.extend_from_slice(&(self.keys.len() as u32).to_le_bytes());
        for key in self.keys.iter() {
            out.extend_from_slice(&key.at.to_le_bytes());
            for v in [key.center.x, key.center.y, key.zoom] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            match key.pointer {
                Some(p) => {
                    out.push(1);
                    out.extend_from_slice(&p.x.to_le_bytes());
                    out.extend_from_slice(&p.y.to_le_bytes());
                }
                None => out.push(0),
            }
        }
        out
    }

    /// Reads the stored form: version, title and author (length-prefixed UTF-8), created,
    /// duration, audio (flag and hash), then the keys, all little-endian.
    pub fn from_bytes(id: &str, bytes: &[u8]) -> Option<Self> {
        let mut r = Reader(bytes);
        if r.take(1)?[0] != VERSION {
            return None;
        }
        let mut text = || -> Option<Arc<str>> {
            let len = r.u32()? as usize;
            std::str::from_utf8(r.take(len)?).ok().map(Into::into)
        };
        let (title, author) = (text()?, text()?);
        let created = i64::from_le_bytes(r.take(8)?.try_into().ok()?);
        let duration = r.u32()?;
        let audio = match r.take(1)?[0] {
            0 => None,
            _ => Some(AssetId::from_bytes(r.take(32)?.try_into().ok()?)),
        };
        let count = r.u32()? as usize;
        let mut keys = Vec::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            let at = r.u32()?;
            let (x, y, zoom) = (r.f64()?, r.f64()?, r.f64()?);
            let pointer = match r.take(1)?[0] {
                0 => None,
                _ => Some(Point::new(r.f64()?, r.f64()?)),
            };
            keys.push(Key {
                at,
                center: Point::new(x, y),
                zoom,
                pointer,
            });
        }
        Some(Self {
            id: id.into(),
            title,
            author,
            created,
            duration,
            audio,
            keys: keys.into(),
        })
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = rest;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Talktrack {
        let key = |at, x, zoom, pointer| Key {
            at,
            center: Point::new(x, 0.0),
            zoom,
            pointer,
        };
        Talktrack {
            id: "t1".into(),
            title: "Walkthrough".into(),
            author: "Ann".into(),
            created: 1_700_000_000_000,
            duration: 3000,
            audio: Some(AssetId::of(b"voice")),
            keys: vec![
                key(0, 0.0, 1.0, None),
                key(1000, 100.0, 4.0, Some(Point::new(5.0, 6.0))),
            ]
            .into(),
        }
    }

    #[test]
    fn round_trips_through_bytes() {
        let t = track();
        assert_eq!(Talktrack::from_bytes("t1", &t.to_bytes()), Some(t.clone()));
        assert_eq!(Talktrack::from_bytes("t1", &t.to_bytes()[..20]), None);
    }

    #[test]
    fn view_moves_between_keys_and_holds_after() {
        let t = track();
        let (center, zoom, pointer) = t.view_at(500).unwrap();
        assert_eq!(center, Point::new(50.0, 0.0));
        assert!((zoom - 2.0).abs() < 1e-9, "halfway on a log scale");
        assert_eq!(pointer, None);
        assert_eq!(
            t.view_at(2500),
            Some((Point::new(100.0, 0.0), 4.0, Some(Point::new(5.0, 6.0))))
        );
    }
}
