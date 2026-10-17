//! Markdown documents: a small, forgiving subset parsed into blocks and laid out down a
//! column.
//!
//! Blocks: `#`–`######` headings (levels past 3 look like 3), paragraphs, `-`/`*`/`+` and
//! `1.` lists (nested by two spaces), `>` quotes, fenced code blocks, and `---` rules.
//! Inline: `**bold**`, `__bold__`, `*italic*`, `_italic_`, `~~struck~~`, `` `code` ``,
//! `[links](url)` and `\` escapes.

use pb_core::{Rgba, Span, TextMarks};

use crate::{TextAlign, TextEngine, TextLayout};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Heading(u8),
    Paragraph,
    /// A list item: nesting depth from 0, and its number for numbered lists.
    Item {
        depth: usize,
        number: Option<u32>,
    },
    Quote,
    Code,
    Rule,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    /// Text with the inline markup taken out.
    pub text: String,
    pub spans: Vec<Span>,
}

/// Color of inline code and code blocks, and of links.
pub const CODE_COLOR: Rgba = Rgba::rgb(0xB4, 0x23, 0x5A);
pub const LINK_COLOR: Rgba = Rgba::rgb(0x25, 0x63, 0xEB);
/// Color of quotes.
pub const QUOTE_COLOR: Rgba = Rgba::rgb(0x5F, 0x63, 0x68);

/// Reads `source` into blocks.
pub fn parse(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut quote: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    let flush = |blocks: &mut Vec<Block>, lines: &mut Vec<&str>, kind: BlockKind| {
        if !lines.is_empty() {
            let joined = lines.join(" ");
            let (text, mut spans) = inline(&joined);
            if kind == BlockKind::Quote {
                color_all(&mut spans, &text, QUOTE_COLOR);
            }
            blocks.push(Block { kind, text, spans });
            lines.clear();
        }
    };
    for line in source.lines() {
        if let Some(lines) = &mut code {
            if line.trim_start().starts_with("```") {
                let text = lines.join("\n");
                let mut spans = Vec::new();
                color_all(&mut spans, &text, CODE_COLOR);
                blocks.push(Block {
                    kind: BlockKind::Code,
                    text,
                    spans,
                });
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        let trimmed = line.trim();
        let block_start = trimmed.is_empty()
            || trimmed.starts_with("```")
            || trimmed.starts_with('#')
            || trimmed.starts_with('>')
            || is_rule(trimmed)
            || list_item(line).is_some();
        if block_start {
            flush(&mut blocks, &mut paragraph, BlockKind::Paragraph);
        }
        if !trimmed.starts_with('>') {
            flush(&mut blocks, &mut quote, BlockKind::Quote);
        }
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("```") {
            code = Some(Vec::new());
        } else if let Some(rest) = heading(trimmed) {
            let (level, text) = rest;
            let (text, mut spans) = inline(text);
            bold_all(&mut spans, &text);
            blocks.push(Block {
                kind: BlockKind::Heading(level),
                text,
                spans,
            });
        } else if is_rule(trimmed) {
            blocks.push(Block {
                kind: BlockKind::Rule,
                text: String::new(),
                spans: Vec::new(),
            });
        } else if let Some(text) = trimmed.strip_prefix('>') {
            quote.push(text.trim());
        } else if let Some((depth, number, text)) = list_item(line) {
            let (text, spans) = inline(text);
            blocks.push(Block {
                kind: BlockKind::Item { depth, number },
                text,
                spans,
            });
        } else {
            paragraph.push(trimmed);
        }
    }
    flush(&mut blocks, &mut paragraph, BlockKind::Paragraph);
    flush(&mut blocks, &mut quote, BlockKind::Quote);
    if let Some(lines) = code {
        // An unclosed fence runs to the end.
        let text = lines.join("\n");
        let mut spans = Vec::new();
        color_all(&mut spans, &text, CODE_COLOR);
        blocks.push(Block {
            kind: BlockKind::Code,
            text,
            spans,
        });
    }
    blocks
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((
        hashes.min(3) as u8,
        rest.trim().trim_end_matches('#').trim(),
    ))
}

fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|m| compact.chars().all(|c| c.to_string() == *m))
}

