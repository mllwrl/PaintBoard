//! Rich text: inline styles, links and paragraph styles (lists) over a plain string.
//!
//! Text keeps its characters as a plain string; styles are [`Span`]s over byte ranges of
//! it, links are [`LinkSpan`]s the same way, and paragraphs (the text between line breaks)
//! can be list items, see [`Para`]. All three are kept normalized: sorted, not
//! overlapping, not empty, not plain, and neighbors with equal marks merged, so equal
//! styling always compares equal. [`Spans`] holds the three together.
//!
//! They are stored apart (see `Document`): older versions of the app read the inline
//! styles and show links and lists as plain text.

use std::ops::{Deref, Range};
use std::sync::Arc;

use crate::{Link, Rgba};

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

/// Marks over a byte range of a text: inline styles ([`TextMarks`]) or a link (see
/// [`LinkSpan`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span<M = TextMarks> {
    pub start: usize,
    pub end: usize,
    pub marks: M,
}

impl<M> Span<M> {
    pub fn range(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// A link over a byte range of a text; normalized link spans are never `None`.
pub type LinkSpan = Span<Option<Link>>;

/// What a list paragraph starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// A bullet: •, ◦ or ▪ by level.
    Bullet,
    /// A number counting the items of its level: 1., a. or i. by level.
    Number,
}

/// How a paragraph is set: plain, or a list item at a nesting level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParaStyle {
    pub list: Option<ListKind>,
    /// Nesting level of a list item, from 0; plain paragraphs have 0.
    pub level: u8,
}

impl ParaStyle {
    /// The deepest nesting level.
    pub const MAX_LEVEL: u8 = 5;

    pub fn list(kind: ListKind, level: u8) -> Self {
        Self {
            list: Some(kind),
            level: level.min(Self::MAX_LEVEL),
        }
    }

    pub fn is_plain(&self) -> bool {
        self.list.is_none()
    }
}

/// The style of paragraph `index` (counted from 0, paragraphs being separated by `\n`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Para {
    pub index: usize,
    pub style: ParaStyle,
}

/// The rich formatting of a text: inline styles, links and paragraph styles; empty for
/// plain text. Dereferences to the inline styles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Spans {
    pub marks: Arc<[Span]>,
    pub links: Arc<[LinkSpan]>,
    pub paras: Arc<[Para]>,
}

impl Deref for Spans {
    type Target = [Span];

    fn deref(&self) -> &[Span] {
        &self.marks
    }
}

/// One flag per mark in the stored form, see [`to_bytes`].
const BOLD: u32 = 1;
const ITALIC: u32 = 2;
const UNDERLINE: u32 = 4;
const STRIKE: u32 = 8;
const COLOR: u32 = 16;

impl Spans {
    /// Inline styles only.
    pub fn from_marks(marks: Vec<Span>) -> Self {
        Self {
            marks: normalize(marks).into(),
            ..Self::default()
        }
    }

    /// From all three parts, normalized.
    pub fn new(marks: Vec<Span>, links: Vec<LinkSpan>, paras: Vec<Para>) -> Self {
        Self {
            marks: normalize(marks).into(),
            links: normalize(links).into(),
            paras: normalize_paras(paras).into(),
        }
    }

