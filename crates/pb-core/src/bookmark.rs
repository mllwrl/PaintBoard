//! Views of the board saved in numbered slots, to go back to (see
//! [`crate::Command::SetBookmark`]).

/// A saved view: where the middle of the screen was (world) and the zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bookmark {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Bookmark {
    /// The slots there are, as numbered for people (the keys 1…9).
    pub const SLOTS: std::ops::RangeInclusive<u8> = 1..=9;

    /// Stored as `x;y;zoom`.
    pub(crate) fn to_value(self) -> String {
        format!("{};{};{}", self.x, self.y, self.zoom)
    }

    pub(crate) fn from_value(s: &str) -> Option<Self> {
        let mut parts = s
            .split(';')
            .map(|p| p.parse::<f64>().ok().filter(|v| v.is_finite()));
        let (x, y, zoom) = (parts.next()??, parts.next()??, parts.next()??);
        (zoom > 0.0).then_some(Self { x, y, zoom })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_form_round_trips_and_rejects_nonsense() {
        let view = Bookmark {
            x: -120.5,
            y: 3e6,
            zoom: 0.125,
        };
        assert_eq!(Bookmark::from_value(&view.to_value()), Some(view));
        for bad in ["", "1;2", "a;2;3", "1;2;0", "1;2;-1", "NaN;2;3", "1;2;inf"] {
            assert_eq!(Bookmark::from_value(bad), None, "{bad}");
        }
    }
}
