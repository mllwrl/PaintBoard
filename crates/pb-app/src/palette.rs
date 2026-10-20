//! The command palette's matching: which entries fit what is typed, best first.
//!
//! An entry fits when what is typed starts it, starts one of its words, or appears in it,
//! in the label shown or in the English one (so English names work in any language).

/// How many entries used last are remembered, to offer them first.
pub const RECENT: usize = 5;

/// How well `query` fits `text`, ignoring case: higher is better, `None` when it does not
/// fit at all. A blank query fits everything equally.
pub fn score(query: &str, text: &str) -> Option<u32> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let text = text.to_lowercase();
    let starts: Vec<&str> = word_starts(&text).map(|i| &text[i..]).collect();
    let words: Vec<&str> = query.split_whitespace().collect();
    let score = if text == query {
        100
    } else if text.starts_with(&query) {
        90
    } else if starts.iter().any(|s| s.starts_with(&query)) {
        80
    } else if words
        .iter()
        .all(|w| starts.iter().any(|s| s.starts_with(w)))
    {
        70
    } else if text.contains(&query) {
        60
    } else if words.iter().all(|w| text.contains(w)) {
        50
    } else {
        return None;
    };
    Some(score)
}

/// Where the words of `text` start: letters or digits after anything else.
fn word_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    let mut before = None;
    text.char_indices().filter_map(move |(i, c)| {
        let word = c.is_alphanumeric();
        let starts = word && !before.is_some_and(char::is_alphanumeric);
        before = Some(c);
        starts.then_some(i)
    })
}

/// The indices of `entries` (each the label shown and the English one) that fit `query`,
/// best first, ties in their order. The `recent` ones (indices, used last first) come
/// first when nothing is typed, and win ties otherwise.
pub fn rank(query: &str, entries: &[(&str, &str)], recent: &[usize]) -> Vec<usize> {
    let mut fitting: Vec<(u32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, (label, english))| {
            let best = score(query, label).max(score(query, english))?;
            // Used lately: ahead of what fits as well, the latest most.
            let lately = recent
                .iter()
                .position(|&r| r == i)
                .map_or(0, |at| (RECENT - at.min(RECENT - 1)) as u32);
            Some((best * 10 + lately, i))
        })
        .collect();
    fitting.sort_by_key(|&(score, i)| (std::cmp::Reverse(score), i));
    fitting.into_iter().map(|(_, i)| i).collect()
}

/// Remembers `item` as used last, keeping the last [`RECENT`].
pub fn remember<T: PartialEq>(recent: &mut Vec<T>, item: T) {
    recent.retain(|r| *r != item);
    recent.insert(0, item);
    recent.truncate(RECENT);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRIES: &[(&str, &str)] = &[
        ("Открыть…", "Open…"),
        ("Сохранить как…", "Save as…"),
        ("Сохранить", "Save"),
        ("Перо", "Pen"),
        ("Разгруппировать", "Ungroup"),
        ("Сгруппировать", "Group"),
        ("Показать мини-карту", "Show the minimap"),
    ];

    fn labels(query: &str, recent: &[usize]) -> Vec<&'static str> {
        rank(query, ENTRIES, recent)
            .into_iter()
            .map(|i| ENTRIES[i].1)
            .collect()
    }

    #[test]
    fn whole_beats_start_beats_word_beats_inside() {
        assert_eq!(score("pen", "Pen"), Some(100));
        assert_eq!(score("sa", "Save as…"), Some(90));
        assert_eq!(score("min", "Show the minimap"), Some(80));
        assert_eq!(score("sh mini", "Show the minimap"), Some(70));
        assert_eq!(score("pen", "Open…"), Some(60));
        assert_eq!(score("map show", "Show the minimap"), Some(50));
        assert_eq!(score("pdf", "Open…"), None);
        assert_eq!(labels("pen", &[]), ["Pen", "Open…"]);
        assert_eq!(labels("group", &[]), ["Group", "Ungroup"]);
    }

    #[test]
    fn ties_keep_their_order_and_case_does_not_matter() {
        assert_eq!(labels("SAVE", &[]), ["Save", "Save as…"]);
        assert_eq!(labels("save", &[]), ["Save", "Save as…"]);
        assert_eq!(labels("sav", &[]), ["Save as…", "Save"]);
    }

    #[test]
    fn matches_the_label_shown_and_the_english_one() {
        assert_eq!(labels("сохр", &[]), ["Save as…", "Save"]);
        assert_eq!(labels("карт", &[]), ["Show the minimap"]);
        assert_eq!(labels("minimap", &[]), ["Show the minimap"]);
    }

    #[test]
    fn used_lately_first_when_nothing_is_typed() {
        let all = labels("", &[]);
        assert_eq!(all.len(), ENTRIES.len());
        assert_eq!(all[0], "Open…");
        let recent = labels(" ", &[3, 5]);
        assert_eq!(recent[..3], ["Pen", "Group", "Open…"]);
        // Typed: still ahead of entries that fit as well, not of better ones.
        assert_eq!(labels("sav", &[2]), ["Save", "Save as…"]);
        assert_eq!(labels("pen", &[0]), ["Pen", "Open…"]);
    }

    #[test]
    fn remembers_the_last_few() {
        let mut recent = Vec::new();
        for i in [1, 2, 3, 1, 4, 5, 6] {
            remember(&mut recent, i);
        }
        assert_eq!(recent, [6, 5, 4, 1, 3]);
    }
}