    /// No styles, links or lists: plain text.
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty() && self.links.is_empty() && self.paras.is_empty()
    }

    /// The same with other inline styles.
    pub fn with_marks(&self, marks: Vec<Span>) -> Self {
        Self {
            marks: normalize(marks).into(),
            ..self.clone()
        }
    }

    /// The same with other links.
    pub fn with_links(&self, links: Vec<LinkSpan>) -> Self {
        Self {
            links: normalize(links).into(),
            ..self.clone()
        }
    }

    /// The same with other paragraph styles.
    pub fn with_paras(&self, paras: Vec<Para>) -> Self {
        Self {
            paras: normalize_paras(paras).into(),
            ..self.clone()
        }
    }

    /// The link over the character starting at byte `index`, if any.
    pub fn link_at(&self, index: usize) -> Option<&Link> {
        self.links
            .iter()
            .find(|s| s.start <= index && index < s.end)
            .and_then(|s| s.marks.as_ref())
    }

    /// The style of paragraph `index`.
    pub fn para(&self, index: usize) -> ParaStyle {
        para_style(&self.paras, index)
    }

    /// Styles that fit `text`: inside it, on character boundaries, and lists only on
    /// paragraphs it has. Stored styles can be off after a concurrent edit of the text.
    pub fn fit_to(&self, text: &str) -> Self {
        let count = paragraph_count(text);
        Self {
            marks: fit_to(&self.marks, text).into(),
            links: fit_to(&self.links, text).into(),
            paras: self
                .paras
                .iter()
                .filter(|p| p.index < count)
                .copied()
                .collect(),
        }
    }

    /// The styles of `range` of `text`, relative to its start (for copying part of a
    /// text). The first paragraph keeps its style only if `range` starts it.
    pub fn slice(&self, text: &str, range: Range<usize>) -> Self {
        let first = paragraph_of(text, range.start);
        let last = paragraph_of(text, range.end);
        let starts = range.start == 0 || text.as_bytes().get(range.start - 1) == Some(&b'\n');
        let paras = self
            .paras
            .iter()
            .filter(|p| p.index >= first && p.index <= last && (p.index > first || starts))
            .map(|p| Para {
                index: p.index - first,
                style: p.style,
            })
            .collect();
        Self {
            marks: slice(&self.marks, range.clone()).into(),
            links: slice(&self.links, range).into(),
            paras,
        }
    }

    /// The styles after `replaced` bytes of `old` were replaced by `inserted`. Typed text
    /// takes `marks`, and a link only if typed inside it; `pasted` styles (of `inserted`,
    /// see [`Self::slice`]) go over it instead. New paragraphs take the style of the one
    /// they were split from, or the pasted ones'.
    pub fn edited(
        &self,
        old: &str,
        replaced: Range<usize>,
        inserted: &str,
        marks: TextMarks,
        pasted: Option<&Spans>,
    ) -> Self {
        let (start, n) = (replaced.start, inserted.len());
        let len = old.len() - replaced.len() + n;
        let mut new_marks = edited(&self.marks, replaced.clone(), n, marks);
        let before = start.checked_sub(1).and_then(|i| marks_at(&self.links, i));
        let link = before.filter(|b| marks_at(&self.links, replaced.end).as_ref() == Some(b));
        let mut new_links = edited(&self.links, replaced.clone(), n, link);
        if let Some(pasted) = pasted {
            new_marks = apply(&new_marks, start..start + n, len, |_| TextMarks::default());
            new_marks.extend(pasted.marks.iter().map(|s| Span {
                start: s.start + start,
                end: s.end + start,
                marks: s.marks,
            }));
            new_links = apply(&new_links, start..start + n, len, |_| None);
            new_links.extend(pasted.links.iter().map(|s| Span {
                start: s.start + start,
                end: s.end + start,
                marks: s.marks.clone(),
            }));
        }
        let paras = paras_edited(
            &self.paras,
            old,
            replaced,
            inserted,
            pasted.map(|p| &*p.paras),
        );
        Self::new(new_marks, new_links, paras)
    }
}

/// Top-level domains a bare address (`example.com`) is linked with as it is typed; other
/// words with dots (`file.txt`, `v1.2`) are left alone unless they start with `www.`.
const COMMON_DOMAINS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "io", "dev", "app", "ai", "co", "me", "info", "biz", "ru",
    "su", "by", "kz", "ua", "de", "uk", "fr", "it", "es", "nl", "pl", "eu", "us", "ca", "jp", "cn",
    "in", "br", "online", "site", "tech", "pro", "xyz",
];

