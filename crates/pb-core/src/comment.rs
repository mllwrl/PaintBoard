//! Comment threads: conversations pinned to an object or a point on the board.

use std::sync::Arc;

use kurbo::Point;

use crate::ObjectId;

/// Where a thread is pinned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThreadAnchor {
    /// On an object; the thread is hidden while the object is gone.
    Object(ObjectId),
    /// On a point of the board (world).
    Point(Point),
}

impl ThreadAnchor {
    /// The stored form: `obj:<id>` or `pt:<x>;<y>`.
    pub(crate) fn to_stored(self) -> String {
        match self {
            Self::Object(id) => format!("obj:{id}"),
            Self::Point(p) => format!("pt:{};{}", p.x, p.y),
        }
    }

    pub(crate) fn from_stored(s: &str) -> Option<Self> {
        if let Some(id) = s.strip_prefix("obj:") {
            return ObjectId::parse(id).map(Self::Object);
        }
        let (x, y) = s.strip_prefix("pt:")?.split_once(';')?;
        let p = Point::new(x.parse().ok()?, y.parse().ok()?);
        p.is_finite().then_some(Self::Point(p))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    /// Unique within its thread (see [`crate::Document::new_line_id`]).
    pub id: Arc<str>,
    pub author: Arc<str>,
    /// When it was written, in milliseconds since the Unix epoch.
    pub at_ms: i64,
    pub text: Arc<str>,
}

impl Comment {
    /// The stored form: `<ms>;<author>;<text>`; `;` in the author's name becomes `,`.
    pub(crate) fn to_stored(&self) -> String {
        format!(
            "{};{};{}",
            self.at_ms,
            self.author.replace(';', ","),
            self.text
        )
    }

    pub(crate) fn from_stored(id: &str, s: &str) -> Option<Self> {
        let (at, rest) = s.split_once(';')?;
        let (author, text) = rest.split_once(';')?;
        Some(Self {
            id: id.into(),
            author: author.into(),
            at_ms: at.parse().ok()?,
            text: text.into(),
        })
    }

    /// The names mentioned with `@`, in order: `@ann` and `@ann.lee`, not mail addresses.
    pub fn mentions(&self) -> Vec<&str> {
        mentions(&self.text)
    }
}

/// Byte ranges of the `@mentions` in `text`, with the `@`.
pub fn mention_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let starts = c == '@' && !prev.is_some_and(|p| p.is_alphanumeric());
        prev = Some(c);
        if !starts {
            continue;
        }
        let mut end = i + 1;
        while let Some(&(j, d)) = chars.peek() {
            if d.is_alphanumeric() || d == '_' || d == '.' || d == '-' {
                end = j + d.len_utf8();
                prev = Some(d);
                chars.next();
            } else {
                break;
            }
        }
        // A trailing dot ends the sentence, not the name.
        let name = text[i + 1..end].trim_end_matches('.');
        if !name.is_empty() {
            out.push(i..i + 1 + name.len());
        }
    }
    out
}

/// The names mentioned with `@` in `text`.
pub fn mentions(text: &str) -> Vec<&str> {
    mention_ranges(text)
        .into_iter()
        .map(|r| &text[r.start + 1..r.end])
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    /// Unique across replicas (see [`crate::Document::new_line_id`]).
    pub id: Arc<str>,
    pub anchor: ThreadAnchor,
    pub resolved: bool,
    /// Oldest first; never empty for threads the document lists.
    pub comments: Vec<Comment>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_forms_round_trip() {
        for anchor in [
            ThreadAnchor::Point(Point::new(1.5, -2.0)),
            ThreadAnchor::Object(ObjectId::parse("3@42").unwrap()),
        ] {
            assert_eq!(ThreadAnchor::from_stored(&anchor.to_stored()), Some(anchor));
        }
        let c = Comment {
            id: "x".into(),
            author: "Ann; Lee".into(),
            at_ms: 1_700_000_000_000,
            text: "Looks good; ship it".into(),
        };
        let back = Comment::from_stored("x", &c.to_stored()).unwrap();
        assert_eq!(&*back.author, "Ann, Lee");
        assert_eq!(&*back.text, "Looks good; ship it");
        assert_eq!(back.at_ms, c.at_ms);
    }

    #[test]
    fn mentions_are_names_after_at() {
        assert_eq!(mentions("@ann please ask @bob.lee."), ["ann", "bob.lee"]);
        assert!(mentions("mail me at ann@example.com").is_empty());
        assert!(mentions("just @ alone").is_empty());
        let ranges = mention_ranges("hi @Аня!");
        assert_eq!((ranges.len(), ranges[0].clone()), (1, 3..10));
    }
}
