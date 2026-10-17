//! What a web page says about itself: its title, description and site name, read from
//! the `<head>` of its HTML (Open Graph tags first, then `<title>` and `description`).

/// What was found on a page; empty fields were not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub title: String,
    pub description: String,
    pub site: String,
}

/// Reads the preview from `html`. Forgiving: it looks only for the tags it needs.
pub fn parse(html: &str) -> Preview {
    // The head is enough, and pages can be long.
    let end = find_ci(html, "</head").unwrap_or(html.len());
    let head = &html[..end];
    // The first tag with each name, trying the names in order.
    let meta = |names: &[&str]| -> String {
        for name in names {
            let mut at = 0;
            while let Some(i) = find_ci(&head[at..], "<meta") {
                let start = at + i;
                let close = head[start..].find('>').map_or(head.len(), |j| start + j);
                let tag = &head[start..close];
                at = close.max(start + 5);
                let key = attr(tag, "property").or_else(|| attr(tag, "name"));
                if key.is_some_and(|k| k.eq_ignore_ascii_case(name))
                    && let Some(content) = attr(tag, "content")
                {
                    let text = clean(&content);
                    if !text.is_empty() {
                        return text;
                    }
                }
            }
        }
        String::new()
    };
    let mut title = meta(&["og:title", "twitter:title"]);
    let description = meta(&["og:description", "description", "twitter:description"]);
    let site = meta(&["og:site_name", "application-name"]);
    if title.is_empty()
        && let Some(i) = find_ci(head, "<title")
        && let Some(open_end) = head[i..].find('>')
    {
        let from = i + open_end + 1;
        let to = find_ci(&head[from..], "</title").map_or(head.len(), |j| from + j);
        title = clean(&head[from..to]);
    }
    Preview {
        title: limit(title, 200),
        description: limit(description, 300),
        site: limit(site, 80),
    }
}

/// The value of attribute `name` in `tag`, quoted or not.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let start = from + i;
        from = start + name.len();
        // A whole attribute name: preceded by a space, followed by `=`.
        let before = lower[..start].chars().last();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or(""),
            _ => rest
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or(""),
        };
        return Some(value.to_owned());
    }
    None
}

/// `text` with its entities decoded and its white space collapsed.
fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` cut to `max` characters, with an ellipsis when cut.
fn limit(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

/// The byte index of `needle` (ASCII) in `haystack`, ignoring ASCII case.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    (0..h.len().saturating_sub(n.len() - 1)).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_graph_comes_first() {
        let html = r#"<!doctype html><html><head>
            <title>Plain &amp; simple</title>
            <meta name="description" content="Fallback">
            <meta property='og:title' content="Rich &quot;title&quot;">
            <META PROPERTY="og:description" CONTENT="Says   a lot&#8230;">
            <meta property="og:site_name" content=Example />
            </head><body><meta property="og:title" content="Not this"></body></html>"#;
        let p = parse(html);
        assert_eq!(p.title, "Rich \"title\"");
        assert_eq!(p.description, "Says a lot…");
        assert_eq!(p.site, "Example");
    }

    #[test]
    fn falls_back_to_the_title_tag() {
        let p = parse("<html><head><title>\n  Hello\n  world </title></head></html>");
        assert_eq!(p.title, "Hello world");
        assert!(p.description.is_empty() && p.site.is_empty());
        assert_eq!(parse("no html at all"), Preview::default());
        let long = format!("<title>{}</title>", "x".repeat(500));
        assert_eq!(parse(&long).title.chars().count(), 200);
    }
}