/// A web address typed or pasted as a word of text: `https://…`, `http://…`, `mailto:…`,
/// `www.…` or a bare address with a common domain. Trailing punctuation is not part of
/// it. Gives the length of the address within `word` and where it links to.
pub fn detect_link(word: &str) -> Option<(usize, Link)> {
    let trimmed =
        word.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\'', '»']);
    let lower = trimmed.to_ascii_lowercase();
    let schemed = ["http://", "https://", "mailto:"]
        .iter()
        .any(|s| lower.starts_with(s) && lower.len() > s.len() + 2);
    let bare = || {
        let host = lower.split(['/', '?', '#', ':']).next().unwrap_or("");
        let domain = host.rsplit('.').next().unwrap_or("");
        host.contains('.')
            && !host.contains('@')
            && (host.starts_with("www.") || COMMON_DOMAINS.contains(&domain))
            && host.split('.').all(|part| {
                !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
    };
    if !schemed && !bare() {
        return None;
    }
    Link::from_input(trimmed).map(|link| (trimmed.len(), link))
}

/// Number of paragraphs in `text`: one more than its line breaks.
pub fn paragraph_count(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count() + 1
}

/// The paragraph byte `index` of `text` is in.
pub fn paragraph_of(text: &str, index: usize) -> usize {
    text.as_bytes()[..index.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

/// Byte ranges of the paragraphs of `text`, without their line breaks.
pub fn paragraph_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            out.push(start..i);
            start = i + 1;
        }
    }
    out.push(start..text.len());
    out
}

/// The style of paragraph `index` in `paras`.
pub fn para_style(paras: &[Para], index: usize) -> ParaStyle {
    paras
        .iter()
        .find(|p| p.index == index)
        .map_or_else(ParaStyle::default, |p| p.style)
}

/// Sorted by paragraph, one per paragraph (the last given wins), without plain ones.
pub fn normalize_paras(paras: Vec<Para>) -> Vec<Para> {
    let mut out: Vec<Para> = Vec::with_capacity(paras.len());
    for p in paras {
        out.retain(|o| o.index != p.index);
        if !p.style.is_plain() {
            out.push(Para {
                index: p.index,
                style: ParaStyle {
                    list: p.style.list,
                    level: p.style.level.min(ParaStyle::MAX_LEVEL),
                },
            });
        }
    }
    out.sort_by_key(|p| p.index);
    out
}

/// `paras` after `replaced` bytes of `old` were replaced by `inserted`, see
/// [`Spans::edited`].
pub fn paras_edited(
    paras: &[Para],
    old: &str,
    replaced: Range<usize>,
    inserted: &str,
    pasted: Option<&[Para]>,
) -> Vec<Para> {
    let p = paragraph_of(old, replaced.start);
    let removed = old[replaced.clone()]
        .bytes()
        .filter(|&b| b == b'\n')
        .count();
    let added = inserted.bytes().filter(|&b| b == b'\n').count();
    // Text from the start of a paragraph into a later one is gone: what is left of the
    // later one keeps its style, like a deleted line takes its bullet along.
    let at_start = replaced.start == 0 || old.as_bytes()[replaced.start - 1] == b'\n';
    let own = para_style(paras, if at_start { p + removed } else { p });
    let mut out: Vec<Para> = paras
        .iter()
        .filter(|para| para.index < p)
        .copied()
        .collect();
    let first = pasted
        .map(|ps| para_style(ps, 0))
        .filter(|s| !s.is_plain())
        .unwrap_or(own);
    out.push(Para {
        index: p,
        style: first,
    });
    for j in 1..=added {
        let style = pasted.map_or(own, |ps| para_style(ps, j));
        out.push(Para {
            index: p + j,
            style,
        });
    }
    out.extend(
        paras
            .iter()
            .filter(|para| para.index > p + removed)
            .map(|para| Para {
                index: para.index - removed + added,
                style: para.style,
            }),
    );
    normalize_paras(out)
}

/// The marker a list paragraph shows: its kind, level and number among its siblings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListMarker {
    pub kind: ListKind,
    pub level: u8,
    /// From 1; bullets count too, though they do not show it.
    pub number: u32,
}

impl ListMarker {
    /// What is drawn before the paragraph: a bullet, or a number (1.), letter (a.) or
    /// roman numeral (i.) by level.
    pub fn text(&self) -> String {
        match self.kind {
            ListKind::Bullet => ["•", "◦", "▪"][usize::from(self.level % 3)].to_owned(),
            ListKind::Number => match self.level % 3 {
                0 => format!("{}.", self.number),
                1 => format!("{}.", letters(self.number)),
                _ => format!("{}.", roman(self.number)),
            },
        }
    }
}

