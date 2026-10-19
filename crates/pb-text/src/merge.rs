//! Merging two edits of the same text made at once: one's own, still being typed, and
//! someone else's that just arrived.
//!
//! Each side's edit is taken as one replaced stretch (what lies between the parts it
//! left alone at both ends). When the stretches do not touch, both are kept; when they
//! do, theirs is kept and the text typed here goes right after it.

/// Bytes at the start and end that `a` and `b` share, on character boundaries and not
/// overlapping.
fn common_ends(a: &str, b: &str) -> (usize, usize) {
    let prefix: usize = a
        .chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let room = a.len().min(b.len()) - prefix;
    let suffix: usize = a[prefix..]
        .chars()
        .rev()
        .zip(b[prefix..].chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(c, _)| c.len_utf8())
        .scan(0, |sum, n| {
            *sum += n;
            Some(*sum)
        })
        .take_while(|&sum| sum <= room)
        .last()
        .unwrap_or(0);
    (prefix, suffix)
}

/// An edit of `base`: bytes `start..end` of it replaced by `text`.
struct Edit<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

fn edit<'a>(base: &str, changed: &'a str) -> Edit<'a> {
    let (prefix, suffix) = common_ends(base, changed);
    Edit {
        start: prefix,
        end: base.len() - suffix,
        text: &changed[prefix..changed.len() - suffix],
    }
}

/// The text with both `mine` and `theirs` applied to `base`, and a function taking a
/// byte offset in `mine` (the cursor) to where that place is in the merged text.
pub fn merge(base: &str, mine: &str, theirs: &str) -> (String, impl Fn(usize) -> usize) {
    let m = edit(base, mine);
    let t = edit(base, theirs);
    let grown = |e: &Edit| e.text.len() as isize - (e.end - e.start) as isize;
    let (dm, dt) = (grown(&m), grown(&t));
    let shift = |at: usize, by: isize| (at as isize + by) as usize;
    let (mine_text, mine_start, mine_end) = (m.text.len(), m.start, m.end);
    let (their_start, their_end) = (t.start, t.end);
    let case = if t.end <= m.start {
        Case::TheirsFirst
    } else if t.start >= m.end {
        Case::TheirsAfter
    } else {
        Case::Overlap
    };
    let merged = match case {
        // Theirs comes first: the text typed here moves along by what they added.
        Case::TheirsFirst => format!(
            "{}{}{}",
            &theirs[..shift(m.start, dt)],
            m.text,
            &theirs[shift(m.end, dt)..]
        ),
        // Theirs comes after: it moves along by what was typed here.
        Case::TheirsAfter => format!(
            "{}{}{}",
            &mine[..shift(t.start, dm)],
            t.text,
            &mine[shift(t.end, dm)..]
        ),
        // Both changed the same stretch: theirs stands, the text typed here follows it.
        Case::Overlap => {
            let at = shift(t.end, dt);
            format!("{}{}{}", &theirs[..at], m.text, &theirs[at..])
        }
    };
    let len = merged.len();
    let map = move |c: usize| {
        let mapped = match case {
            Case::TheirsFirst if c < their_start => c,
            Case::TheirsFirst if c < their_end => their_start,
            Case::TheirsFirst => shift(c, dt),
            Case::TheirsAfter if c <= shift(their_start, dm) => c,
            Case::TheirsAfter if c < shift(their_end, dm) => shift(their_start, dm),
            Case::TheirsAfter => shift(c, dt),
            Case::Overlap => {
                let at = shift(their_end, dt);
                if c < mine_start {
                    c.min(their_start)
                } else if c <= mine_start + mine_text {
                    at + (c - mine_start)
                } else {
                    // After the typed text: the same place in the base, then in theirs.
                    let in_base = shift(c, -dm).max(mine_end);
                    if in_base >= their_end {
                        shift(in_base, dt) + mine_text
                    } else {
                        at + mine_text
                    }
                }
            }
        };
        mapped.min(len)
    };
    (merged, map)
}

#[derive(Clone, Copy)]
enum Case {
    TheirsFirst,
    TheirsAfter,
    Overlap,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(base: &str, mine: &str, theirs: &str) -> String {
        merge(base, mine, theirs).0
    }

    #[test]
    fn edits_apart_are_both_kept() {
        assert_eq!(
            merged("Hello world", "Hello big world", "Hello world!"),
            "Hello big world!"
        );
        assert_eq!(
            merged("Hello world", "Hello world!", "Hello big world"),
            "Hello big world!"
        );
        assert_eq!(merged("abc", "Xabc", "abcY"), "XabcY");
        assert_eq!(merged("abc", "abc", "aZc"), "aZc");
        assert_eq!(merged("abc", "aZc", "abc"), "aZc");
    }

    #[test]
    fn edits_of_the_same_place_keep_both_texts() {
        assert_eq!(merged("ab", "aXb", "aYb"), "aYXb");
        assert_eq!(merged("abc", "aXc", "aYc"), "aYXc");
        assert_eq!(merged("", "mine", "theirs"), "theirsmine");
    }

    #[test]
    fn non_ascii_text_stays_whole() {
        assert_eq!(
            merged("Привет мир", "Привет большой мир", "Привет мир!"),
            "Привет большой мир!"
        );
        assert_eq!(merged("ёё", "ёЖё", "ёЯё"), "ёЯЖё");
    }

    #[test]
    fn the_cursor_follows_the_text_around_it() {
        // Typing "big " before "world": the cursor after it stays after it.
        let (text, map) = merge("Hello world", "Hello big world", "Oh, Hello world");
        assert_eq!(text, "Oh, Hello big world");
        assert_eq!(&text[..map("Hello big ".len())], "Oh, Hello big ");
        // They typed after the cursor: it stays put.
        let (text, map) = merge("Hello world", "Hello big world", "Hello world!");
        assert_eq!(&text[..map("Hello big ".len())], "Hello big ");
        // The same place: the cursor stays after the text typed here.
        let (text, map) = merge("ab", "aXb", "aYb");
        assert_eq!(&text[..map(2)], "aYX");
    }

    /// Random small edits of random text: the cursor always lands on a character.
    #[test]
    fn mapped_places_are_on_characters() {
        let alphabet: Vec<char> = "abЖё 😀".chars().collect();
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut random = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let change = |text: &str, random: &mut dyn FnMut(usize) -> usize| {
            let mut chars: Vec<char> = text.chars().collect();
            let at = random(chars.len() + 1);
            let cut = random(chars.len() - at + 1).min(2);
            chars.drain(at..at + cut);
            for _ in 0..random(3) {
                chars.insert(at, alphabet[random(alphabet.len())]);
            }
            chars.into_iter().collect::<String>()
        };
        for _ in 0..2000 {
            let base: String = (0..random(6))
                .map(|_| alphabet[random(alphabet.len())])
                .collect();
            let mine = change(&base, &mut random);
            let theirs = change(&base, &mut random);
            let (text, map) = merge(&base, &mine, &theirs);
            for (c, _) in mine.char_indices().chain([(mine.len(), ' ')]) {
                let at = map(c);
                assert!(
                    text.is_char_boundary(at),
                    "{base:?} {mine:?} {theirs:?} {c}"
                );
            }
        }
    }
}
