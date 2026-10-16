//! Editing one piece of text: cursor, selection, typing and IME composition.

use std::borrow::Cow;

use parley::{FontFamily, FontFamilyName, PlainEditor, PlainEditorDriver, StyleProperty};

use crate::{FAMILY, LINE_HEIGHT, TextAlign, TextEngine, TextLayout, extract};

/// A rectangle in layout coordinates (origin at the text's top-left corner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Cursor movements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    TextStart,
    TextEnd,
}

/// Width of the caret, in the text's units relative to the font size.
const CARET_WIDTH: f32 = 0.08;

/// An editable text with the same layout as displayed text (see [`TextEngine::layout`]).
pub struct TextEditor {
    editor: PlainEditor<()>,
    size: f64,
}

impl TextEditor {
    /// Starts with the cursor at the end of `text`.
    pub fn new(engine: &mut TextEngine, text: &str, size: f64) -> Self {
        let mut editor = PlainEditor::new(size as f32);
        editor.set_text(text);
        editor.set_width(None);
        editor.set_alignment(parley::Alignment::Start);
        // Unrounded, like displayed text, so nothing shifts when editing ends.
        editor.set_quantize(false);
        let styles = editor.edit_styles();
        styles.insert(StyleProperty::FontFamily(FontFamily::Single(
            FontFamilyName::Named(Cow::Borrowed(FAMILY)),
        )));
        styles.insert(StyleProperty::LineHeight(
            parley::LineHeight::FontSizeRelative(LINE_HEIGHT),
        ));
        let mut this = Self { editor, size };
        this.driver(engine).move_to_text_end();
        this
    }

    fn driver<'a>(&'a mut self, engine: &'a mut TextEngine) -> PlainEditorDriver<'a, ()> {
        self.editor.driver(&mut engine.fonts, &mut engine.layouts)
    }

    pub fn size(&self) -> f64 {
        self.size
    }

    pub fn set_size(&mut self, size: f64) {
        if size != self.size {
            self.size = size;
            self.editor
                .edit_styles()
                .insert(StyleProperty::FontSize(size as f32));
        }
    }

    /// Wraps lines at `width` and aligns them within it, matching
    /// [`TextEngine::layout_aligned`] with the same arguments.
    pub fn set_wrap(&mut self, width: Option<f64>, align: TextAlign) {
        self.editor.set_width(width.map(|w| w as f32));
        self.editor.set_alignment(align.to_parley());
    }

    /// The text without any IME composition in progress.
    pub fn text(&self) -> String {
        self.editor.text().to_string()
    }

    /// The text as shown, including IME composition.
    pub fn display_text(&self) -> String {
        self.editor.raw_text().to_owned()
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editor.selected_text().map(str::to_owned)
    }

    pub fn is_composing(&self) -> bool {
        self.editor.is_composing()
    }

    /// Types `text` over the selection.
    pub fn insert(&mut self, engine: &mut TextEngine, text: &str) {
        self.driver(engine).insert_or_replace_selection(text);
    }

    /// Backspace: deletes the selection, or the character (or word) before the cursor.
    pub fn backspace(&mut self, engine: &mut TextEngine, word: bool) {
        let mut driver = self.driver(engine);
        if word {
            driver.backdelete_word();
        } else {
            driver.backdelete();
        }
    }

    /// Delete: deletes the selection, or the character (or word) after the cursor.
    pub fn delete(&mut self, engine: &mut TextEngine, word: bool) {
        let mut driver = self.driver(engine);
        if word {
            driver.delete_word();
        } else {
            driver.delete();
        }
    }

    /// Moves the cursor; `extend` (Shift) grows the selection instead.
    pub fn motion(&mut self, engine: &mut TextEngine, motion: Motion, extend: bool) {
        let mut d = self.driver(engine);
        match (motion, extend) {
            (Motion::Left, false) => d.move_left(),
            (Motion::Left, true) => d.select_left(),
            (Motion::Right, false) => d.move_right(),
            (Motion::Right, true) => d.select_right(),
            (Motion::Up, false) => d.move_up(),
            (Motion::Up, true) => d.select_up(),
            (Motion::Down, false) => d.move_down(),
            (Motion::Down, true) => d.select_down(),
            (Motion::WordLeft, false) => d.move_word_left(),
            (Motion::WordLeft, true) => d.select_word_left(),
            (Motion::WordRight, false) => d.move_word_right(),
            (Motion::WordRight, true) => d.select_word_right(),
            (Motion::LineStart, false) => d.move_to_line_start(),
            (Motion::LineStart, true) => d.select_to_line_start(),
            (Motion::LineEnd, false) => d.move_to_line_end(),
            (Motion::LineEnd, true) => d.select_to_line_end(),
            (Motion::TextStart, false) => d.move_to_text_start(),
            (Motion::TextStart, true) => d.select_to_text_start(),
            (Motion::TextEnd, false) => d.move_to_text_end(),
            (Motion::TextEnd, true) => d.select_to_text_end(),
        }
    }

    pub fn select_all(&mut self, engine: &mut TextEngine) {
        self.driver(engine).select_all();
    }

    /// A click at a point in layout coordinates; `extend` (Shift) selects up to it.
    pub fn click(&mut self, engine: &mut TextEngine, x: f64, y: f64, extend: bool) {
        let mut driver = self.driver(engine);
        if extend {
            driver.shift_click_extension(x as f32, y as f32);
        } else {
            driver.move_to_point(x as f32, y as f32);
        }
    }

