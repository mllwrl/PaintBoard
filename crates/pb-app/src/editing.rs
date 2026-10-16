//! Typing on the canvas: one text object, sticky note or connector label being edited at a
//! time.
//!
//! Edits stay local until editing ends, then become a single command, so one undo reverts
//! the whole edit and autosave and sync see finished text only.

use pb_core::{
    Command, Document, Label, ObjectData, ObjectId, ObjectKind, Rgba, Sticky, Style, Text,
};
use pb_geom::Frame;
use pb_geom::kurbo::{Point, Rect, Size, Vec2};
use pb_platform::clipboard;
use pb_render::{TextEditOverlay, sticky_font_size};
use pb_text::{Motion, TextAlign, TextEditor, TextEngine, TextRect};
use winit::event::{ElementState, Ime, KeyEvent};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

/// Font size of new text, in logical pixels at the zoom it is created at.
pub const NEW_TEXT_SIZE: f64 = 24.0;
/// An empty text box is still this wide, relative to its font size, so it can be clicked.
const MIN_WIDTH: f64 = 0.5;
/// Space between a sticky note and the next one added with Tab, relative to its size.
const STICKY_GAP: f64 = 0.1;

/// What the editor did with a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    Handled,
    /// Escape: stop editing.
    Finish,
    /// A board shortcut (save, undo, ...): stop editing, then run it.
    FinishAndPass,
    /// Tab on a sticky note: stop editing and start a new note to its right, or below it
    /// with Shift.
    NextSticky {
        below: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Existing(ObjectId),
    /// Not on the board until editing ends with some text.
    New,
}

/// Where the text sits.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Placement {
    /// A text object: the box grows with the text from its top-left corner (world, before
    /// rotation).
    Free { origin: Point },
    /// A sticky note: the text is fitted and centered in the note's fixed bounds.
    Sticky { bounds: Rect },
    /// A connector's label: centered on `center`, `at` along the line.
    Label { center: Point, at: f64 },
}

pub struct TextEdit {
    pub target: Target,
    editor: TextEditor,
    placement: Placement,
    rotation: f64,
    style: Style,
    /// Text when editing started, to skip edits that change nothing.
    original: String,
    selecting: bool,
    /// The connector whose label this is.
    connector: Option<ObjectData>,
}

impl TextEdit {
    /// A new text whose first line is centered vertically on `at`.
    pub fn new_text(engine: &mut TextEngine, at: Point, size: f64, color: Rgba) -> Self {
        let editor = TextEditor::new(engine, "", size);
        let line = engine.layout("", size, None).height;
        Self {
            target: Target::New,
            editor,
            placement: Placement::Free {
                origin: at - Vec2::new(0.0, line / 2.0),
            },
            rotation: 0.0,
            style: Style {
                fill: Rgba(0),
                stroke: color,
                stroke_width: 0.0,
                opacity: 1.0,
            },
            original: String::new(),
            selecting: false,
            connector: None,
        }
    }

    /// A new sticky note with `bounds` (world) and `style`, empty until typed into.
    pub fn new_sticky(engine: &mut TextEngine, bounds: Rect, style: Style) -> Self {
        Self::sticky(engine, Target::New, "", bounds, 0.0, style)
    }

    fn sticky(
        engine: &mut TextEngine,
        target: Target,
        text: &str,
        bounds: Rect,
        rotation: f64,
        style: Style,
    ) -> Self {
        let mut editor = TextEditor::new(engine, text, Sticky::max_font_size(bounds));
        editor.set_wrap(Some(Sticky::text_area(bounds).width()), TextAlign::Center);
        let mut this = Self {
            target,
            editor,
            placement: Placement::Sticky { bounds },
            rotation,
            style,
            original: text.to_owned(),
            selecting: false,
            connector: None,
        };
        this.refit(engine);
        this
    }

