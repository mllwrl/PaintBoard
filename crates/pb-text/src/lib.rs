//! Text layout: shaping, line breaking and glyph positions, independent of rendering.
//!
//! All text uses Inter (SIL Open Font License, see `assets/fonts/Inter-OFL.txt`), which
//! covers Latin, Cyrillic and Greek.
//!
//! TODO: rich text (bold, italic, lists, links).

mod editor;

pub use editor::{Motion, TextEditor, TextRect};

use std::borrow::Cow;
use std::sync::Arc;

use parley::fontique::Blob;
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, Layout, LayoutContext,
    PositionedLayoutItem, StyleProperty,
};

/// The font for board text and the UI.
pub const FONT: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const FAMILY: &str = "Inter";
/// Line height relative to the font size.
const LINE_HEIGHT: f32 = 1.2;

/// A glyph of [`FONT`] placed on its baseline, relative to the layout's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayout {
    pub width: f64,
    pub height: f64,
    pub glyphs: Vec<Glyph>,
}

/// Shapes and lays out text. Keeps font data and caches between calls.
pub struct TextEngine {
    pub(crate) fonts: FontContext,
    pub(crate) layouts: LayoutContext<()>,
}

impl TextEngine {
    pub fn new() -> Self {
        let mut fonts = FontContext::new();
        fonts
            .collection
            .register_fonts(Blob::new(Arc::new(FONT)), None);
        Self {
            fonts,
            layouts: LayoutContext::new(),
        }
    }

    /// Lays out `text` at font `size`, wrapping lines at `max_width` if given. Lines start
    /// at the left edge, as in [`TextEditor`].
    pub fn layout(&mut self, text: &str, size: f64, max_width: Option<f64>) -> TextLayout {
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::Single(
            FontFamilyName::Named(Cow::Borrowed(FAMILY)),
        )));
        builder.push_default(StyleProperty::FontSize(size as f32));
        builder.push_default(StyleProperty::LineHeight(
            parley::LineHeight::FontSizeRelative(LINE_HEIGHT),
        ));
        let mut layout: Layout<()> = builder.build(text);
        layout.break_all_lines(max_width.map(|w| w as f32));
        layout.align(Alignment::Start, AlignmentOptions::default());
        extract(&layout)
    }
}

/// Glyph positions and size of a finished layout.
pub(crate) fn extract(layout: &Layout<()>) -> TextLayout {
    let mut glyphs = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(run) = item {
                glyphs.extend(run.positioned_glyphs().map(|g| Glyph {
                    id: g.id,
                    x: g.x,
                    y: g.y,
                }));
            }
        }
    }
    TextLayout {
        width: f64::from(layout.full_width()),
        height: f64::from(layout.height()),
        glyphs,
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
        assert_eq!(layout.glyphs.len(), 13);
        // Inter has these glyphs, so none is the missing-glyph box (id 0).
        assert!(layout.glyphs.iter().all(|g| g.id != 0));
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
    fn wraps_at_max_width() {
        let mut engine = TextEngine::new();
        let one_line = engine.layout("one two three", 20.0, None);
        let wrapped = engine.layout("one two three", 20.0, Some(60.0));
        assert!(wrapped.height > one_line.height * 2.0);
        assert!(wrapped.width <= 60.0 + 1e-3);
    }
}
