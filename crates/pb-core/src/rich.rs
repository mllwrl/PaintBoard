//! Inline text styles: runs of bold, italic, underlined, struck-through or colored text.
//!
//! Text keeps its characters as a plain string; styles are [`Span`]s over byte ranges of
//! it. Spans are kept normalized: sorted, not overlapping, not empty, not plain, and
//! neighbors with equal marks merged, so equal styling always compares equal.

use std::ops::Range;
use std::sync::Arc;

use crate::Rgba;

/// How a run of text looks, on top of its object's text style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextMarks {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// `None` keeps the object's text color.
    pub color: Option<Rgba>,
}

impl TextMarks {
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }
}

/// Marks over a byte range of a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub marks: TextMarks,
}

impl Span {
    pub fn range(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// The styled runs of a text; empty for plain text.
pub type Spans = Arc<[Span]>;

/// One flag per mark in the stored form, see [`to_bytes`].
const BOLD: u32 = 1;
const ITALIC: u32 = 2;
const UNDERLINE: u32 = 4;
const STRIKE: u32 = 8;
const COLOR: u32 = 16;

/// Marks of the character starting at byte `index`; plain past the end.
pub fn marks_at(spans: &[Span], index: usize) -> TextMarks {
    spans
        .iter()
        .find(|s| s.start <= index && index < s.end)
        .map_or_else(TextMarks::default, |s| s.marks)
}

/// Marks for text typed at `index`: those of the character before it, like most editors,
/// or of the first character at the very start.
pub fn marks_for_typing(spans: &[Span], index: usize) -> TextMarks {
    if index == 0 {
        marks_at(spans, 0)
    } else {
        marks_at(spans, index - 1)
    }
}

/// Whether every character in `range` has marks `f` agrees with; an empty range asks
/// about the character before it.
pub fn all(spans: &[Span], range: Range<usize>, f: impl Fn(&TextMarks) -> bool) -> bool {
    if range.is_empty() {
        return f(&marks_for_typing(spans, range.start));
    }
    pieces(spans, range.end)
        .into_iter()
        .filter(|p| p.start < range.end && p.end > range.start)
        .all(|p| f(&p.marks))
}

/// `spans` with `change` applied to the marks of `range`, in a text `len` bytes long.
pub fn apply(
    spans: &[Span],
    range: Range<usize>,
    len: usize,
    change: impl Fn(TextMarks) -> TextMarks,
) -> Vec<Span> {
    let range = range.start.min(len)..range.end.min(len);
    let mut out = Vec::new();
    for piece in pieces(spans, len) {
        // Split the piece at the range's ends.
        let cuts = [piece.start, range.start, range.end, piece.end];
        let mut points: Vec<usize> = cuts
            .into_iter()
            .filter(|&p| p >= piece.start && p <= piece.end)
            .collect();
        points.sort_unstable();
        points.dedup();
        for pair in points.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            let inside = start >= range.start && end <= range.end;
            let marks = if inside {
                change(piece.marks)
            } else {
                piece.marks
            };
            out.push(Span { start, end, marks });
        }
    }
    normalize(out)
}

/// `spans` after the bytes in `replaced` were replaced by `inserted` new bytes, which
/// take `marks`.
pub fn edited(
    spans: &[Span],
    replaced: Range<usize>,
    inserted: usize,
    marks: TextMarks,
) -> Vec<Span> {
    let (start, end) = (replaced.start, replaced.end);
    // Bytes after the replaced range move to after the new text.
    let after = |p: usize| p - end + start + inserted;
    let mut out = Vec::with_capacity(spans.len() + 2);
    for s in spans {
        if s.start < start {
            out.push(Span {
                start: s.start,
                end: s.end.min(start),
                marks: s.marks,
            });
        }
        if s.end > end {
            out.push(Span {
                start: after(s.start.max(end)),
                end: after(s.end),
                marks: s.marks,
            });
        }
    }
    out.push(Span {
        start,
        end: start + inserted,
        marks,
    });
    normalize(out)
}

/// The spans within `range`, relative to its start (for copying part of a text).
pub fn slice(spans: &[Span], range: Range<usize>) -> Vec<Span> {
    let out = spans
        .iter()
        .filter(|s| s.start < range.end && s.end > range.start)
        .map(|s| Span {
            start: s.start.max(range.start) - range.start,
            end: s.end.min(range.end) - range.start,
            marks: s.marks,
        })
        .collect();
    normalize(out)
}

/// Sorted, merged, without empty or plain spans.
pub fn normalize(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|s| s.end > s.start && !s.marks.is_plain());
    spans.sort_by_key(|s| s.start);
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        match out.last_mut() {
            Some(last) if last.end == s.start && last.marks == s.marks => last.end = s.end,
            _ => out.push(s),
        }
    }
    out
}

/// The text split into consecutive pieces covering `0..len`, plain ones included.
fn pieces(spans: &[Span], len: usize) -> Vec<Span> {
    let mut out = Vec::new();
    let mut at = 0;
    for s in spans {
        let (start, end) = (s.start.min(len), s.end.min(len));
        if start > at {
            out.push(Span {
                start: at,
                end: start,
                marks: TextMarks::default(),
            });
        }
        if end > start {
            out.push(Span {
                start,
                end,
                marks: s.marks,
            });
        }
        at = at.max(end);
    }
    if len > at {
        out.push(Span {
            start: at,
            end: len,
            marks: TextMarks::default(),
        });
    }
    out
}