    /// Edits an existing text object, sticky note or connector label; `None` for other
    /// objects. A connector without a label gets one where its line is nearest to `near`
    /// (a double click), or in its middle.
    pub fn existing(
        engine: &mut TextEngine,
        doc: &Document,
        id: ObjectId,
        near: Option<Point>,
    ) -> Option<Self> {
        let data = &doc.get(id)?.data;
        let target = Target::Existing(id);
        match &data.kind {
            ObjectKind::Text(text) => Some(Self {
                target,
                editor: TextEditor::new(engine, &text.text, text.size),
                placement: Placement::Free {
                    origin: data.bounds.origin(),
                },
                rotation: data.rotation,
                style: data.style,
                original: text.text.to_string(),
                selecting: false,
                connector: None,
            }),
            ObjectKind::Sticky(sticky) => Some(Self::sticky(
                engine,
                target,
                &sticky.text,
                data.bounds,
                data.rotation,
                data.style,
            )),
            ObjectKind::Connector(c) => {
                let path = doc.connector_path(id)?;
                let at = match near {
                    Some(p) if c.label.is_empty() => path.nearest_fraction(p),
                    _ => c.label.at,
                };
                let mut editor = TextEditor::new(engine, &c.label.text, Label::FONT_SIZE);
                editor.set_wrap(None, TextAlign::Center);
                Some(Self {
                    target,
                    editor,
                    placement: Placement::Label {
                        center: path.point_at(at),
                        at,
                    },
                    rotation: 0.0,
                    style: data.style,
                    original: c.label.text.to_string(),
                    selecting: false,
                    connector: Some(data.clone()),
                })
            }
            ObjectKind::Shape(_) | ObjectKind::Ink(_) | ObjectKind::Image(_) => None,
        }
    }

    /// Sizes a sticky note's text to fit, after the text changed.
    fn refit(&mut self, engine: &mut TextEngine) {
        if let Placement::Sticky { bounds } = self.placement {
            let size = sticky_font_size(engine, &self.editor.display_text(), bounds);
            self.editor.set_size(size);
        }
    }

    /// A new, empty sticky note next to this one, with the same size and style; `None` when
    /// this is not a sticky note.
    pub fn next_sticky(&mut self, engine: &mut TextEngine, below: bool) -> Option<Self> {
        let Placement::Sticky { bounds } = self.placement else {
            return None;
        };
        let step = if below {
            Vec2::new(0.0, bounds.height() * (1.0 + STICKY_GAP))
        } else {
            Vec2::new(bounds.width() * (1.0 + STICKY_GAP), 0.0)
        };
        Some(Self::new_sticky(engine, bounds + step, self.style))
    }

    /// The text box as it is now, in world space.
    pub fn frame(&mut self, engine: &mut TextEngine) -> Frame {
        let rect = match self.placement {
            Placement::Free { origin } => {
                let layout = self.editor.layout(engine);
                let width = layout.width.max(self.editor.size() * MIN_WIDTH);
                Rect::from_origin_size(origin, (width, layout.height))
            }
            Placement::Sticky { bounds } => bounds,
            Placement::Label { center, .. } => {
                let layout = self.editor.layout(engine);
                let width = layout.width.max(self.editor.size() * MIN_WIDTH);
                Rect::from_center_size(center, (width, layout.height))
            }
        };
        Frame::new(rect, self.rotation)
    }

    /// Top-left corner of the text layout, in the box's unrotated coordinates.
    fn text_origin(&mut self, engine: &mut TextEngine) -> Point {
        match self.placement {
            Placement::Free { origin } => origin,
            Placement::Sticky { bounds } => {
                let area = Sticky::text_area(bounds);
                let height = self.editor.layout(engine).height;
                Point::new(area.x0, area.y0 + (area.height() - height) / 2.0)
            }
            // Drawn the same way, see `pb_render`.
            Placement::Label { center, .. } => {
                let layout = self.editor.layout(engine);
                center - Vec2::new(layout.width, layout.height) / 2.0
            }
        }
    }

    /// World point to layout coordinates.
    fn layout_point(&mut self, engine: &mut TextEngine, world: Point) -> Point {
        let local = self.frame(engine).to_local(world);
        local - self.text_origin(engine).to_vec2()
    }

