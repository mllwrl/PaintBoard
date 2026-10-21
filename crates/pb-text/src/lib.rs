//! Text layout: shaping, line breaking and glyph positions, independent of rendering.
//!
//! All text uses Inter (SIL Open Font License, see `assets/fonts/Inter-OFL.txt`), which
//! covers Latin, Cyrillic and Greek, in regular, bold, italic and bold italic.
//!
//! Markdown documents are laid out in [`markdown`].

mod editor;
pub mod markdown;
pub mod merge;
mod rich_editor;

pub use editor::{Mark, Motion, TextEditor, TextRect};

use std::borrow::Cow;
use std::sync::Arc;

use parley::fontique::Blob;
use parley::{
    Alignment, AlignmentOptions, BreakReason, FontContext, FontFamily, FontFamilyName, FontStyle,
    FontWeight, Layout, LayoutContext, PositionedLayoutItem, RangedBuilder, StyleProperty,
    YieldData,
};
use pb_core::rich;
use pb_core::{Rgba, Span, Spans};

pub use parley::FontData;

/// The font for board text and the UI.
pub const FONT: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
/// The other faces of [`FONT`], for styled runs of board text.
const BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");
const ITALIC: &[u8] = include_bytes!("../assets/fonts/Inter-Italic.ttf");
const BOLD_ITALIC: &[u8] = include_bytes!("../assets/fonts/Inter-BoldItalic.ttf");
const FAMILY: &str = "Inter";
/// Line height relative to the font size.
const LINE_HEIGHT: f32 = 1.2;

/// What text is drawn with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Brush {
    /// The text's default color.
    #[default]
    Text,
    /// A color of its own.
    Color(Rgba),
    /// The link color, which the renderer picks to suit the text's color.
    Link,
}

/// Where a list paragraph's text starts, relative to the font size: past the marker at
/// level 0, and further in per level.
const LIST_INDENT: f32 = 1.5;
const LIST_STEP: f32 = 1.25;
/// Space between a list marker and its paragraph's text, relative to the font size.
const MARKER_GAP: f32 = 0.4;

/// A glyph placed on its baseline, relative to the layout's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// Glyphs of one face, size and color.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    pub font: FontData,
    pub size: f32,
    pub brush: Brush,
    pub glyphs: Vec<Glyph>,
}

/// An underline or strikethrough: a horizontal bar, relative to the layout's top-left
/// corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoration {
    pub x0: f32,
    pub x1: f32,
    /// Middle of the bar.
    pub y: f32,
    pub thickness: f32,
    pub brush: Brush,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayout {
    /// Including trailing spaces, so a caret after them stays inside.
    pub width: f64,
    /// Without trailing spaces, including those hanging at wrapped line ends.
    pub content_width: f64,
    pub height: f64,
    pub runs: Vec<GlyphRun>,
    pub decorations: Vec<Decoration>,
    /// Where the text's links are, for files that keep them clickable.
    pub links: Vec<LinkArea>,
}

/// A box over (part of) a link's text, relative to the layout's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkArea {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub link: pb_core::Link,
}

/// Boxes over the links of `spans` in `layout`, a box per line of each.
pub(crate) fn link_areas(layout: &Layout<Brush>, spans: &Spans) -> Vec<LinkArea> {
    let mut out = Vec::new();
    for span in spans.links.iter() {
        let Some(link) = &span.marks else { continue };
        let cursor = |i: usize, affinity| parley::Cursor::from_byte_index(layout, i, affinity);
        let selection = parley::Selection::new(
            cursor(span.start, parley::Affinity::Downstream),
            cursor(span.end, parley::Affinity::Upstream),
        );
        for (b, _) in selection.geometry(layout) {
            out.push(LinkArea {
                x0: b.x0,
                y0: b.y0,
                x1: b.x1,
                y1: b.y1,
                link: link.clone(),
            });
        }
    }
    out
}

impl TextLayout {
    pub fn glyph_count(&self) -> usize {
        self.runs.iter().map(|r| r.glyphs.len()).sum()
    }
}