/// 1 → a, 26 → z, 27 → aa.
fn letters(mut n: u32) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'a' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Lowercase roman numerals; plain digits past 3999.
fn roman(mut n: u32) -> String {
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    let table = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, digits) in table {
        while n >= value {
            out += digits;
            n -= value;
        }
    }
    out
}

/// The markers of `count` paragraphs with `paras`: `None` for plain ones. Numbers count
/// the items of a level, start again under each item of the level above, and after a
/// plain paragraph or an item of the other kind.
pub fn list_markers(paras: &[Para], count: usize) -> Vec<Option<ListMarker>> {
    let mut counters: Vec<Option<(ListKind, u32)>> = Vec::new();
    (0..count)
        .map(|i| {
            let style = para_style(paras, i);
            let Some(kind) = style.list else {
                counters.clear();
                return None;
            };
            let level = usize::from(style.level);
            counters.truncate(level + 1);
            counters.resize(level + 1, None);
            let number = match counters[level] {
                Some((k, n)) if k == kind => n + 1,
                _ => 1,
            };
            counters[level] = Some((kind, number));
            Some(ListMarker {
                kind,
                level: style.level,
                number,
            })
        })
        .collect()
}

/// Marks of the character starting at byte `index`; plain past the end.
pub fn marks_at<M: Clone + Default>(spans: &[Span<M>], index: usize) -> M {
    spans
        .iter()
        .find(|s| s.start <= index && index < s.end)
        .map_or_else(M::default, |s| s.marks.clone())
}

/// Marks for text typed at `index`: those of the character before it, like most editors,
/// or of the first character at the very start.
pub fn marks_for_typing<M: Clone + Default>(spans: &[Span<M>], index: usize) -> M {
    if index == 0 {
        marks_at(spans, 0)
    } else {
        marks_at(spans, index - 1)
    }
}

/// Whether every character in `range` has marks `f` agrees with; an empty range asks
/// about the character before it.
pub fn all<M: Clone + Default + PartialEq>(
    spans: &[Span<M>],
    range: Range<usize>,
    f: impl Fn(&M) -> bool,
) -> bool {
    if range.is_empty() {
        return f(&marks_for_typing(spans, range.start));
    }
    pieces(spans, range.end)
        .into_iter()
        .filter(|p| p.start < range.end && p.end > range.start)
        .all(|p| f(&p.marks))
}

/// `spans` with `change` applied to the marks of `range`, in a text `len` bytes long.
pub fn apply<M: Clone + Default + PartialEq>(
    spans: &[Span<M>],
    range: Range<usize>,
    len: usize,
    change: impl Fn(M) -> M,
) -> Vec<Span<M>> {
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
                change(piece.marks.clone())
            } else {
                piece.marks.clone()
            };
            out.push(Span { start, end, marks });
        }
    }
    normalize(out)
}