    fn world_point(&mut self, engine: &mut TextEngine, p: Point) -> Point {
        let origin = self.text_origin(engine);
        self.frame(engine).to_world(origin + p.to_vec2())
    }

    pub fn contains(&mut self, engine: &mut TextEngine, world: Point, pad: f64) -> bool {
        self.frame(engine).contains(world, pad)
    }

    /// A press inside the box: places the cursor, or selects a word on a double click.
    pub fn press(&mut self, engine: &mut TextEngine, world: Point, shift: bool, double: bool) {
        let p = self.layout_point(engine, world);
        if double {
            self.editor.select_word_at(engine, p.x, p.y);
        } else {
            self.editor.click(engine, p.x, p.y, shift);
            self.selecting = true;
        }
    }

    /// Returns true if the selection changed.
    pub fn drag(&mut self, engine: &mut TextEngine, world: Point) -> bool {
        if !self.selecting {
            return false;
        }
        let p = self.layout_point(engine, world);
        self.editor.drag_to(engine, p.x, p.y);
        true
    }

    pub fn release(&mut self) {
        self.selecting = false;
    }

    pub fn select_all(&mut self, engine: &mut TextEngine) {
        self.editor.select_all(engine);
    }

    pub fn key(
        &mut self,
        engine: &mut TextEngine,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> KeyOutcome {
        if event.state != ElementState::Pressed {
            return KeyOutcome::Handled;
        }
        let ctrl = modifiers.control_key();
        let shift = modifiers.shift_key();
        let ed = &mut self.editor;
        // Board shortcuts by physical key, like elsewhere.
        if ctrl && let PhysicalKey::Code(code) = event.physical_key {
            match code {
                KeyCode::KeyA => ed.select_all(engine),
                KeyCode::KeyC => {
                    if let Some(text) = ed.selected_text() {
                        clipboard::set_text(&text);
                    }
                }
                KeyCode::KeyX => {
                    if let Some(text) = ed.selected_text() {
                        clipboard::set_text(&text);
                        ed.backspace(engine, false);
                    }
                }
                KeyCode::KeyV => {
                    if let Some(text) = clipboard::text() {
                        ed.insert(engine, &text.replace("\r\n", "\n"));
                    }
                }
                KeyCode::KeyS | KeyCode::KeyO | KeyCode::KeyN | KeyCode::KeyZ | KeyCode::KeyY => {
                    return KeyOutcome::FinishAndPass;
                }
                _ => {}
            }
        }
        let word = ctrl;
        let sticky = matches!(self.placement, Placement::Sticky { .. });
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => return KeyOutcome::Finish,
            Key::Named(NamedKey::Tab) if sticky && !ctrl && !ed.is_composing() => {
                return KeyOutcome::NextSticky { below: shift };
            }
            Key::Named(NamedKey::Backspace) => ed.backspace(engine, word),
            Key::Named(NamedKey::Delete) => ed.delete(engine, word),
            Key::Named(NamedKey::Enter) => ed.insert(engine, "\n"),
            Key::Named(NamedKey::ArrowLeft) => {
                let motion = if word { Motion::WordLeft } else { Motion::Left };
                ed.motion(engine, motion, shift);
            }
            Key::Named(NamedKey::ArrowRight) => {
                let motion = if word {
                    Motion::WordRight
                } else {
                    Motion::Right
                };
                ed.motion(engine, motion, shift);
            }
            Key::Named(NamedKey::ArrowUp) => ed.motion(engine, Motion::Up, shift),
            Key::Named(NamedKey::ArrowDown) => ed.motion(engine, Motion::Down, shift),
            Key::Named(NamedKey::Home) => {
                let motion = if ctrl {
                    Motion::TextStart
                } else {
                    Motion::LineStart
                };
                ed.motion(engine, motion, shift);
            }
            Key::Named(NamedKey::End) => {
                let motion = if ctrl {
                    Motion::TextEnd
                } else {
                    Motion::LineEnd
                };
                ed.motion(engine, motion, shift);
            }
            _ if ctrl || ed.is_composing() => {}
            _ => {
                // Characters typed directly; IME input arrives as `Ime::Commit` instead.
                if let Some(text) = &event.text {
                    let text: String = text.chars().filter(|c| !c.is_control()).collect();
                    if !text.is_empty() {
                        ed.insert(engine, &text);
                    }
                }
            }
        }
        self.refit(engine);
        KeyOutcome::Handled
    }

