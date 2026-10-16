//! Editing one piece of text: cursor, selection, typing and IME composition.

use std::borrow::Cow;

use parley::{FontFamily, FontFamilyName, StyleProperty};

use pb_core::Rgba;
use pb_core::rich::{self, Span, TextMarks};

use crate::rich_editor::{RichEditor, RichEditorDriver};

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

/// What an edit did, to merge runs of the same edit into one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    /// Paste, cut, a newline or an IME commit: always a step of its own.
    Other,
}

/// Text, styles and selection (anchor, focus as byte offsets) before an edit.
#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    text: String,
    spans: Vec<Span>,
    anchor: usize,
    focus: usize,
}

/// A mark that is on or off, see [`TextMarks`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Underline,
    Strike,
}

impl Mark {
    fn get(self, m: &TextMarks) -> bool {
        match self {
            Self::Bold => m.bold,
            Self::Italic => m.italic,
            Self::Underline => m.underline,
            Self::Strike => m.strike,
        }
    }

    fn set(self, mut m: TextMarks, on: bool) -> TextMarks {
        match self {
            Self::Bold => m.bold = on,
            Self::Italic => m.italic = on,
            Self::Underline => m.underline = on,
            Self::Strike => m.strike = on,
        }
        m
    }
}

/// An editable text with the same layout as displayed text (see [`TextEngine::layout`]).
///
/// Keeps its own undo history while it lives: consecutive typing or deleting is one step,
/// and a space, a cursor move or another kind of edit starts the next.
pub struct TextEditor {
    editor: RichEditor,
    size: f64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The kind of the last edit, while further edits of that kind join its step.
    last_edit: Option<EditKind>,
}

impl TextEditor {
    /// Starts with the cursor at the end of `text`.
    pub fn new(engine: &mut TextEngine, text: &str, size: f64) -> Self {
        Self::with_spans(engine, text, &[], size)
    }

    /// Like [`Self::new`], for text with inline styles.
    pub fn with_spans(engine: &mut TextEngine, text: &str, spans: &[Span], size: f64) -> Self {
        let mut editor = RichEditor::new(size as f32);
        editor.set_text(text, spans);
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
        let mut this = Self {
            editor,
            size,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
        };
        this.driver(engine).move_to_text_end();
        this
    }