/// How lines sit horizontally.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Start,
    /// Centered within the wrapping width.
    Center,
    /// Against the right edge of the wrapping width (for left-to-right text).
    End,
}

impl TextAlign {
    pub(crate) fn to_parley(self) -> Alignment {
        match self {
            Self::Start => Alignment::Start,
            Self::Center => Alignment::Center,
            Self::End => Alignment::End,
        }
    }
}

/// Bisection steps when fitting text; the size ends up within `max / 2^STEPS`.
const FIT_STEPS: u32 = 10;
/// Text is never fitted smaller than this fraction of the largest size.
const MIN_FIT: f64 = 1.0 / 32.0;

/// Shapes and lays out text. Keeps font data and caches between calls.
pub struct TextEngine {
    pub(crate) fonts: FontContext,
    pub(crate) layouts: LayoutContext<Brush>,
}

impl TextEngine {
    pub fn new() -> Self {
        let mut fonts = FontContext::new();
        for face in [FONT, BOLD, ITALIC, BOLD_ITALIC] {
            fonts
                .collection
                .register_fonts(Blob::new(Arc::new(face)), None);
        }
        Self {
            fonts,
            layouts: LayoutContext::new(),
        }
    }

    /// Lays out `text` at font `size`, wrapping lines at `max_width` if given. Lines start
    /// at the left edge, as in [`TextEditor`].
    pub fn layout(&mut self, text: &str, size: f64, max_width: Option<f64>) -> TextLayout {
        self.layout_aligned(text, &Spans::default(), size, max_width, TextAlign::Start)
    }