/// Spans that fit `text`: inside it and on character boundaries. Stored spans can be off
/// after a concurrent edit of the text.
pub fn fit_to(spans: &[Span], text: &str) -> Vec<Span> {
    let floor = |mut i: usize| {
        i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let out = spans
        .iter()
        .map(|s| Span {
            start: floor(s.start),
            end: floor(s.end),
            marks: s.marks,
        })
        .collect();
    normalize(out)
}

/// Stored form: little-endian `u32` quadruples `start, end, flags, color`.
pub(crate) fn to_bytes(spans: &[Span]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(spans.len() * 16);
    for s in spans {
        let m = s.marks;
        let flags = [
            (m.bold, BOLD),
            (m.italic, ITALIC),
            (m.underline, UNDERLINE),
            (m.strike, STRIKE),
            (m.color.is_some(), COLOR),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(0, |acc, (_, bit)| acc | bit);
        let color = m.color.map_or(0, |c| c.0);
        for v in [s.start as u32, s.end as u32, flags, color] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}

pub(crate) fn from_bytes(bytes: &[u8]) -> Vec<Span> {
    let (chunks, _) = bytes.as_chunks::<16>();
    let spans = chunks
        .iter()
        .map(|c| {
            let v = |i: usize| u32::from_le_bytes(c[i..i + 4].try_into().expect("4 bytes"));
            let flags = v(8);
            Span {
                start: v(0) as usize,
                end: v(4) as usize,
                marks: TextMarks {
                    bold: flags & BOLD != 0,
                    italic: flags & ITALIC != 0,
                    underline: flags & UNDERLINE != 0,
                    strike: flags & STRIKE != 0,
                    color: (flags & COLOR != 0).then_some(Rgba(v(12))),
                },
            }
        })
        .collect();
    normalize(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOLD_MARKS: TextMarks = TextMarks {
        bold: true,
        italic: false,
        underline: false,
        strike: false,
        color: None,
    };

    fn bold(start: usize, end: usize) -> Span {
        Span {
            start,
            end,
            marks: BOLD_MARKS,
        }
    }

    fn set_bold(m: TextMarks) -> TextMarks {
        TextMarks { bold: true, ..m }
    }

    #[test]
    fn apply_splits_and_merges() {
        // "hello world": bold "llo w", then the rest of "world".
        let spans = apply(&[], 2..7, 11, set_bold);
        assert_eq!(spans, vec![bold(2, 7)]);
        let spans = apply(&spans, 7..11, 11, set_bold);
        assert_eq!(spans, vec![bold(2, 11)]);
        let unbold = |m: TextMarks| TextMarks { bold: false, ..m };
        assert_eq!(
            apply(&spans, 4..6, 11, unbold),
            vec![bold(2, 4), bold(6, 11)]
        );
        assert!(all(&spans, 3..5, |m| m.bold));
        assert!(!all(&spans, 0..5, |m| m.bold));
        // Italic over part of the bold keeps both.
        let italic = apply(&spans, 0..4, 11, |m| TextMarks { italic: true, ..m });
        assert_eq!(italic.len(), 3);
        assert_eq!(
            marks_at(&italic, 3),
            TextMarks {
                italic: true,
                ..BOLD_MARKS
            }
        );
        assert!(!marks_at(&italic, 1).bold);
    }

    #[test]
    fn edits_shift_and_extend_spans() {
        let spans = vec![bold(2, 5)];
        // Typing inside the bold run, in bold, grows it.
        assert_eq!(edited(&spans, 3..3, 2, BOLD_MARKS), vec![bold(2, 7)]);
        // Plain text before it pushes it along.
        assert_eq!(
            edited(&spans, 0..0, 3, TextMarks::default()),
            vec![bold(5, 8)]
        );
        // Plain text right at its start goes before it.
        assert_eq!(
            edited(&spans, 2..2, 1, TextMarks::default()),
            vec![bold(3, 6)]
        );
        // Plain text inside splits it.
        assert_eq!(
            edited(&spans, 3..3, 1, TextMarks::default()),
            vec![bold(2, 3), bold(4, 6)]
        );
        // Deleting across its start trims it.
        assert_eq!(
            edited(&spans, 1..3, 0, TextMarks::default()),
            vec![bold(1, 3)]
        );
        // Replacing all of it with plain text removes it.
        assert_eq!(edited(&spans, 2..5, 1, TextMarks::default()), vec![]);
        assert_eq!(marks_for_typing(&spans, 5), BOLD_MARKS);
        assert_eq!(marks_for_typing(&spans, 2), TextMarks::default());
    }

    #[test]
    fn roundtrip_slice_and_fit() {
        let red = Rgba::rgb(0xF2, 0x55, 0x55);
        let spans = vec![
            bold(0, 2),
            Span {
                start: 4,
                end: 6,
                marks: TextMarks {
                    underline: true,
                    strike: true,
                    color: Some(red),
                    ..TextMarks::default()
                },
            },
        ];
        assert_eq!(from_bytes(&to_bytes(&spans)), spans);
        assert_eq!(
            slice(&spans, 1..5),
            vec![
                bold(0, 1),
                Span {
                    start: 3,
                    end: 4,
                    ..spans[1]
                }
            ]
        );
        // "дом" is 6 bytes; a span ending mid-character is pulled back.
        assert_eq!(fit_to(&[bold(0, 3)], "дом"), vec![bold(0, 2)]);
        assert_eq!(fit_to(&[bold(4, 10)], "дом"), vec![bold(4, 6)]);
    }
}