    fn driver<'a>(&'a mut self, engine: &'a mut TextEngine) -> RichEditorDriver<'a> {
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

    /// Inline styles of [`Self::text`]; meant for when no IME composition is in progress.
    pub fn spans(&self) -> Vec<Span> {
        self.editor.spans().to_vec()
    }

    /// Inline styles of [`Self::display_text`].
    pub fn display_spans(&self) -> Vec<Span> {
        self.editor.spans().to_vec()
    }

    fn selection_range(&self) -> std::ops::Range<usize> {
        self.editor.raw_selection().text_range()
    }

    /// The marks the selection has throughout (a mark is on only if all of it has it),
    /// or those the next typed text gets.
    pub fn marks(&self) -> TextMarks {
        let range = self.selection_range();
        if range.is_empty() {
            return self
                .editor
                .insert_marks()
                .unwrap_or_else(|| rich::marks_for_typing(self.editor.spans(), range.start));
        }
        let spans = self.editor.spans();
        let on = |mark: Mark| rich::all(spans, range.clone(), |m| mark.get(m));
        let first = rich::marks_at(spans, range.start).color;
        TextMarks {
            bold: on(Mark::Bold),
            italic: on(Mark::Italic),
            underline: on(Mark::Underline),
            strike: on(Mark::Strike),
            color: first.filter(|_| rich::all(spans, range.clone(), |m| m.color == first)),
        }
    }

    /// Turns `mark` on for the selection, or off if all of it has it already. With nothing
    /// selected, it is for the text typed next.
    pub fn toggle(&mut self, engine: &mut TextEngine, mark: Mark) {
        let on = !mark.get(&self.marks());
        self.restyle(engine, |m| mark.set(m, on));
    }

    /// Colors the selection, or the text typed next; `None` for the text's own color.
    pub fn set_color(&mut self, engine: &mut TextEngine, color: Option<Rgba>) {
        self.restyle(engine, |m| TextMarks { color, ..m });
    }

    fn restyle(&mut self, engine: &mut TextEngine, change: impl Fn(TextMarks) -> TextMarks) {
        let range = self.selection_range();
        if range.is_empty() {
            let marks = change(self.marks());
            self.editor.set_insert_marks(Some(marks));
            // Text typed in the new style is a step of its own.
            self.last_edit = None;
            return;
        }
        let before = self.snapshot();
        let len = self.editor.raw_text().len();
        let spans = rich::apply(self.editor.spans(), range, len, change);
        self.editor.set_spans(spans);
        self.driver(engine).refresh_layout();
        if self.editor.spans() != before.spans.as_slice() {
            self.undo.push(before);
            self.redo.clear();
            self.last_edit = None;
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editor.selected_text().map(str::to_owned)
    }

    pub fn is_composing(&self) -> bool {
        self.editor.is_composing()
    }

    fn snapshot(&self) -> Snapshot {
        let selection = self.editor.raw_selection();
        Snapshot {
            text: self.text(),
            spans: self.spans(),
            anchor: selection.anchor().index(),
            focus: selection.focus().index(),
        }
    }

    fn restore(&mut self, engine: &mut TextEngine, snapshot: &Snapshot) {
        self.editor.set_text(&snapshot.text, &snapshot.spans);
        self.driver(engine)
            .select_byte_range(snapshot.anchor, snapshot.focus);
    }

    /// Runs an edit, recording an undo step if it changed the text and does not continue
    /// the previous step.
    fn edit(&mut self, kind: EditKind, joins: bool, f: impl FnOnce(&mut Self)) {
        let before = self.snapshot();
        f(self);
        if self.editor.text() == before.text.as_str()
            && self.editor.spans() == before.spans.as_slice()
        {
            return;
        }
        let continues = joins && kind != EditKind::Other && self.last_edit == Some(kind);
        if !continues {
            self.undo.push(before);
        }
        self.redo.clear();
        self.last_edit = Some(kind);
    }

    /// Ends the current undo step, so the next edit starts another; the cursor moved, so
    /// marks set for typing there are dropped.
    fn break_step(&mut self) {
        self.last_edit = None;
        self.editor.set_insert_marks(None);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Reverts the last undo step; returns false when there is none.
    pub fn undo(&mut self, engine: &mut TextEngine) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(engine, &snapshot);
        self.break_step();
        true
    }

    /// Repeats the last undone step; returns false when there is none.
    pub fn redo(&mut self, engine: &mut TextEngine) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(engine, &snapshot);
        self.break_step();
        true
    }

    /// Types `text` over the selection. Typing joins the previous step unless it starts a
    /// word or contains a line break.
    pub fn insert(&mut self, engine: &mut TextEngine, text: &str) {
        let typing = text.chars().count() == 1 && text != "\n";
        let kind = if typing {
            EditKind::Typing
        } else {
            EditKind::Other
        };
        // A word starts after whitespace, so each word is a step.
        let after_space = self.editor.raw_text()[..self.cursor_index()]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        let joins = !(after_space && !text.starts_with(char::is_whitespace));
        let replaces = !self.editor.raw_selection().is_collapsed();
        self.edit(kind, joins && !replaces, |this| {
            this.driver(engine).insert_or_replace_selection(text);
        });
    }

    fn cursor_index(&self) -> usize {
        let raw = self.editor.raw_text();
        let mut index = self.editor.raw_selection().focus().index().min(raw.len());
        while !raw.is_char_boundary(index) {
            index -= 1;
        }
        index
    }

    /// Backspace: deletes the selection, or the character (or word) before the cursor.
    pub fn backspace(&mut self, engine: &mut TextEngine, word: bool) {
        let joins = !word && self.editor.raw_selection().is_collapsed();
        self.edit(EditKind::Deleting, joins, |this| {
            let mut driver = this.driver(engine);
            if word {
                driver.backdelete_word();
            } else {
                driver.backdelete();
            }
        });
    }

    /// Delete: deletes the selection, or the character (or word) after the cursor.
    pub fn delete(&mut self, engine: &mut TextEngine, word: bool) {
        let joins = !word && self.editor.raw_selection().is_collapsed();
        self.edit(EditKind::Deleting, joins, |this| {
            let mut driver = this.driver(engine);
            if word {
                driver.delete_word();
            } else {
                driver.delete();
            }
        });
    }

    /// Moves the cursor; `extend` (Shift) grows the selection instead.
    pub fn motion(&mut self, engine: &mut TextEngine, motion: Motion, extend: bool) {
        self.break_step();
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
        self.break_step();
        self.driver(engine).select_all();
    }

    /// A click at a point in layout coordinates; `extend` (Shift) selects up to it.
    pub fn click(&mut self, engine: &mut TextEngine, x: f64, y: f64, extend: bool) {
        self.break_step();
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
        self.break_step();
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

    fn type_chars(ed: &mut TextEditor, engine: &mut TextEngine, text: &str) {
        for c in text.chars() {
            ed.insert(engine, &c.to_string());
        }
    }

    #[test]
    fn undo_steps_are_words_and_runs_of_deletes() {
        let mut engine = TextEngine::new();
        let mut ed = TextEditor::new(&mut engine, "", 20.0);
        assert!(!ed.can_undo());
        type_chars(&mut ed, &mut engine, "один два");
        ed.backspace(&mut engine, false);
        ed.backspace(&mut engine, false);
        assert_eq!(ed.text(), "один д");
        assert!(ed.undo(&mut engine));
        assert_eq!(ed.text(), "один два");
        assert!(ed.undo(&mut engine));
        // A space ends the word it follows.
        assert_eq!(ed.text(), "один ");
        assert!(ed.undo(&mut engine));
        assert_eq!(ed.text(), "");
        assert!(!ed.undo(&mut engine));
        assert!(ed.redo(&mut engine));
        assert!(ed.redo(&mut engine));
        assert_eq!(ed.text(), "один два");
        // The cursor is back where it was, so typing goes on at the end.
        ed.insert(&mut engine, "!");
        assert_eq!(ed.text(), "один два!");
        assert!(!ed.redo(&mut engine), "a new edit drops what was undone");
    }

    #[test]
    fn moving_the_cursor_and_pasting_start_new_steps() {
        let mut engine = TextEngine::new();
        let mut ed = TextEditor::new(&mut engine, "ab", 20.0);
        type_chars(&mut ed, &mut engine, "cd");
        ed.motion(&mut engine, Motion::TextStart, false);
        type_chars(&mut ed, &mut engine, "x");
        ed.insert(&mut engine, "pasted");
        assert_eq!(ed.text(), "xpastedabcd");
        ed.undo(&mut engine);
        assert_eq!(ed.text(), "xabcd");
        ed.undo(&mut engine);
        assert_eq!(ed.text(), "abcd");
        ed.undo(&mut engine);
        assert_eq!(ed.text(), "ab");
        // Deleting nothing is not a step.
        ed.motion(&mut engine, Motion::TextStart, false);
        ed.backspace(&mut engine, false);
        assert!(!ed.can_undo());
        // Typing over a selection starts a step, and the selection comes back with it.
        type_chars(&mut ed, &mut engine, "q");
        ed.select_all(&mut engine);
        type_chars(&mut ed, &mut engine, "zz");
        ed.undo(&mut engine);
        assert_eq!(ed.text(), "qab");
        assert_eq!(ed.selected_text().as_deref(), Some("qab"));
    }

    #[test]
    fn marks_apply_to_the_selection_or_what_is_typed_next() {
        let mut engine = TextEngine::new();
        let mut ed = TextEditor::new(&mut engine, "one two", 20.0);
        // Select "two" and make it bold: one undo step.
        ed.motion(&mut engine, Motion::WordLeft, true);
        ed.toggle(&mut engine, Mark::Bold);
        assert!(ed.marks().bold);
        assert_eq!(ed.spans().len(), 1);
        assert_eq!(ed.spans()[0].range(), 4..7);
        // Typing after it continues bold.
        ed.motion(&mut engine, Motion::TextEnd, false);
        type_chars(&mut ed, &mut engine, "s");
        assert_eq!(ed.spans()[0].range(), 4..8);
        // Bold off for what comes next, then plain typing.
        ed.toggle(&mut engine, Mark::Bold);
        type_chars(&mut ed, &mut engine, "!");
        assert_eq!(ed.text(), "one twos!");
        assert_eq!(ed.spans()[0].range(), 4..8);
        assert!(!ed.marks().bold);
        // The layout as edited matches the layout as shown.
        let shown = engine.layout_aligned(&ed.text(), &ed.spans(), 20.0, None, TextAlign::Start);
        assert_eq!(ed.layout(&mut engine), shown);
        // Undo: the "!", the "s", then the bold.
        ed.undo(&mut engine);
        ed.undo(&mut engine);
        assert_eq!(ed.text(), "one two");
        assert_eq!(ed.spans()[0].range(), 4..7);
        ed.undo(&mut engine);
        assert!(ed.spans().is_empty());
        // A color over part of the text, and the selection reports it only if uniform.
        let red = Rgba::rgb(0xF2, 0x55, 0x55);
        ed.select_all(&mut engine);
        ed.set_color(&mut engine, Some(red));
        assert_eq!(ed.marks().color, Some(red));
        ed.motion(&mut engine, Motion::TextStart, false);
        ed.motion(&mut engine, Motion::WordRight, true);
        ed.set_color(&mut engine, None);
        ed.select_all(&mut engine);
        assert_eq!(ed.marks().color, None);
    }

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
        let shown = engine.layout_aligned(
            "one two three four",
            &[],
            16.0,
            Some(90.0),
            TextAlign::Center,
        );
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