/// A list item: its depth (two spaces or a tab a level), number, and text.
fn list_item(line: &str) -> Option<(usize, Option<u32>, &str)> {
    let indent: usize = line
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 2 } else { 1 })
        .sum();
    let rest = line.trim_start();
    let depth = indent / 2;
    for bullet in ["- ", "* ", "+ "] {
        if let Some(text) = rest.strip_prefix(bullet) {
            return Some((depth, None, text.trim()));
        }
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits <= 9 {
        let after = &rest[digits..];
        if let Some(text) = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))
        {
            let number = rest[..digits].parse().ok();
            return Some((depth, number, text.trim()));
        }
    }
    None
}

fn bold_all(spans: &mut Vec<Span>, text: &str) {
    restyle_all(spans, text, |m| m.bold = true);
}

fn color_all(spans: &mut Vec<Span>, text: &str, color: Rgba) {
    restyle_all(spans, text, |m| {
        if m.color.is_none() {
            m.color = Some(color);
        }
    });
}

/// Applies `f` to the marks of all of `text`, filling the gaps between spans.
fn restyle_all(spans: &mut Vec<Span>, text: &str, f: impl Fn(&mut TextMarks)) {
    let mut out = Vec::new();
    let mut at = 0;
    for span in spans.iter() {
        if span.start > at {
            out.push(Span {
                start: at,
                end: span.start,
                marks: TextMarks::default(),
            });
        }
        out.push(*span);
        at = span.end;
    }
    if at < text.len() {
        out.push(Span {
            start: at,
            end: text.len(),
            marks: TextMarks::default(),
        });
    }
    for span in &mut out {
        f(&mut span.marks);
    }
    *spans = out;
}

/// Inline markup taken out of `source`: the plain text and its styled runs.
pub fn inline(source: &str) -> (String, Vec<Span>) {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut marks = TextMarks::default();
    let mut run_start = 0;
    let mut i = 0;
    // Closes the run so far (if styled) when the marks are about to change.
    let close = |out: &String, spans: &mut Vec<Span>, run_start: &mut usize, marks: &TextMarks| {
        if out.len() > *run_start && !marks.is_plain() {
            spans.push(Span {
                start: *run_start,
                end: out.len(),
                marks: *marks,
            });
        }
        *run_start = out.len();
    };
    let has_closing = |from: usize, pattern: &[char]| {
        (from..chars.len()).any(|j| chars[j..].starts_with(pattern))
    };
    while i < chars.len() {
        let c = chars[i];
        let rest = &chars[i..];
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '`'
            && let Some(len) = chars[i + 1..].iter().position(|&x| x == '`')
        {
            close(&out, &mut spans, &mut run_start, &marks);
            let saved = marks;
            marks.color = Some(CODE_COLOR);
            out.extend(&chars[i + 1..i + 1 + len]);
            close(&out, &mut spans, &mut run_start, &marks);
            marks = saved;
            i += len + 2;
            continue;
        }
        if c == '['
            && let Some(close_text) = chars[i + 1..].iter().position(|&x| x == ']')
        {
            let after = i + 1 + close_text + 1;
            if chars.get(after) == Some(&'(')
                && let Some(close_url) = chars[after..].iter().position(|&x| x == ')')
            {
                close(&out, &mut spans, &mut run_start, &marks);
                let saved = marks;
                marks.underline = true;
                marks.color = Some(LINK_COLOR);
                let (text, inner) = inline(&chars[i + 1..after - 1].iter().collect::<String>());
                let base = out.len();
                out.push_str(&text);
                // Styles inside the link text keep the link look.
                if inner.is_empty() {
                    close(&out, &mut spans, &mut run_start, &marks);
                } else {
                    for mut span in inner {
                        span.start += base;
                        span.end += base;
                        span.marks.underline = true;
                        span.marks.color = Some(LINK_COLOR);
                        spans.push(span);
                    }
                    run_start = out.len();
                }
                marks = saved;
                i = after + close_url + 1;
                continue;
            }
        }
        let toggle = |marks: &mut TextMarks, which: &str| match which {
            "bold" => marks.bold = !marks.bold,
            "italic" => marks.italic = !marks.italic,
            _ => marks.strike = !marks.strike,
        };
        let pairs: [(&[char], &str); 5] = [
            (&['*', '*'], "bold"),
            (&['_', '_'], "bold"),
            (&['~', '~'], "strike"),
            (&['*'], "italic"),
            (&['_'], "italic"),
        ];
        let mut matched = false;
        for (pattern, which) in pairs {
            if !rest.starts_with(pattern) {
                continue;
            }
            let on = match which {
                "bold" => marks.bold,
                "italic" => marks.italic,
                _ => marks.strike,
            };
            // Opens only when a closing one follows, so lone `*` and `snake_case` stay.
            let word_inside = pattern == ['_']
                && i > 0
                && chars[i - 1].is_alphanumeric()
                && chars.get(i + 1).is_some_and(|c| c.is_alphanumeric());
            if word_inside || (!on && !has_closing(i + pattern.len(), pattern)) {
                continue;
            }
            close(&out, &mut spans, &mut run_start, &marks);
            toggle(&mut marks, which);
            i += pattern.len();
            matched = true;
            break;
        }
        if matched {
            continue;
        }
        out.push(c);
        i += 1;
    }
    close(&out, &mut spans, &mut run_start, &marks);
    (out, spans)
}

