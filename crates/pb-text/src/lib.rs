//! Text layout: shaping, line breaking and glyph positions, independent of rendering.
//!
//! All text uses Inter (SIL Open Font License, see `assets/fonts/Inter-OFL.txt`), which
//! covers Latin, Cyrillic and Greek, in regular, bold, italic and bold italic.
//!
//! Markdown documents are laid out in [`markdown`].

mod editor;
pub mod markdown;
mod rich_editor;

pub use editor::{Mark, Motion, TextEditor, TextRect};

use std::borrow::Cow;
use std::sync::Arc;

use parley::fontique::Blob;
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, FontStyle, FontWeight,
    Layout, LayoutContext, PositionedLayoutItem, StyleProperty,
};
use pb_core::{Rgba, Span};

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

/// What text is drawn with: its own color, or `None` for the text's default color.
pub type Brush = Option<Rgba>;

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
        self.layout_aligned(text, &[], size, max_width, TextAlign::Start)
    }

    /// Like [`Self::layout`], with inline styles and lines aligned within `max_width`.
    pub fn layout_aligned(
        &mut self,
        text: &str,
        spans: &[Span],
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
        for span in spans {
            for prop in span_styles(span) {
                builder.push(prop, span.range());
            }
        }
        let mut layout: Layout<Brush> = builder.build(text);
        layout.break_all_lines(max_width.map(|w| w as f32));
        layout.align(align.to_parley(), AlignmentOptions::default());
        extract(&layout)
    }

    /// The largest font size, up to `max_size`, at which `text` with `spans` wrapped at
    /// `width` fits in `width` x `height`. Words are not broken, so a long word shrinks the
    /// text instead.
    pub fn fit_size(
        &mut self,
        text: &str,
        spans: &[Span],
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
        props.push(StyleProperty::Brush(Some(color)));
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
        let start = engine.layout_aligned("hi", &[], 20.0, Some(200.0), TextAlign::Start);
        let center = engine.layout_aligned("hi", &[], 20.0, Some(200.0), TextAlign::Center);
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
        assert_eq!(engine.fit_size("ok", &[], 200.0, 200.0, 30.0), 30.0);
        assert_eq!(engine.fit_size("", &[], 200.0, 200.0, 30.0), 30.0);
        let long = "a fairly long sentence that cannot fit on a note at full size ".repeat(4);
        let size = engine.fit_size(&long, &[], 200.0, 200.0, 30.0);
        assert!(size < 30.0 && size > 5.0, "{size}");
        let layout = engine.layout(&long, size, Some(200.0));
        assert!(layout.height <= 200.0 && layout.content_width <= 200.0);
        // Spaces hanging at wrapped line ends do not count, so the text fills the note.
        assert!(layout.height > 150.0, "{}", layout.height);
        // A single long word shrinks to the width instead of overflowing.
        let word = engine.fit_size("Donaudampfschifffahrt", &[], 100.0, 100.0, 30.0);
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
        let styled = engine.layout_aligned("bold", &[span(bold)], 20.0, None, TextAlign::Start);
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
        let styled = engine.layout_aligned("bold", &[span(marks)], 20.0, None, TextAlign::Start);
        assert_eq!(styled.runs[0].brush, Some(red));
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
        let ru = engine.layout_aligned(
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

    #[test]
    fn wraps_at_max_width() {
        let mut engine = TextEngine::new();
        let one_line = engine.layout("one two three", 20.0, None);
        let wrapped = engine.layout("one two three", 20.0, Some(60.0));
        assert!(wrapped.height > one_line.height * 2.0);
        assert!(wrapped.width <= 60.0 + 1e-3);
    }
}