    pub fn ime(&mut self, engine: &mut TextEngine, ime: &Ime) {
        match ime {
            Ime::Preedit(text, cursor) => self.editor.set_preedit(engine, text, *cursor),
            Ime::Commit(text) => {
                self.editor.clear_preedit(engine);
                self.editor.insert(engine, text);
            }
            Ime::Disabled => self.editor.clear_preedit(engine),
            Ime::Enabled => {}
        }
        self.refit(engine);
    }

    /// A connector's label with the text being edited.
    fn label(&mut self, engine: &mut TextEngine, text: String) -> Option<Label> {
        let Placement::Label { at, .. } = self.placement else {
            return None;
        };
        let layout = self.editor.layout(engine);
        Some(Label {
            text: text.into(),
            at,
            size: Size::new(layout.width, layout.height),
        })
    }

    /// The object as it looks now, including IME composition.
    pub fn data(&mut self, engine: &mut TextEngine) -> ObjectData {
        let display = self.editor.display_text();
        if let Some(label) = self.label(engine, display.clone())
            && let Some(mut data) = self.connector.clone()
        {
            if let ObjectKind::Connector(c) = &mut data.kind {
                c.label = label;
            }
            return data;
        }
        let frame = self.frame(engine);
        let text = display.into();
        ObjectData {
            kind: match self.placement {
                Placement::Free { .. } => ObjectKind::Text(Text {
                    text,
                    size: self.editor.size(),
                }),
                Placement::Sticky { .. } | Placement::Label { .. } => {
                    ObjectKind::Sticky(Sticky { text })
                }
            },
            bounds: frame.rect,
            rotation: frame.rotation,
            style: self.style,
        }
    }

    /// Box, selection and caret for the overlay, in world space.
    pub fn overlay(&mut self, engine: &mut TextEngine) -> TextEditOverlay {
        let rects = self.editor.selection(engine);
        let selection: Vec<_> = rects.iter().map(|r| self.quad(engine, *r)).collect();
        // Like most editors, no caret while text is selected.
        let caret = self.editor.caret(engine).filter(|_| selection.is_empty());
        let caret = caret.map(|r| {
            let x = (r.x0 + r.x1) / 2.0;
            let top = self.world_point(engine, Point::new(x, r.y0));
            let bottom = self.world_point(engine, Point::new(x, r.y1));
            (top, bottom)
        });
        TextEditOverlay {
            frame: self.frame(engine),
            selection,
            caret,
            caret_width: (self.editor.size() * 0.08).max(0.5),
        }
    }