    /// Dragging after a click selects up to the point.
    pub fn drag_to(&mut self, engine: &mut TextEngine, x: f64, y: f64) {
        self.driver(engine)
            .extend_selection_to_point(x as f32, y as f32);
    }

    pub fn select_word_at(&mut self, engine: &mut TextEngine, x: f64, y: f64) {
        self.driver(engine).select_word_at_point(x as f32, y as f32);
    }

    /// Shows IME composition `text` at the cursor; `cursor` is a byte range within it.
    pub fn set_preedit(
        &mut self,
        engine: &mut TextEngine,
        text: &str,
        cursor: Option<(usize, usize)>,
    ) {
        let mut driver = self.driver(engine);
        if text.is_empty() {
            driver.clear_compose();
        } else {
            driver.set_compose(text, cursor);
        }
    }

    pub fn clear_preedit(&mut self, engine: &mut TextEngine) {
        self.driver(engine).clear_compose();
    }

    /// Glyphs and size of the text as shown.
    pub fn layout(&mut self, engine: &mut TextEngine) -> TextLayout {
        extract(self.editor.layout(&mut engine.fonts, &mut engine.layouts))
    }

    /// The caret, unless a selection or composition hides it.
    pub fn caret(&mut self, engine: &mut TextEngine) -> Option<TextRect> {
        self.editor
            .refresh_layout(&mut engine.fonts, &mut engine.layouts);
        let width = (self.size as f32 * CARET_WIDTH).max(1.0);
        self.editor.cursor_geometry(width).map(rect)
    }

    pub fn selection(&mut self, engine: &mut TextEngine) -> Vec<TextRect> {
        self.editor
            .refresh_layout(&mut engine.fonts, &mut engine.layouts);
        self.editor
            .selection_geometry()
            .into_iter()
            .map(|(b, _)| rect(b))
            .collect()
    }

    /// Where the IME should show its candidate window.
    pub fn ime_area(&mut self, engine: &mut TextEngine) -> TextRect {
        self.editor
            .refresh_layout(&mut engine.fonts, &mut engine.layouts);
        rect(self.editor.ime_cursor_area())
    }
}

fn rect(b: parley::BoundingBox) -> TextRect {
    TextRect {
        x0: b.x0,
        y0: b.y0,
        x1: b.x1,
        y1: b.y1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_deleting() {
        let mut engine = TextEngine::new();
        let mut ed = TextEditor::new(&mut engine, "Hello", 20.0);
        ed.insert(&mut engine, ", мир");
        assert_eq!(ed.text(), "Hello, мир");
        ed.backspace(&mut engine, false);
        assert_eq!(ed.text(), "Hello, ми");
        ed.backspace(&mut engine, true);
        assert_eq!(ed.text(), "Hello, ");
        ed.motion(&mut engine, Motion::TextStart, false);
        ed.delete(&mut engine, false);
        assert_eq!(ed.text(), "ello, ");
    }

    #[test]
    fn selection_replaces_on_typing() {
        let mut engine = TextEngine::new();
        let mut ed = TextEditor::new(&mut engine, "one two", 20.0);
        ed.motion(&mut engine, Motion::WordLeft, true);
        assert_eq!(ed.selected_text().as_deref(), Some("two"));
        assert!(!ed.selection(&mut engine).is_empty());
        assert!(ed.caret(&mut engine).is_some());
        ed.insert(&mut engine, "три");
        assert_eq!(ed.text(), "one три");
        ed.select_all(&mut engine);
        ed.insert(&mut engine, "x");
        assert_eq!(ed.text(), "x");
    }

    #[test]
    fn newlines_and_layout_match_display() {
        let mut engine = TextEngine::new();
        let mut ed = TextEditor::new(&mut engine, "a", 20.0);
        ed.insert(&mut engine, "\nbb");
        let edited = ed.layout(&mut engine);
        let shown = engine.layout("a\nbb", 20.0, None);
        assert_eq!(edited, shown);
        assert!(edited.height > 40.0);
    }

    #[test]
    fn wrapped_centered_layout_matches_display() {
        let mut engine = TextEngine::new();
        let mut ed = TextEditor::new(&mut engine, "one two three four", 20.0);
        ed.set_wrap(Some(90.0), TextAlign::Center);
        ed.set_size(16.0);
        let shown =
            engine.layout_aligned("one two three four", 16.0, Some(90.0), TextAlign::Center);
        assert_eq!(ed.layout(&mut engine), shown);
        assert_eq!(ed.size(), 16.0);
    }

    #[test]
    fn clicks_place_the_cursor() {
        let mut engine = TextEngine::new();
        let mut ed = TextEditor::new(&mut engine, "abcdef", 20.0);
        ed.click(&mut engine, 0.0, 10.0, false);
        ed.insert(&mut engine, "_");
        assert_eq!(ed.text(), "_abcdef");
        let end = ed.layout(&mut engine).width + 5.0;
        ed.click(&mut engine, 0.0, 10.0, false);
        ed.click(&mut engine, end, 10.0, true);
        assert_eq!(ed.selected_text().as_deref(), Some("_abcdef"));
    }

    #[test]
    fn ime_composition_is_shown_but_not_committed() {
        let mut engine = TextEngine::new();
        let mut ed = TextEditor::new(&mut engine, "a", 20.0);
        ed.set_preedit(&mut engine, "日本", Some((6, 6)));
        assert!(ed.is_composing());
        assert_eq!(ed.display_text(), "a日本");
        assert_eq!(ed.text(), "a");
        ed.clear_preedit(&mut engine);
        ed.insert(&mut engine, "日本");
        assert_eq!(ed.text(), "a日本");
    }
}