    /// Like [`Self::layout`], with inline styles, links and lists, and lines aligned
    /// within `max_width`.
    pub fn layout_aligned(
        &mut self,
        text: &str,
        spans: &Spans,
        size: f64,
        max_width: Option<f64>,
        align: TextAlign,
    ) -> TextLayout {
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, text, 1.0, false);
        for prop in default_styles(size) {
            builder.push_default(prop);
        }
        push_styles(&mut builder, spans);
        let mut layout: Layout<Brush> = builder.build(text);
        let indents = list_indents(text, &spans.paras, size as f32);
        break_lines(&mut layout, max_width.map(|w| w as f32), &indents);
        layout.align(align.to_parley(), AlignmentOptions::default());
        let mut out = extract(&layout);
        self.add_markers(&mut out, &layout, text, spans, size);
        out.links = link_areas(&layout, spans);
        out
    }

    /// Like [`Self::layout_aligned`] for text with inline styles only.
    pub fn layout_marks(
        &mut self,
        text: &str,
        marks: &[Span],
        size: f64,
        max_width: Option<f64>,
        align: TextAlign,
    ) -> TextLayout {
        let spans = Spans::from_marks(marks.to_vec());
        self.layout_aligned(text, &spans, size, max_width, align)
    }

    /// Adds the bullets and numbers of the list paragraphs of `text` (laid out as
    /// `layout`) to `out`, before the first line of each.
    pub(crate) fn add_markers(
        &mut self,
        out: &mut TextLayout,
        layout: &Layout<Brush>,
        text: &str,
        spans: &Spans,
        size: f64,
    ) {
        if spans.paras.is_empty() {
            return;
        }
        let ranges = rich::paragraph_ranges(text);
        let markers = rich::list_markers(&spans.paras, ranges.len());
        // The first line of each paragraph: where its text starts, and its baseline.
        let mut starts = Vec::new();
        for line in layout.lines() {
            let start = line.text_range().start;
            let m = line.metrics();
            if let Some(i) = ranges.iter().position(|r| r.start == start)
                && starts.iter().all(|&(j, _, _)| j != i)
            {
                starts.push((i, m.offset + m.inline_min_coord, m.baseline));
            }
        }
        for (i, x, baseline) in starts {
            let Some(marker) = markers.get(i).copied().flatten() else {
                continue;
            };
            let label = marker.text();
            let shaped =
                self.layout_aligned(&label, &Spans::default(), size, None, TextAlign::Start);
            let right = x - MARKER_GAP * size as f32;
            let left = right - shaped.width as f32;
            for run in shaped.runs {
                let first_baseline = run.glyphs.first().map_or(0.0, |g| g.y);
                out.runs.push(GlyphRun {
                    glyphs: run
                        .glyphs
                        .iter()
                        .map(|g| Glyph {
                            id: g.id,
                            x: g.x + left,
                            y: g.y - first_baseline + baseline,
                        })
                        .collect(),
                    ..run
                });
            }
        }
    }

    /// The largest font size, up to `max_size`, at which `text` with `spans` wrapped at
    /// `width` fits in `width` x `height`. Words are not broken, so a long word shrinks the
    /// text instead.
    pub fn fit_size(
        &mut self,
        text: &str,
        spans: &Spans,
        width: f64,
        height: f64,
        max_size: f64,
    ) -> f64 {
        let fits = |engine: &mut Self, size: f64| {
            let layout = engine.layout_aligned(text, spans, size, Some(width), TextAlign::Start);
            layout.content_width <= width && layout.height <= height
        };
        if max_size <= 0.0 || fits(self, max_size) {
            return max_size.max(0.0);
        }
        let (mut lo, mut hi) = (max_size * MIN_FIT, max_size);
        for _ in 0..FIT_STEPS {
            let mid = (lo + hi) / 2.0;
            if fits(self, mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

/// Styles all text starts with, at font `size`.
pub(crate) fn default_styles(size: f64) -> [StyleProperty<'static, Brush>; 3] {
    [
        StyleProperty::FontFamily(FontFamily::Single(FontFamilyName::Named(Cow::Borrowed(
            FAMILY,
        )))),
        StyleProperty::FontSize(size as f32),
        StyleProperty::LineHeight(parley::LineHeight::FontSizeRelative(LINE_HEIGHT)),
    ]
}

/// Pushes the styles of inline marks and links onto `builder`.
pub(crate) fn push_styles(builder: &mut RangedBuilder<'_, Brush>, spans: &Spans) {
    for span in spans.marks.iter() {
        for prop in span_styles(span) {
            builder.push(prop, span.range());
        }
    }
    for link in spans.links.iter() {
        builder.push(StyleProperty::Underline(true), link.range());
        builder.push(StyleProperty::Brush(Brush::Link), link.range());
    }
}

/// How far in each paragraph of `text` starts: list items by their level, others not.
pub(crate) fn list_indents(text: &str, paras: &[pb_core::Para], size: f32) -> Vec<f32> {
    if paras.is_empty() {
        return Vec::new();
    }
    (0..rich::paragraph_count(text))
        .map(|i| {
            let style = rich::para_style(paras, i);
            match style.list {
                Some(_) => size * (LIST_INDENT + LIST_STEP * f32::from(style.level)),
                None => 0.0,
            }
        })
        .collect()
}

/// Breaks `layout` into lines at `max_width`, each line of paragraph `i` starting
/// `indents[i]` in (a hanging indent: wrapped lines line up with the first).
pub(crate) fn break_lines(layout: &mut Layout<Brush>, max_width: Option<f32>, indents: &[f32]) {
    if indents.iter().all(|&i| i == 0.0) {
        layout.break_all_lines(max_width);
        return;
    }
    let width = max_width.unwrap_or(f32::MAX);
    let mut lines = layout.break_lines();
    lines.state_mut().set_layout_max_advance(width);
    let mut paragraph = 0;
    loop {
        let indent = indents.get(paragraph).copied().unwrap_or(0.0);
        let state = lines.state_mut();
        state.set_line_x(indent);
        state.set_line_max_advance(if width == f32::MAX {
            f32::MAX
        } else {
            // Even a narrow box keeps some room for the text.
            (width - indent).max(width.min(indent))
        });
        match lines.break_next() {
            None => break,
            Some(YieldData::LineBreak(data)) => {
                if data.reason == BreakReason::Explicit {
                    paragraph += 1;
                }
            }
            // No inline boxes and no height limit here.
            Some(YieldData::InlineBoxBreak(_) | YieldData::MaxHeightExceeded(_)) => break,
        }
    }
    lines.finish();
}

/// The layout styles for a span's marks.
pub(crate) fn span_styles(span: &Span) -> Vec<StyleProperty<'static, Brush>> {
    let m = span.marks;
    let mut props = Vec::new();
    if m.bold {
        props.push(StyleProperty::FontWeight(FontWeight::BOLD));
    }
    if m.italic {
        props.push(StyleProperty::FontStyle(FontStyle::Italic));
    }
    if m.underline {
        props.push(StyleProperty::Underline(true));
    }
    if m.strike {
        props.push(StyleProperty::Strikethrough(true));
    }
    if let Some(color) = m.color {
        props.push(StyleProperty::Brush(Brush::Color(color)));
    }
    props
}

/// Glyph runs, decorations and size of a finished layout.
pub(crate) fn extract(layout: &Layout<Brush>) -> TextLayout {
    let mut runs = Vec::new();
    let mut decorations = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let style = run.style();
            let glyphs = run
                .positioned_glyphs()
                .map(|g| Glyph {
                    id: g.id,
                    x: g.x,
                    y: g.y,
                })
                .collect();
            runs.push(GlyphRun {
                font: run.run().font().clone(),
                size: run.run().font_size(),
                brush: style.brush,
                glyphs,
            });
            let metrics = run.run().font_metrics();
            let (x0, x1) = (run.offset(), run.offset() + run.advance());
            let bar = |offset: f32, size: f32, brush: Brush| Decoration {
                x0,
                x1,
                // Offsets are up from the baseline; y grows down.
                y: run.baseline() - offset,
                thickness: size,
                brush,
            };
            if let Some(d) = &style.underline {
                let offset = d.offset.unwrap_or(metrics.underline_offset);
                let size = d.size.unwrap_or(metrics.underline_size);
                decorations.push(bar(offset - size / 2.0, size, d.brush));
            }
            if let Some(d) = &style.strikethrough {
                let offset = d.offset.unwrap_or(metrics.strikethrough_offset);
                let size = d.size.unwrap_or(metrics.strikethrough_size);
                decorations.push(bar(offset - size / 2.0, size, d.brush));
            }
        }
    }
    TextLayout {
        width: f64::from(layout.full_width()),
        content_width: f64::from(layout.width()),
        height: f64::from(layout.height()),
        runs,
        decorations,
        links: Vec::new(),
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_latin_and_cyrillic() {
        let mut engine = TextEngine::new();
        let layout = engine.layout("Hi Привет 2+7", 20.0, None);
        assert_eq!(layout.glyph_count(), 13);
        // Inter has these glyphs, so none is the missing-glyph box (id 0).
        assert!(
            layout
                .runs
                .iter()
                .flat_map(|r| &r.glyphs)
                .all(|g| g.id != 0)
        );
        assert!((layout.height - 24.0).abs() < 1.0, "{}", layout.height);
        assert!(
            layout.width > 100.0 && layout.width < 200.0,
            "{}",
            layout.width
        );
    }

    #[test]
    fn size_scales_the_layout() {
        let mut engine = TextEngine::new();
        let small = engine.layout("19", 10.0, None);
        let big = engine.layout("19", 40.0, None);
        assert!((big.width / small.width - 4.0).abs() < 0.05);
    }

    #[test]
    fn centers_lines_in_the_wrapping_width() {
        let mut engine = TextEngine::new();
        let plain = Spans::default();
        let start = engine.layout_aligned("hi", &plain, 20.0, Some(200.0), TextAlign::Start);
        let center = engine.layout_aligned("hi", &plain, 20.0, Some(200.0), TextAlign::Center);
        let shift = center.runs[0].glyphs[0].x - start.runs[0].glyphs[0].x;
        let expected = (200.0 - start.width) / 2.0;
        assert!(
            (f64::from(shift) - expected).abs() < 0.5,
            "{shift} vs {expected}"
        );
    }

    #[test]
    fn fit_size_shrinks_long_text_only() {
        let mut engine = TextEngine::new();
        let plain = Spans::default();
        assert_eq!(engine.fit_size("ok", &plain, 200.0, 200.0, 30.0), 30.0);
        assert_eq!(engine.fit_size("", &plain, 200.0, 200.0, 30.0), 30.0);
        let long = "a fairly long sentence that cannot fit on a note at full size ".repeat(4);
        let size = engine.fit_size(&long, &plain, 200.0, 200.0, 30.0);
        assert!(size < 30.0 && size > 5.0, "{size}");
        let layout = engine.layout(&long, size, Some(200.0));
        assert!(layout.height <= 200.0 && layout.content_width <= 200.0);
        // Spaces hanging at wrapped line ends do not count, so the text fills the note.
        assert!(layout.height > 150.0, "{}", layout.height);
        // A single long word shrinks to the width instead of overflowing.
        let word = engine.fit_size("Donaudampfschifffahrt", &plain, 100.0, 100.0, 30.0);
        assert!(engine.layout("Donaudampfschifffahrt", word, None).width <= 100.0);
    }

    #[test]
    fn spans_pick_faces_colors_and_lines() {
        use pb_core::{Span, TextMarks};
        let mut engine = TextEngine::new();
        let plain = engine.layout("bold", 20.0, None);
        let span = |marks| Span {
            start: 0,
            end: 4,
            marks,
        };
        let bold = TextMarks {
            bold: true,
            ..TextMarks::default()
        };
        let styled = engine.layout_marks("bold", &[span(bold)], 20.0, None, TextAlign::Start);
        assert!(
            styled.width > plain.width,
            "{} vs {}",
            styled.width,
            plain.width
        );
        assert_ne!(styled.runs[0].font, plain.runs[0].font);
        let red = pb_core::Rgba::rgb(0xF2, 0x55, 0x55);
        let marks = TextMarks {
            italic: true,
            underline: true,
            strike: true,
            color: Some(red),
            ..TextMarks::default()
        };
        let styled = engine.layout_marks("bold", &[span(marks)], 20.0, None, TextAlign::Start);
        assert_eq!(styled.runs[0].brush, Brush::Color(red));
        assert_eq!(styled.decorations.len(), 2);
        let [under, strike] = [styled.decorations[0], styled.decorations[1]];
        // Underline below the baseline, strikethrough above it, both across the word.
        let baseline = styled.runs[0].glyphs[0].y;
        assert!(
            under.y > baseline && strike.y < baseline,
            "{under:?} {strike:?}"
        );
        assert!(under.x1 - under.x0 > 30.0);
        // Faces have glyphs for Cyrillic too.
        let ru = engine.layout_marks(
            "жирный",
            &[Span {
                start: 0,
                end: 12,
                marks,
            }],
            20.0,
            None,
            TextAlign::Start,
        );
        assert!(ru.runs.iter().flat_map(|r| &r.glyphs).all(|g| g.id != 0));
    }

    fn list(paras: &[(usize, pb_core::ListKind, u8)]) -> Spans {
        let paras = paras
            .iter()
            .map(|&(index, kind, level)| pb_core::Para {
                index,
                style: pb_core::ParaStyle::list(kind, level),
            })
            .collect();
        Spans::new(vec![], vec![], paras)
    }

    /// The `n` list markers (the runs after the text's), top to bottom: where each starts
    /// and its glyph count.
    fn markers(layout: &TextLayout, n: usize) -> Vec<(f32, f32, usize)> {
        let mut out: Vec<_> = layout.runs[layout.runs.len() - n..]
            .iter()
            .filter_map(|r| {
                let g = r.glyphs.first()?;
                Some((g.x, g.y, r.glyphs.len()))
            })
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        out
    }

    #[test]
    fn list_items_hang_indented_behind_their_markers() {
        use pb_core::ListKind::{Bullet, Number};
        let mut engine = TextEngine::new();
        let text = "intro\none two three four five six\nnested";
        let spans = list(&[(1, Bullet, 0), (2, Bullet, 1)]);
        let plain =
            engine.layout_aligned(text, &Spans::default(), 20.0, Some(150.0), TextAlign::Start);
        let listed = engine.layout_aligned(text, &spans, 20.0, Some(150.0), TextAlign::Start);
        // Every line of the item, wrapped ones too, starts at the indent; the nested one
        // further in; the intro at the edge.
        let line_starts = |layout: &TextLayout| -> Vec<(f32, f32)> {
            let mut starts: Vec<(f32, f32)> = Vec::new();
            for g in layout
                .runs
                .iter()
                .filter(|r| r.glyphs.len() > 3)
                .flat_map(|r| &r.glyphs)
            {
                match starts.iter_mut().find(|(_, y)| (y - g.y).abs() < 0.1) {
                    Some(s) => s.0 = s.0.min(g.x),
                    None => starts.push((g.x, g.y)),
                }
            }
            starts.sort_by(|a, b| a.1.total_cmp(&b.1));
            starts
        };
        let starts = line_starts(&listed);
        assert!(starts.len() >= 4, "{starts:?}");
        let indent = 20.0 * LIST_INDENT;
        assert!(starts[0].0 < 1.0);
        for &(x, _) in &starts[1..starts.len() - 1] {
            assert!((x - indent).abs() < 1.0, "{starts:?}");
        }
        let nested = starts.last().unwrap().0;
        assert!(
            (nested - indent - 20.0 * LIST_STEP).abs() < 1.0,
            "{starts:?}"
        );
        // Bullets sit before the text on the first line of their item.
        let bullets = markers(&listed, 2);
        assert_eq!(bullets.len(), 2, "{bullets:?}");
        assert!(bullets[0].0 < indent && bullets[0].0 > 0.0);
        assert!((bullets[0].1 - starts[1].1).abs() < 0.1);
        assert!(bullets[1].0 > bullets[0].0);
        // Lists take room: the text is as tall, wrapped more.
        assert!(listed.height >= plain.height);
        // The marker glyphs are in the font (no missing-glyph boxes).
        for kind in [Bullet, Number] {
            for level in 0..3 {
                let one = engine.layout_aligned(
                    "x",
                    &list(&[(0, kind, level)]),
                    20.0,
                    None,
                    TextAlign::Start,
                );
                assert!(one.runs.iter().flat_map(|r| &r.glyphs).all(|g| g.id != 0));
            }
        }
    }

    #[test]
    fn numbers_start_again_after_a_plain_paragraph() {
        use pb_core::ListKind::Number;
        let mut engine = TextEngine::new();
        let text = "a\nb\nplain\nc";
        let spans = list(&[(0, Number, 0), (1, Number, 0), (3, Number, 0)]);
        let layout = engine.layout_aligned(text, &spans, 20.0, None, TextAlign::Start);
        let shown = markers(&layout, 3);
        assert_eq!(shown.len(), 3, "{shown:?}");
        // "1." "2." then "1." again: the first and last marker have the same glyphs.
        let glyph_ids = |i: usize| {
            let run = layout
                .runs
                .iter()
                .find(|r| {
                    r.glyphs
                        .first()
                        .is_some_and(|g| g.y == shown[i].1 && g.x == shown[i].0)
                })
                .unwrap();
            run.glyphs.iter().map(|g| g.id).collect::<Vec<_>>()
        };
        assert_eq!(glyph_ids(0), glyph_ids(2));
        assert_ne!(glyph_ids(0), glyph_ids(1));
    }

    #[test]
    fn links_are_underlined_in_the_link_color() {
        let mut engine = TextEngine::new();
        let spans = Spans::new(
            vec![],
            vec![pb_core::LinkSpan {
                start: 4,
                end: 15,
                marks: pb_core::Link::from_input("example.com"),
            }],
            vec![],
        );
        let layout = engine.layout_aligned("see example.com", &spans, 20.0, None, TextAlign::Start);
        assert!(layout.runs.iter().any(|r| r.brush == Brush::Link));
        assert!(layout.runs.iter().any(|r| r.brush == Brush::Text));
        assert_eq!(layout.decorations.len(), 1);
        assert_eq!(layout.decorations[0].brush, Brush::Link);
    }

    #[test]
    fn wraps_at_max_width() {
        let mut engine = TextEngine::new();
        let one_line = engine.layout("one two three", 20.0, None);
        let wrapped = engine.layout("one two three", 20.0, Some(60.0));
        assert!(wrapped.height > one_line.height * 2.0);
        assert!(wrapped.width <= 60.0 + 1e-3);
    }
}