/// `spans` after the bytes in `replaced` were replaced by `inserted` new bytes, which
/// take `marks`.
pub fn edited<M: Clone + Default + PartialEq>(
    spans: &[Span<M>],
    replaced: Range<usize>,
    inserted: usize,
    marks: M,
) -> Vec<Span<M>> {
    let (start, end) = (replaced.start, replaced.end);
    // Bytes after the replaced range move to after the new text.
    let after = |p: usize| p - end + start + inserted;
    let mut out = Vec::with_capacity(spans.len() + 2);
    for s in spans {
        if s.start < start {
            out.push(Span {
                start: s.start,
                end: s.end.min(start),
                marks: s.marks.clone(),
            });
        }
        if s.end > end {
            out.push(Span {
                start: after(s.start.max(end)),
                end: after(s.end),
                marks: s.marks.clone(),
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
pub fn slice<M: Clone + Default + PartialEq>(
    spans: &[Span<M>],
    range: Range<usize>,
) -> Vec<Span<M>> {
    let out = spans
        .iter()
        .filter(|s| s.start < range.end && s.end > range.start)
        .map(|s| Span {
            start: s.start.max(range.start) - range.start,
            end: s.end.min(range.end) - range.start,
            marks: s.marks.clone(),
        })
        .collect();
    normalize(out)
}

/// Sorted, merged, without empty or plain spans.
pub fn normalize<M: Clone + Default + PartialEq>(mut spans: Vec<Span<M>>) -> Vec<Span<M>> {
    let plain = M::default();
    spans.retain(|s| s.end > s.start && s.marks != plain);
    spans.sort_by_key(|s| s.start);
    let mut out: Vec<Span<M>> = Vec::with_capacity(spans.len());
    for s in spans {
        match out.last_mut() {
            Some(last) if last.end == s.start && last.marks == s.marks => last.end = s.end,
            // Overlaps (from pasting over a range) are cut back to the later span.
            Some(last) if last.end > s.start => {
                last.end = s.start;
                if last.end <= last.start {
                    out.pop();
                }
                out.push(s);
            }
            _ => out.push(s),
        }
    }
    out
}

/// The text split into consecutive pieces covering `0..len`, plain ones included.
fn pieces<M: Clone + Default>(spans: &[Span<M>], len: usize) -> Vec<Span<M>> {
    let mut out = Vec::new();
    let mut at = 0;
    for s in spans {
        let (start, end) = (s.start.min(len).max(at), s.end.min(len));
        if start > at {
            out.push(Span {
                start: at,
                end: start,
                marks: M::default(),
            });
        }
        if end > start {
            out.push(Span {
                start,
                end,
                marks: s.marks.clone(),
            });
        }
        at = at.max(end);
    }
    if len > at {
        out.push(Span {
            start: at,
            end: len,
            marks: M::default(),
        });
    }
    out
}

/// Spans that fit `text`: inside it and on character boundaries. Stored spans can be off
/// after a concurrent edit of the text.
pub fn fit_to<M: Clone + Default + PartialEq>(spans: &[Span<M>], text: &str) -> Vec<Span<M>> {
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
            marks: s.marks.clone(),
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

/// Stored form of links: per link, little-endian `u32`s `start, end, n` and then `n` bytes
/// of the link's stored form (`url:…` or `obj:…`).
pub(crate) fn links_to_bytes(links: &[LinkSpan]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for s in links {
        let Some(link) = &s.marks else { continue };
        let stored = link.to_stored();
        for v in [s.start as u32, s.end as u32, stored.len() as u32] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(stored.as_bytes());
    }
    bytes
}

pub(crate) fn links_from_bytes(mut bytes: &[u8]) -> Vec<LinkSpan> {
    let mut out = Vec::new();
    let u32_at = |b: &[u8], i: usize| -> Option<usize> {
        Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize)
    };
    while let (Some(start), Some(end), Some(n)) =
        (u32_at(bytes, 0), u32_at(bytes, 4), u32_at(bytes, 8))
    {
        let Some(stored) = bytes.get(12..12 + n) else {
            break;
        };
        if let Some(link) = std::str::from_utf8(stored).ok().and_then(Link::from_stored) {
            out.push(Span {
                start,
                end,
                marks: Some(link),
            });
        }
        bytes = &bytes[12 + n..];
    }
    normalize(out)
}

/// Stored kinds of list in [`paras_to_bytes`]; unknown ones (from newer versions) read as
/// plain paragraphs.
const LIST_BULLET: u32 = 1;
const LIST_NUMBER: u32 = 2;

/// Stored form of paragraph styles: little-endian `u32` pairs `index, style`, the style
/// being the list kind in the low byte and the level in the next.
pub(crate) fn paras_to_bytes(paras: &[Para]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(paras.len() * 8);
    for p in paras {
        let kind = match p.style.list {
            Some(ListKind::Bullet) => LIST_BULLET,
            Some(ListKind::Number) => LIST_NUMBER,
            None => continue,
        };
        let style = kind | u32::from(p.style.level) << 8;
        for v in [p.index as u32, style] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}

pub(crate) fn paras_from_bytes(bytes: &[u8]) -> Vec<Para> {
    let (chunks, _) = bytes.as_chunks::<8>();
    let paras = chunks
        .iter()
        .filter_map(|c| {
            let v = |i: usize| u32::from_le_bytes(c[i..i + 4].try_into().expect("4 bytes"));
            let style = v(4);
            let kind = match style & 0xFF {
                LIST_BULLET => ListKind::Bullet,
                LIST_NUMBER => ListKind::Number,
                _ => return None,
            };
            Some(Para {
                index: v(0) as usize,
                style: ParaStyle::list(kind, ((style >> 8) & 0xFF) as u8),
            })
        })
        .collect();
    normalize_paras(paras)
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

    fn bullet(index: usize, level: u8) -> Para {
        Para {
            index,
            style: ParaStyle::list(ListKind::Bullet, level),
        }
    }

    fn number(index: usize, level: u8) -> Para {
        Para {
            index,
            style: ParaStyle::list(ListKind::Number, level),
        }
    }

    fn link(start: usize, end: usize, to: &str) -> LinkSpan {
        Span {
            start,
            end,
            marks: Link::from_input(to),
        }
    }

    /// `spans` of `old` after typing `inserted` over `range`, with plain marks.
    fn typed(spans: &Spans, old: &str, range: Range<usize>, inserted: &str) -> (String, Spans) {
        let mut text = old.to_owned();
        text.replace_range(range.clone(), inserted);
        let marks = TextMarks::default();
        (text, spans.edited(old, range, inserted, marks, None))
    }

    #[test]
    fn list_paragraphs_follow_edits() {
        // "a\nb\nc": b is a bullet, c a nested number.
        let text = "a\nb\nc";
        let spans = Spans::new(vec![], vec![], vec![bullet(1, 0), number(2, 1)]);
        // Enter at the end of "b" continues its list; c moves down.
        let (t, s) = typed(&spans, text, 3..3, "\n");
        assert_eq!(t, "a\nb\n\nc");
        assert_eq!(&*s.paras, &[bullet(1, 0), bullet(2, 0), number(3, 1)]);
        // Enter at the start of "b" leaves an empty bullet above it.
        let (_, s) = typed(&spans, text, 2..2, "\n");
        assert_eq!(&*s.paras, &[bullet(1, 0), bullet(2, 0), number(3, 1)]);
        // A line break typed into "a" moves both lists down; the new line is plain.
        let (_, s) = typed(&spans, text, 1..1, "\nx");
        assert_eq!(&*s.paras, &[bullet(2, 0), number(3, 1)]);
        // Joining "b" onto "a" (deleting the break before it) leaves "ab" plain.
        let (t, s) = typed(&spans, text, 1..2, "");
        assert_eq!(t, "ab\nc");
        assert_eq!(&*s.paras, &[number(1, 1)]);
        // Deleting the whole line "b\n" takes its bullet along.
        let (t, s) = typed(&spans, text, 2..4, "");
        assert_eq!(t, "a\nc");
        assert_eq!(&*s.paras, &[number(1, 1)]);
        // Typing inside a paragraph changes no paragraph.
        let (_, s) = typed(&spans, text, 3..3, "xyz");
        assert_eq!(s.paras, spans.paras);
        // Pasted lines take the style of the paragraph pasted into...
        let (_, s) = typed(&spans, text, 3..3, "1\n2");
        assert_eq!(&*s.paras, &[bullet(1, 0), bullet(2, 0), number(3, 1)]);
        // ...unless they bring their own.
        let pasted = Spans::new(vec![], vec![], vec![number(1, 0)]);
        let s = spans.edited(text, 3..3, "1\n2", TextMarks::default(), Some(&pasted));
        assert_eq!(&*s.paras, &[bullet(1, 0), number(2, 0), number(3, 1)]);
    }

    #[test]
    fn links_move_with_edits_and_do_not_grow_at_their_ends() {
        let text = "see example.com now";
        let spans = Spans::new(vec![], vec![link(4, 15, "example.com")], vec![]);
        assert_eq!(
            spans.link_at(4),
            Some(&Link::Url("https://example.com".into()))
        );
        assert_eq!(spans.link_at(15), None);
        // Text before it pushes it along.
        let (t, s) = typed(&spans, text, 0..0, "Go ");
        assert_eq!(&t[s.links[0].range()], "example.com");
        // Typing right after it, or right before it, stays plain.
        let (_, s) = typed(&spans, text, 15..15, "!");
        assert_eq!(s.links[0].range(), 4..15);
        let (_, s) = typed(&spans, text, 4..4, "x");
        assert_eq!(s.links[0].range(), 5..16);
        // Typing inside it grows it.
        let (_, s) = typed(&spans, text, 8..8, "xx");
        assert_eq!(s.links[0].range(), 4..17);
        // Deleting part of it shrinks it; all of it removes it.
        let (_, s) = typed(&spans, text, 3..8, "");
        assert_eq!(s.links[0].range(), 3..10);
        let (_, s) = typed(&spans, text, 4..15, "");
        assert!(s.links.is_empty());
        // Copied and pasted, a link comes along.
        let copied = spans.slice(text, 0..15);
        assert_eq!(copied.links[0].range(), 4..15);
        let s =
            Spans::default().edited("ab", 1..1, &text[..15], TextMarks::default(), Some(&copied));
        assert_eq!(s.links[0].range(), 5..16);
    }

    #[test]
    fn typed_addresses_are_found() {
        let url = |s: &str| Some(Link::Url(s.into()));
        assert_eq!(detect_link("https://a.b/c").map(|d| d.0), Some(13));
        assert_eq!(detect_link("example.com,").map(|d| d.0), Some(11));
        assert_eq!(
            detect_link("example.com").map(|d| d.1),
            url("https://example.com")
        );
        assert_eq!(
            detect_link("www.site.example/x").map(|d| d.1),
            url("https://www.site.example/x")
        );
        assert_eq!(detect_link("mailto:me@x.ru").map(|d| d.0), Some(14));
        for word in [
            "file.txt", "v1.2", "e.g.", "т.е.", "https://", "me@x.ru", "a..com",
        ] {
            assert_eq!(detect_link(word), None, "{word}");
        }
    }

    #[test]
    fn slices_keep_whole_paragraphs_styles() {
        let text = "a\nbb\ncc";
        let spans = Spans::new(vec![], vec![], vec![bullet(1, 0), number(2, 0)]);
        // From the start of "bb": both lists come along, renumbered from 0.
        assert_eq!(
            &*spans.slice(text, 2..7).paras,
            &[bullet(0, 0), number(1, 0)]
        );
        // From inside "bb": its bullet stays behind.
        assert_eq!(&*spans.slice(text, 3..7).paras, &[number(1, 0)]);
        // Fitting to a shorter text drops lists past its end.
        assert_eq!(&*spans.fit_to("a\nbb").paras, &[bullet(1, 0)]);
    }

    #[test]
    fn numbers_count_per_level_and_start_again() {
        let paras = [
            number(0, 0),
            number(1, 1),
            number(2, 1),
            number(3, 0),
            number(4, 1),
            // Plain paragraph 5 ends the list.
            number(6, 0),
            bullet(7, 0),
            number(8, 0),
        ];
        let markers = list_markers(&paras, 9);
        let shown: Vec<String> = markers
            .iter()
            .map(|m| m.map_or_else(String::new, |m| m.text()))
            .collect();
        assert_eq!(shown, ["1.", "a.", "b.", "2.", "a.", "", "1.", "•", "1."]);
        assert_eq!(
            ListMarker {
                kind: ListKind::Number,
                level: 2,
                number: 14
            }
            .text(),
            "xiv."
        );
        assert_eq!(letters(28), "ab");
    }

    #[test]
    fn links_and_paras_roundtrip_as_bytes() {
        let links = vec![link(0, 3, "https://a.b/c?d=е"), link(5, 9, "mailto:x@y.z")];
        assert_eq!(links_from_bytes(&links_to_bytes(&links)), links);
        // Cut short (a damaged board), what is whole still reads.
        let bytes = links_to_bytes(&links);
        assert_eq!(links_from_bytes(&bytes[..bytes.len() - 2]), links[..1]);
        let paras = vec![bullet(0, 0), number(3, 4)];
        assert_eq!(paras_from_bytes(&paras_to_bytes(&paras)), paras);
        // A kind of list from a newer version reads as a plain paragraph.
        let mut bytes = paras_to_bytes(&paras);
        bytes[4] = 9;
        assert_eq!(paras_from_bytes(&bytes), [number(3, 4)]);
    }
}
