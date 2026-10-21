//! Editing one piece of text: cursor, selection, typing and IME composition.

use std::borrow::Cow;
use std::ops::Range;

use parley::{FontFamily, FontFamilyName, StyleProperty};

use pb_core::rich::{self, Span, TextMarks};
use pb_core::{Link, LinkSpan, ListKind, Para, ParaStyle, Rgba, Spans};

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

impl TextRect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
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
    spans: Spans,
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
    pub fn get(self, m: &TextMarks) -> bool {
        match self {
            Self::Bold => m.bold,
            Self::Italic => m.italic,
            Self::Underline => m.underline,
            Self::Strike => m.strike,
        }
    }

    pub fn set(self, mut m: TextMarks, on: bool) -> TextMarks {
        match self {
            Self::Bold => m.bold = on,
            Self::Italic => m.italic = on,
            Self::Underline => m.underline = on,
            Self::Strike => m.strike = on,
        }
        m
    }
}

/// What starts a list when typed at the start of a paragraph and followed by a space.
const LIST_PREFIXES: [(&str, ListKind); 4] = [
    ("-", ListKind::Bullet),
    ("*", ListKind::Bullet),
    ("1.", ListKind::Number),
    ("1)", ListKind::Number),
];

/// An editable text with the same layout as displayed text (see [`TextEngine::layout`]).
///
/// Keeps its own undo history while it lives: consecutive typing or deleting is one step,
/// and a space, a cursor move or another kind of edit starts the next.
pub struct TextEditor {
    editor: RichEditor,
    size: f64,
    /// Where lines wrap and how they sit, as last set.
    wrap: (Option<f64>, TextAlign),
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The kind of the last edit, while further edits of that kind join its step.
    last_edit: Option<EditKind>,
    /// Typing turns `- `, `1. ` and web addresses into lists and links.
    auto_format: bool,
}

impl TextEditor {
    /// Starts with the cursor at the end of `text`.
    pub fn new(engine: &mut TextEngine, text: &str, size: f64) -> Self {
        let mut this = Self::with_spans(engine, text, &Spans::default(), size);
        this.auto_format = false;
        this
    }