    fn quad(&mut self, engine: &mut TextEngine, r: TextRect) -> [Point; 4] {
        [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)]
            .map(|(x, y)| self.world_point(engine, Point::new(x, y)))
    }

    /// Where the IME candidate window goes, in world space (axis-aligned).
    pub fn ime_area(&mut self, engine: &mut TextEngine) -> Rect {
        let r = self.editor.ime_area(engine);
        let a = self.world_point(engine, Point::new(r.x0, r.y0));
        let b = self.world_point(engine, Point::new(r.x1, r.y1));
        Rect::from_points(a, b)
    }

    /// The command that keeps the edit, if anything changed. A text object left empty is
    /// removed; a sticky note stays, even a new one; a label left blank goes.
    pub fn finish(mut self, engine: &mut TextEngine) -> Option<Command> {
        self.editor.clear_preedit(engine);
        self.refit(engine);
        let text = self.editor.text();
        if let (Target::Existing(id), Placement::Label { .. }) = (self.target, self.placement) {
            let text = if text.trim().is_empty() {
                String::new()
            } else {
                text
            };
            if text == self.original {
                return None;
            }
            let label = self.label(engine, text)?;
            return Some(Command::SetLabel { id, label });
        }
        let empty = text.trim().is_empty() && matches!(self.placement, Placement::Free { .. });
        match self.target {
            Target::Existing(_) if text == self.original => None,
            Target::Existing(id) if empty => Some(Command::Delete(vec![id])),
            Target::Existing(id) => Some(Command::SetText {
                id,
                text: text.into(),
                bounds: self.frame(engine).rect,
            }),
            Target::New if empty => None,
            Target::New => Some(Command::Create(vec![self.data(engine).into()])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    #[test]
    fn new_text_is_created_only_with_content() {
        let mut e = engine();
        let edit = TextEdit::new_text(&mut e, Point::new(100.0, 100.0), 20.0, Rgba(0xFF));
        assert_eq!(edit.finish(&mut e), None);

        let mut edit = TextEdit::new_text(&mut e, Point::new(100.0, 100.0), 20.0, Rgba(0xFF));
        edit.editor.insert(&mut e, "Hi");
        let frame = edit.frame(&mut e);
        // The first line is centered on the click.
        assert!((frame.rect.center().y - 100.0).abs() < 1e-9);
        assert_eq!(frame.rect.x0, 100.0);
        match edit.finish(&mut e) {
            Some(Command::Create(items)) => assert_eq!(items.len(), 1),
            other => panic!("expected Create, got {other:?}"),
        }
    }

    fn text_object(text: &str) -> (Document, ObjectId) {
        let mut doc = Document::new();
        let data = ObjectData {
            kind: ObjectKind::Text(Text {
                text: text.into(),
                size: 20.0,
            }),
            bounds: Rect::new(10.0, 10.0, 60.0, 34.0),
            rotation: 0.0,
            style: Style::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        (doc, id)
    }

    #[test]
    fn editing_existing_text() {
        let mut e = engine();
        let (doc, object) = text_object("abc");
        let unchanged = TextEdit::existing(&mut e, &doc, object, None).unwrap();
        assert_eq!(unchanged.finish(&mut e), None);

        let mut edit = TextEdit::existing(&mut e, &doc, object, None).unwrap();
        edit.editor.insert(&mut e, "d");
        match edit.finish(&mut e) {
            Some(Command::SetText { id, text, bounds }) => {
                assert_eq!(id, object);
                assert_eq!(&*text, "abcd");
                assert_eq!(bounds.origin(), Point::new(10.0, 10.0));
            }
            other => panic!("expected SetText, got {other:?}"),
        }

        let mut cleared = TextEdit::existing(&mut e, &doc, object, None).unwrap();
        cleared.select_all(&mut e);
        cleared.editor.backspace(&mut e, false);
        assert_eq!(cleared.finish(&mut e), Some(Command::Delete(vec![object])));
    }

    fn sticky_style() -> Style {
        Style {
            fill: Rgba::rgb(0xFF, 0xF1, 0x76),
            stroke_width: 0.0,
            ..Style::default()
        }
    }

    #[test]
    fn new_sticky_is_kept_empty_and_fits_its_text() {
        let mut e = engine();
        let bounds = Rect::new(0.0, 0.0, 200.0, 200.0);
        let edit = TextEdit::new_sticky(&mut e, bounds, sticky_style());
        match edit.finish(&mut e) {
            Some(Command::Create(items)) => match &items[..] {
                [pb_core::Content::Object(data)] => {
                    assert_eq!(data.kind.text(), Some(""));
                    assert_eq!(data.bounds, bounds);
                }
                other => panic!("unexpected content {other:?}"),
            },
            other => panic!("expected Create, got {other:?}"),
        }

        let mut edit = TextEdit::new_sticky(&mut e, bounds, sticky_style());
        let full = edit.editor.size();
        assert_eq!(full, Sticky::max_font_size(bounds));
        edit.editor
            .insert(&mut e, &"many words on a small note ".repeat(8));
        edit.refit(&mut e);
        assert!(edit.editor.size() < full);
        // The box stays the note, whatever the text.
        assert_eq!(edit.frame(&mut e).rect, bounds);
    }

    #[test]
    fn emptied_sticky_is_kept_and_next_goes_beside() {
        let mut e = engine();
        let mut doc = Document::new();
        let data = ObjectData {
            kind: ObjectKind::Sticky(Sticky {
                text: "idea".into(),
            }),
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            rotation: 0.0,
            style: sticky_style(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        let mut edit = TextEdit::existing(&mut e, &doc, id, None).unwrap();
        edit.select_all(&mut e);
        edit.editor.backspace(&mut e, false);
        let mut right = edit.next_sticky(&mut e, false).unwrap();
        let mut below = edit.next_sticky(&mut e, true).unwrap();
        let near = |a: Rect, b: Rect| {
            let d = [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1];
            d.iter().all(|d| d.abs() < 1e-9)
        };
        assert!(near(
            right.frame(&mut e).rect,
            Rect::new(110.0, 0.0, 210.0, 100.0)
        ));
        assert!(near(
            below.frame(&mut e).rect,
            Rect::new(0.0, 110.0, 100.0, 210.0)
        ));
        match edit.finish(&mut e) {
            Some(Command::SetText {
                id: target, text, ..
            }) => {
                assert_eq!(target, id);
                assert_eq!(&*text, "");
            }
            other => panic!("expected SetText, got {other:?}"),
        }
    }

    #[test]
    fn clicks_map_through_the_box() {
        let mut e = engine();
        let (doc, object) = text_object("hello world");
        let mut edit = TextEdit::existing(&mut e, &doc, object, None).unwrap();
        let frame = edit.frame(&mut e);
        assert!(edit.contains(&mut e, frame.center(), 0.0));
        assert!(!edit.contains(&mut e, Point::new(0.0, 0.0), 0.0));
        // Double click on the first word selects it.
        edit.press(&mut e, Point::new(15.0, 20.0), false, true);
        let overlay = edit.overlay(&mut e);
        assert_eq!(overlay.selection.len(), 1);
        assert!(overlay.caret.is_none());
    }

    #[test]
    fn labels_start_at_the_click_and_blank_ones_go() {
        let mut e = engine();
        let mut doc = Document::new();
        let line = ObjectData {
            kind: ObjectKind::Connector(pb_core::Connector {
                start: pb_core::End::Free(Point::new(0.0, 0.0)),
                end: pb_core::End::Free(Point::new(400.0, 0.0)),
                route: pb_core::Route::Straight,
                start_cap: pb_core::Cap::None,
                end_cap: pb_core::Cap::None,
                label: Label::default(),
            }),
            bounds: Rect::ZERO,
            rotation: 0.0,
            style: Style::default(),
        };
        let id = doc
            .apply(Command::Create(vec![line.into()]))
            .unwrap()
            .created[0];
        let near = Some(Point::new(100.0, 10.0));
        let untouched = TextEdit::existing(&mut e, &doc, id, near).unwrap();
        assert_eq!(untouched.finish(&mut e), None);

        let mut edit = TextEdit::existing(&mut e, &doc, id, near).unwrap();
        edit.editor.insert(&mut e, "yes");
        // The box is centered on the line where it was clicked.
        let center = edit.frame(&mut e).rect.center();
        assert!(
            (center - Point::new(100.0, 0.0)).hypot() < 1e-6,
            "{center:?}"
        );
        let Some(Command::SetLabel { id: target, label }) = edit.finish(&mut e) else {
            panic!("expected SetLabel");
        };
        assert_eq!(target, id);
        assert_eq!(&*label.text, "yes");
        assert!((label.at - 0.25).abs() < 1e-6);
        assert!(label.size.width > 0.0 && label.size.height > 0.0);
        doc.apply(Command::SetLabel { id, label }).unwrap();

        // Editing again keeps its place; clearing it removes the text.
        let mut edit = TextEdit::existing(&mut e, &doc, id, Some(Point::ZERO)).unwrap();
        edit.select_all(&mut e);
        edit.editor.insert(&mut e, "  ");
        let Some(Command::SetLabel { label, .. }) = edit.finish(&mut e) else {
            panic!("expected SetLabel");
        };
        assert!(label.is_empty());
        assert!((label.at - 0.25).abs() < 1e-6);
    }
}