/// A laid-out block: where its text goes (from the column's top left), its kind, and
/// its text, plus a marker (bullet or number) for list items.
#[derive(Clone, Debug)]
pub struct PlacedBlock {
    pub kind: BlockKind,
    pub x: f64,
    pub y: f64,
    pub layout: TextLayout,
    pub marker: Option<(f64, TextLayout)>,
    /// The block's own box: for code backgrounds, quote bars and rules.
    pub area: (f64, f64, f64, f64),
}

#[derive(Clone, Debug, Default)]
pub struct MarkdownLayout {
    pub blocks: Vec<PlacedBlock>,
    pub height: f64,
}

/// Lays `source` out in a column `width` wide with body text at `size`.
pub fn layout(engine: &mut TextEngine, source: &str, width: f64, size: f64) -> MarkdownLayout {
    let mut out = MarkdownLayout::default();
    let mut y = 0.0;
    let gap = size * 0.6;
    let width = width.max(size);
    let mut previous: Option<BlockKind> = None;
    for block in parse(source) {
        // Lists keep their items close.
        let item = matches!(block.kind, BlockKind::Item { .. });
        let tight = item && matches!(previous, Some(BlockKind::Item { .. }));
        if previous.is_some() {
            y += if tight { size * 0.25 } else { gap };
            if matches!(block.kind, BlockKind::Heading(_)) {
                y += size * 0.4;
            }
        }
        let text_size = match block.kind {
            BlockKind::Heading(1) => size * 1.8,
            BlockKind::Heading(2) => size * 1.45,
            BlockKind::Heading(_) => size * 1.2,
            BlockKind::Code => size * 0.9,
            _ => size,
        };
        let (x, inner, pad) = match block.kind {
            BlockKind::Item { depth, .. } => {
                let x = (depth as f64 + 1.0) * size * 1.4;
                (x, width - x, 0.0)
            }
            BlockKind::Quote => (size, width - size, 0.0),
            BlockKind::Code => (size * 0.6, width - size * 1.2, size * 0.5),
            _ => (0.0, width, 0.0),
        };
        if block.kind == BlockKind::Rule {
            let h = size;
            out.blocks.push(PlacedBlock {
                kind: block.kind,
                x: 0.0,
                y,
                layout: TextLayout::default(),
                marker: None,
                area: (0.0, y, width, y + h),
            });
            y += h;
            previous = Some(block.kind);
            continue;
        }
        let text = if block.text.is_empty() {
            " "
        } else {
            &block.text
        };
        let spans = if block.text.is_empty() {
            &[][..]
        } else {
            &block.spans[..]
        };
        let text_layout = engine.layout_aligned(
            text,
            spans,
            text_size,
            Some(inner.max(size)),
            TextAlign::Start,
        );
        let marker = match block.kind {
            BlockKind::Item { depth, number } => {
                let label = match number {
                    Some(n) => format!("{n}."),
                    None if depth % 2 == 1 => "◦".to_owned(),
                    None => "•".to_owned(),
                };
                let m = engine.layout(&label, text_size, None);
                Some((x - m.width - size * 0.4, m))
            }
            _ => None,
        };
        let top = y + pad;
        let h = text_layout.height + 2.0 * pad;
        out.blocks.push(PlacedBlock {
            kind: block.kind,
            x,
            y: top,
            area: match block.kind {
                BlockKind::Code => (0.0, y, width, y + h),
                BlockKind::Quote => (0.0, y, width, y + h),
                _ => (x, y, width, y + h),
            },
            layout: text_layout,
            marker,
        });
        y += h;
        previous = Some(block.kind);
    }
    out.height = y;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_recognized() {
        let blocks = parse(
            "# Title\n\nSome *nice*\ntext.\n\n- one\n  - two\n1. first\n> said\n> twice\n\n---\n```\nlet x = 1;\n```\n",
        );
        let kinds: Vec<_> = blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            [
                BlockKind::Heading(1),
                BlockKind::Paragraph,
                BlockKind::Item {
                    depth: 0,
                    number: None
                },
                BlockKind::Item {
                    depth: 1,
                    number: None
                },
                BlockKind::Item {
                    depth: 0,
                    number: Some(1)
                },
                BlockKind::Quote,
                BlockKind::Rule,
                BlockKind::Code,
            ]
        );
        assert_eq!(blocks[1].text, "Some nice text.");
        assert_eq!(blocks[5].text, "said twice");
        assert_eq!(blocks[7].text, "let x = 1;");
    }

    #[test]
    fn inline_markup_becomes_styles() {
        let (text, spans) = inline("a **b** *c* ~~d~~ `e` [f](http://x) snake_case 2*3");
        assert_eq!(text, "a b c d e f snake_case 2*3");
        let marks = |s: &str| {
            let start = text.find(s).unwrap();
            spans
                .iter()
                .find(|sp| sp.start == start)
                .map(|sp| sp.marks)
                .unwrap_or_default()
        };
        assert!(marks("b").bold);
        assert!(marks("c").italic);
        assert!(marks("d").strike);
        assert_eq!(marks("e").color, Some(CODE_COLOR));
        assert!(marks("f").underline);
        assert!(marks("snake_case").is_plain());
        let (text, _) = inline(r"\*not italic\*");
        assert_eq!(text, "*not italic*");
    }

    #[test]
    fn layout_stacks_blocks_down_the_column() {
        let mut engine = TextEngine::new();
        let md = layout(&mut engine, "# Big\n\nsmall text\n\n- item", 300.0, 16.0);
        assert_eq!(md.blocks.len(), 3);
        assert!(md.blocks[0].layout.height > md.blocks[1].layout.height);
        assert!(md.blocks[1].y > md.blocks[0].y && md.blocks[2].y > md.blocks[1].y);
        assert!(md.blocks[2].marker.is_some() && md.blocks[2].x > 0.0);
        // Narrower means taller.
        let long = "word ".repeat(60);
        let wide = layout(&mut engine, &long, 600.0, 16.0).height;
        let narrow = layout(&mut engine, &long, 200.0, 16.0).height;
        assert!(narrow > wide);
    }
}