    /// Like [`Self::new`], for text with styles, links and lists, which typing can add.
    pub fn with_spans(engine: &mut TextEngine, text: &str, spans: &Spans, size: f64) -> Self {
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
            wrap: (None, TextAlign::Start),
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            auto_format: true,
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
            self.editor.set_font_size(size as f32);
        }
    }

    /// Wraps lines at `width` and aligns them within it, matching
    /// [`TextEngine::layout_aligned`] with the same arguments.
    pub fn set_wrap(&mut self, width: Option<f64>, align: TextAlign) {
        self.wrap = (width, align);
        self.editor.set_width(width.map(|w| w as f32));
        self.editor.set_alignment(align.to_parley());
    }

    /// Takes the size and wrapping of `other`, keeping this editor's text and history.
    pub fn match_layout(&mut self, other: &TextEditor) {
        self.set_size(other.size);
        self.set_wrap(other.wrap.0, other.wrap.1);
    }

    /// The text without any IME composition in progress.
    pub fn text(&self) -> String {
        self.editor.text().to_string()
    }

    /// The text as shown, including IME composition.
    pub fn display_text(&self) -> String {
        self.editor.raw_text().to_owned()
    }

    /// Styles, links and lists of [`Self::text`]; meant for when no IME composition is in
    /// progress.
    pub fn spans(&self) -> Spans {
        self.editor.spans().clone()
    }

    /// Styles, links and lists of [`Self::display_text`].
    pub fn display_spans(&self) -> Spans {
        self.editor.spans().clone()
    }

    fn selection_range(&self) -> Range<usize> {
        self.editor.raw_selection().text_range()
    }

    /// The marks the selection has throughout (a mark is on only if all of it has it),
    /// or those the next typed text gets.
    pub fn marks(&self) -> TextMarks {
        let range = self.selection_range();
        if range.is_empty() {
            return self.editor.insert_marks().unwrap_or_else(|| {
                rich::marks_for_typing(&self.editor.spans().marks, range.start)
            });
        }
        let spans = &self.editor.spans().marks;
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
        let len = self.editor.raw_text().len();
        let spans = self.editor.spans();
        let marks = rich::apply(&spans.marks, range, len, change);
        let spans = spans.with_marks(marks);
        self.set_spans_step(engine, spans);
    }

    /// Replaces the styles, links and lists as an undo step of its own, if they change.
    fn set_spans_step(&mut self, engine: &mut TextEngine, spans: Spans) {
        self.edit(EditKind::Other, false, |this| {
            this.editor.set_spans(spans);
            this.driver(engine).refresh_layout();
        });
        self.last_edit = None;
    }

    /// Paragraphs (by index) the selection touches; a selection ending right after a line
    /// break does not reach into the next paragraph.
    fn selected_paragraphs(&self) -> Range<usize> {
        let text = self.editor.raw_text();
        let range = self.selection_range();
        let mut end = range.end;
        if end > range.start && text.as_bytes().get(end - 1) == Some(&b'\n') {
            end -= 1;
        }
        rich::paragraph_of(text, range.start)..rich::paragraph_of(text, end) + 1
    }

    /// The style of the paragraph with the cursor (the selection's start).
    pub fn para_style(&self) -> ParaStyle {
        let text = self.editor.raw_text();
        let start = self.selection_range().start;
        self.editor.spans().para(rich::paragraph_of(text, start))
    }

    /// Changes the style of the selected paragraphs, as one undo step.
    fn restyle_paras(&mut self, engine: &mut TextEngine, change: impl Fn(ParaStyle) -> ParaStyle) {
        let spans = self.editor.spans();
        let mut paras = spans.paras.to_vec();
        for index in self.selected_paragraphs() {
            let style = change(spans.para(index));
            paras.push(Para { index, style });
        }
        let spans = spans.with_paras(paras);
        self.set_spans_step(engine, spans);
    }

    /// Makes the selected paragraphs `kind` list items, or plain ones if they all are.
    pub fn toggle_list(&mut self, engine: &mut TextEngine, kind: ListKind) {
        let spans = self.editor.spans();
        let all = self
            .selected_paragraphs()
            .all(|i| spans.para(i).list == Some(kind));
        self.restyle_paras(engine, |style| {
            if all {
                ParaStyle::default()
            } else {
                ParaStyle::list(kind, style.level)
            }
        });
    }

    /// Tab (or Shift+Tab, `out`) in list items: nests them a level deeper (or less; out of
    /// the list from the top level). False, changing nothing, when no selected paragraph
    /// is a list item.
    pub fn indent(&mut self, engine: &mut TextEngine, out: bool) -> bool {
        let spans = self.editor.spans();
        if self
            .selected_paragraphs()
            .all(|i| spans.para(i).list.is_none())
        {
            return false;
        }
        self.restyle_paras(engine, |style| match style.list {
            None => style,
            Some(_) if out && style.level == 0 => ParaStyle::default(),
            Some(kind) if out => ParaStyle::list(kind, style.level - 1),
            Some(kind) => ParaStyle::list(kind, style.level.saturating_add(1)),
        });
        true
    }

    /// The link at the selection's start, or at the cursor, and the text it covers.
    pub fn link(&self) -> Option<(Range<usize>, Link)> {
        let range = self.selection_range();
        let spans = self.editor.spans();
        let at = |i: usize| {
            spans
                .links
                .iter()
                .find(|s| s.start <= i && i < s.end)
                .and_then(|s| Some((s.range(), s.marks.clone()?)))
        };
        at(range.start).or_else(|| {
            range
                .is_empty()
                .then(|| range.start.checked_sub(1).and_then(at))
                .flatten()
        })
    }

    /// Links the selection to `link`, or unlinks it with `None`. With nothing selected, it
    /// is the link at the cursor that changes; or, outside any, the web address is typed
    /// in, linked.
    pub fn set_link(&mut self, engine: &mut TextEngine, link: Option<Link>) {
        let mut range = self.selection_range();
        if range.is_empty() {
            match (self.link(), &link) {
                (Some((r, _)), _) => range = r,
                (None, Some(Link::Url(url))) => {
                    let start = range.start;
                    let shown = url.strip_prefix("https://").unwrap_or(url).to_owned();
                    self.insert(engine, &shown);
                    range = start..start + shown.len();
                }
                (None, _) => return,
            }
        }
        let len = self.editor.raw_text().len();
        let spans = self.editor.spans();
        let links = rich::apply(&spans.links, range, len, |_| link.clone());
        let spans = spans.with_links(links);
        self.set_spans_step(engine, spans);
    }

    /// The link drawn at a point in layout coordinates, if any.
    pub fn link_at(&mut self, engine: &mut TextEngine, x: f64, y: f64) -> Option<Link> {
        self.editor
            .refresh_layout(&mut engine.fonts, &mut engine.layouts);
        let links: Vec<LinkSpan> = self.editor.spans().links.to_vec();
        links.into_iter().find_map(|s| {
            let hit = self
                .editor
                .range_geometry(s.range())
                .into_iter()
                .any(|b| rect(b).contains(x, y));
            if hit { s.marks } else { None }
        })
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editor.selected_text().map(str::to_owned)
    }

    /// The selected text with its styles, links and lists, for pasting here again.
    pub fn selected_rich(&self) -> Option<(String, Spans)> {
        let text = self.editor.selected_text()?.to_owned();
        let range = self.selection_range();
        let spans = self.editor.spans().slice(self.editor.raw_text(), range);
        Some((text, spans))
    }

    /// Pastes `text` that keeps `spans` (see [`Self::selected_rich`]).
    pub fn paste_rich(&mut self, engine: &mut TextEngine, text: &str, spans: &Spans) {
        self.editor.set_pasted(Some(spans.clone()));
        self.insert(engine, text);
        self.editor.set_pasted(None);
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
        if self.editor.text() == before.text.as_str() && *self.editor.spans() == before.spans {
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

    /// Takes in a change someone else made to the text meanwhile: the text becomes `base`
    /// (the text when editing started) with both their change (`theirs`) and the one being
    /// typed here, the cursor and styles staying with the text around them. Undo within
    /// the edit starts over, as its steps were of the text before.
    pub fn rebase(&mut self, engine: &mut TextEngine, base: &str, theirs: &str) {
        let now = self.snapshot();
        let (text, map) = crate::merge::merge(base, &now.text, theirs);
        let moved = |s: &Span| Span {
            start: map(s.start),
            end: map(s.end),
            marks: s.marks,
        };
        let marks = now.spans.marks.iter().map(moved).collect();
        let links = now
            .spans
            .links
            .iter()
            .map(|s| LinkSpan {
                start: map(s.start),
                end: map(s.end),
                marks: s.marks.clone(),
            })
            .collect();
        let spans = Spans::new(marks, links, now.spans.paras.to_vec()).fit_to(&text);
        self.editor.set_text(&text, &spans);
        self.driver(engine)
            .select_byte_range(map(now.anchor), map(now.focus));
        self.undo.clear();
        self.redo.clear();
        self.break_step();
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

    /// The paragraph with the cursor, when nothing is selected: its index and bytes.
    fn cursor_paragraph(&self) -> Option<(usize, Range<usize>)> {
        let range = self.selection_range();
        if !range.is_empty() || self.editor.is_composing() {
            return None;
        }
        let text = self.editor.raw_text();
        let index = rich::paragraph_of(text, range.start);
        Some((index, rich::paragraph_ranges(text)[index].clone()))
    }

    /// Types `text` over the selection. Typing joins the previous step unless it starts a
    /// word or contains a line break. Enter on an empty list item ends the list; a space
    /// after `- ` or `1. ` at the start of a paragraph starts one, and a space or Enter
    /// after a web address links it (each a step of its own, so undo takes it back).
    pub fn insert(&mut self, engine: &mut TextEngine, text: &str) {
        if text == "\n"
            && let Some((index, bytes)) = self.cursor_paragraph()
            && bytes.is_empty()
            && self.editor.spans().para(index).list.is_some()
        {
            self.restyle_paras(engine, |_| ParaStyle::default());
            return;
        }
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
        if self.auto_format && (text == " " || text == "\n") {
            self.auto_list(engine);
            self.auto_link(engine, text.len());
        } else if self.auto_format && !typing && !text.contains(char::is_whitespace) {
            // A pasted address is linked at once.
            self.auto_link(engine, 0);
        }
    }

    /// After a space typed at the start of a plain paragraph behind `-`, `*`, `1.` or
    /// `1)`: the paragraph becomes a list item instead.
    fn auto_list(&mut self, engine: &mut TextEngine) {
        let Some((index, bytes)) = self.cursor_paragraph() else {
            return;
        };
        let text = self.editor.raw_text();
        let cursor = self.selection_range().start;
        let typed = &text[bytes.start..cursor];
        let Some(kind) = LIST_PREFIXES
            .iter()
            .find(|(p, _)| typed.strip_suffix(' ') == Some(*p))
            .map(|&(_, kind)| kind)
        else {
            return;
        };
        if self.editor.spans().para(index).list.is_some() {
            return;
        }
        let prefix = bytes.start..cursor;
        self.edit(EditKind::Other, false, |this| {
            let mut driver = this.driver(engine);
            driver.select_byte_range(prefix.start, prefix.end);
            driver.delete_selection();
            let spans = this.editor.spans();
            let mut paras = spans.paras.to_vec();
            paras.push(Para {
                index,
                style: ParaStyle::list(kind, 0),
            });
            let spans = spans.with_paras(paras);
            this.editor.set_spans(spans);
            this.driver(engine).refresh_layout();
        });
        self.last_edit = None;
    }

    /// Links the web address just before the cursor, less the `typed` bytes after it,
    /// unless it is linked already.
    fn auto_link(&mut self, engine: &mut TextEngine, typed: usize) {
        let range = self.selection_range();
        if !range.is_empty() || range.start < typed {
            return;
        }
        let text = self.editor.raw_text();
        let end = range.start - typed;
        let start = text[..end]
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let Some((len, link)) = rich::detect_link(&text[start..end]) else {
            return;
        };
        let spans = self.editor.spans();
        let word = start..start + len;
        if spans
            .links
            .iter()
            .any(|s| s.start < word.end && s.end > word.start)
        {
            return;
        }
        let links = rich::apply(&spans.links, word, text.len(), |_| Some(link.clone()));
        let spans = spans.with_links(links);
        self.set_spans_step(engine, spans);
    }

    fn cursor_index(&self) -> usize {
        let raw = self.editor.raw_text();
        let mut index = self.editor.raw_selection().focus().index().min(raw.len());
        while !raw.is_char_boundary(index) {
            index -= 1;
        }
        index
    }

    /// Backspace: deletes the selection, or the character (or word) before the cursor. At
    /// the start of a list item it ends the list there instead.
    pub fn backspace(&mut self, engine: &mut TextEngine, word: bool) {
        if let Some((index, bytes)) = self.cursor_paragraph()
            && bytes.start == self.selection_range().start
            && self.editor.spans().para(index).list.is_some()
        {
            self.restyle_paras(engine, |_| ParaStyle::default());
            return;
        }
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

    /// Selects bytes `range` of the text.
    pub fn select(&mut self, engine: &mut TextEngine, range: Range<usize>) {
        self.break_step();
        self.driver(engine)
            .select_byte_range(range.start, range.end);
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

    /// Glyphs and size of the text as shown, list markers included.
    pub fn layout(&mut self, engine: &mut TextEngine) -> TextLayout {
        self.editor
            .refresh_layout(&mut engine.fonts, &mut engine.layouts);
        let layout = self.editor.try_layout().expect("a fresh layout");
        let mut out = extract(layout);
        let (text, spans) = (self.editor.raw_text(), self.editor.spans());
        engine.add_markers(&mut out, layout, text, spans, self.size);
        out.links = crate::link_areas(layout, spans);
        out
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
            &Spans::default(),
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

    fn rich(engine: &mut TextEngine, text: &str) -> TextEditor {
        TextEditor::with_spans(engine, text, &Spans::default(), 20.0)
    }

    fn lists(ed: &TextEditor) -> Vec<(usize, Option<ListKind>, u8)> {
        ed.spans()
            .paras
            .iter()
            .map(|p| (p.index, p.style.list, p.style.level))
            .collect()
    }

    const BULLET: Option<ListKind> = Some(ListKind::Bullet);
    const NUMBER: Option<ListKind> = Some(ListKind::Number);

    #[test]
    fn dash_or_number_and_space_start_a_list_and_enter_continues_it() {
        let mut engine = TextEngine::new();
        let mut ed = rich(&mut engine, "");
        type_chars(&mut ed, &mut engine, "- milk");
        assert_eq!(ed.text(), "milk");
        assert_eq!(lists(&ed), [(0, BULLET, 0)]);
        // Enter continues the list.
        ed.insert(&mut engine, "\n");
        type_chars(&mut ed, &mut engine, "eggs");
        assert_eq!(ed.text(), "milk\neggs");
        assert_eq!(lists(&ed), [(0, BULLET, 0), (1, BULLET, 0)]);
        // Enter on an empty item ends the list, adding no line.
        ed.insert(&mut engine, "\n");
        ed.insert(&mut engine, "\n");
        assert_eq!(ed.text(), "milk\neggs\n");
        assert_eq!(lists(&ed), [(0, BULLET, 0), (1, BULLET, 0)]);
        // A numbered list the same way; "1." elsewhere in a line stays as typed.
        type_chars(&mut ed, &mut engine, "1. one");
        assert_eq!(ed.text(), "milk\neggs\none");
        assert_eq!(lists(&ed)[2], (2, NUMBER, 0));
        type_chars(&mut ed, &mut engine, " - 1. x");
        assert_eq!(ed.text(), "milk\neggs\none - 1. x");
        // Undo takes back the typing, then the list, keeping what was typed.
        let mut fresh = rich(&mut engine, "");
        type_chars(&mut fresh, &mut engine, "* ");
        assert_eq!(lists(&fresh), [(0, BULLET, 0)]);
        fresh.undo(&mut engine);
        assert_eq!(fresh.text(), "* ");
        assert!(lists(&fresh).is_empty());
        // Plain editors (titles, labels) take "- " as it is.
        let mut plain = TextEditor::new(&mut engine, "", 20.0);
        type_chars(&mut plain, &mut engine, "- a");
        assert_eq!(plain.text(), "- a");
    }

    #[test]
    fn tab_nests_list_items_and_shift_tab_leaves_the_list() {
        let mut engine = TextEngine::new();
        let mut ed = rich(&mut engine, "");
        assert!(!ed.indent(&mut engine, false), "plain text is not indented");
        type_chars(&mut ed, &mut engine, "- a");
        assert!(ed.indent(&mut engine, false));
        assert!(ed.indent(&mut engine, false));
        assert_eq!(lists(&ed), [(0, BULLET, 2)]);
        assert!(ed.indent(&mut engine, true));
        assert_eq!(ed.para_style(), ParaStyle::list(ListKind::Bullet, 1));
        assert!(ed.indent(&mut engine, true));
        assert!(ed.indent(&mut engine, true));
        assert!(lists(&ed).is_empty());
        // Backspace at the start of an item ends the list first, then joins lines.
        let mut ed = rich(&mut engine, "x");
        ed.insert(&mut engine, "\n");
        ed.toggle_list(&mut engine, ListKind::Number);
        type_chars(&mut ed, &mut engine, "y");
        ed.motion(&mut engine, Motion::LineStart, false);
        ed.backspace(&mut engine, false);
        assert_eq!(ed.text(), "x\ny");
        assert!(lists(&ed).is_empty());
        ed.backspace(&mut engine, false);
        assert_eq!(ed.text(), "xy");
    }

    #[test]
    fn list_buttons_switch_the_selected_paragraphs() {
        let mut engine = TextEngine::new();
        let mut ed = rich(&mut engine, "a\nb\nc");
        ed.select(&mut engine, 0..3);
        ed.toggle_list(&mut engine, ListKind::Bullet);
        assert_eq!(lists(&ed), [(0, BULLET, 0), (1, BULLET, 0)]);
        // All bulleted: the same button takes them off; the other kind switches them.
        ed.toggle_list(&mut engine, ListKind::Number);
        assert_eq!(lists(&ed), [(0, NUMBER, 0), (1, NUMBER, 0)]);
        ed.toggle_list(&mut engine, ListKind::Number);
        assert!(lists(&ed).is_empty());
        ed.undo(&mut engine);
        assert_eq!(lists(&ed), [(0, NUMBER, 0), (1, NUMBER, 0)]);
        // Lists change the layout as shown, and it matches the editor's.
        let shown = engine.layout_aligned(&ed.text(), &ed.spans(), 20.0, None, TextAlign::Start);
        assert_eq!(ed.layout(&mut engine), shown);
        // The caret at the start of an item is where its text starts, past the marker.
        ed.motion(&mut engine, Motion::TextStart, false);
        let caret = ed.caret(&mut engine).unwrap();
        let first = shown.runs[0].glyphs[0].x;
        assert!(
            (caret.x0 - f64::from(first)).abs() < 2.0,
            "{caret:?} vs {first}"
        );
        // Down a line keeps to the same place in the next item.
        ed.motion(&mut engine, Motion::Down, false);
        let below = ed.caret(&mut engine).unwrap();
        assert!((below.x0 - caret.x0).abs() < 2.0 && below.y0 > caret.y0);
        ed.insert(&mut engine, "^");
        assert_eq!(ed.text(), "a\n^b\nc");
    }

    #[test]
    fn addresses_are_linked_as_typed_and_links_can_be_set() {
        let mut engine = TextEngine::new();
        let mut ed = rich(&mut engine, "");
        type_chars(&mut ed, &mut engine, "see example.com, then");
        assert_eq!(ed.spans().links.len(), 1);
        assert_eq!(ed.spans().links[0].range(), 4..15);
        // Undo takes the link back but keeps the text.
        let mut fresh = rich(&mut engine, "");
        type_chars(&mut fresh, &mut engine, "https://a.ru ");
        assert_eq!(fresh.spans().links.len(), 1);
        fresh.undo(&mut engine);
        assert_eq!(fresh.text(), "https://a.ru ");
        assert!(fresh.spans().links.is_empty());
        // A pasted address is linked at once.
        let mut pasted = rich(&mut engine, "go ");
        pasted.insert(&mut engine, "www.rust-lang.org");
        assert_eq!(pasted.spans().links[0].range(), 3..20);
        // A link set on a selection, found at the cursor, changed and taken off.
        let mut ed = rich(&mut engine, "one two");
        ed.select(&mut engine, 4..7);
        let board = Link::from_input("example.org").unwrap();
        ed.set_link(&mut engine, Some(board.clone()));
        ed.motion(&mut engine, Motion::TextEnd, false);
        assert_eq!(ed.link(), Some((4..7, board)));
        // Clicking on it finds it; beside it, nothing.
        let layout = ed.layout(&mut engine);
        let y = layout.height / 2.0;
        assert!(ed.link_at(&mut engine, layout.width - 2.0, y).is_some());
        assert!(ed.link_at(&mut engine, 2.0, y).is_none());
        ed.set_link(&mut engine, None);
        assert!(ed.spans().links.is_empty());
        // With nothing selected and no link there, the address is typed in, linked.
        let mut ed = rich(&mut engine, "");
        ed.set_link(&mut engine, Link::from_input("https://example.net/x"));
        assert_eq!(ed.text(), "example.net/x");
        assert_eq!(ed.spans().links[0].range(), 0..13);
    }

    #[test]
    fn copied_lists_and_links_paste_along() {
        let mut engine = TextEngine::new();
        let mut ed = rich(&mut engine, "");
        type_chars(&mut ed, &mut engine, "- a.com x");
        ed.insert(&mut engine, "\n");
        type_chars(&mut ed, &mut engine, "b");
        ed.select_all(&mut engine);
        let (text, spans) = ed.selected_rich().unwrap();
        let mut other = rich(&mut engine, "");
        other.paste_rich(&mut engine, &text, &spans);
        assert_eq!(other.text(), "a.com x\nb");
        assert_eq!(other.spans(), ed.spans());
    }
}
