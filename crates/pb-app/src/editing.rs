//! Typing on the canvas: one text object being edited at a time.
//!
//! Edits stay local until editing ends, then become a single command, so one undo reverts
//! the whole edit and autosave and sync see finished text only.

use pb_core::{Command, Object, ObjectData, ObjectId, ObjectKind, Rgba, Style, Text};
use pb_geom::Frame;
use pb_geom::kurbo::{Point, Rect, Vec2};
use pb_platform::clipboard;
use pb_render::TextEditOverlay;
use pb_text::{Motion, TextEditor, TextEngine, TextRect};
use winit::event::{ElementState, Ime, KeyEvent};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

/// Font size of new text, in logical pixels at the zoom it is created at.
pub const NEW_TEXT_SIZE: f64 = 24.0;
/// An empty text box is still this wide, relative to its font size, so it can be clicked.
const MIN_WIDTH: f64 = 0.5;

/// What the editor did with a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    Handled,
    /// Escape: stop editing.
    Finish,
    /// A board shortcut (save, undo, ...): stop editing, then run it.
    FinishAndPass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Existing(ObjectId),
    /// Not on the board until editing ends with some text.
    New,
}

pub struct TextEdit {
    pub target: Target,
    editor: TextEditor,
    /// World position of the text's top-left corner, before rotation.
    origin: Point,
    rotation: f64,
    style: Style,
    /// Text when editing started, to skip edits that change nothing.
    original: String,
    selecting: bool,
}

impl TextEdit {
    /// A new text whose first line is centered vertically on `at`.
    pub fn new_text(engine: &mut TextEngine, at: Point, size: f64, color: Rgba) -> Self {
        let editor = TextEditor::new(engine, "", size);
        let line = engine.layout("", size, None).height;
        Self {
            target: Target::New,
            editor,
            origin: at - Vec2::new(0.0, line / 2.0),
            rotation: 0.0,
            style: Style {
                fill: Rgba(0),
                stroke: color,
                stroke_width: 0.0,
                opacity: 1.0,
            },
            original: String::new(),
            selecting: false,
        }
    }

    /// Edits an existing text object; `None` for other objects.
    pub fn existing(engine: &mut TextEngine, object: &Object) -> Option<Self> {
        let ObjectKind::Text(text) = &object.data.kind else {
            return None;
        };
        Some(Self {
            target: Target::Existing(object.id),
            editor: TextEditor::new(engine, &text.text, text.size),
            origin: object.data.bounds.origin(),
            rotation: object.data.rotation,
            style: object.data.style,
            original: text.text.to_string(),
            selecting: false,
        })
    }

    /// The text box as it is now, in world space.
    pub fn frame(&mut self, engine: &mut TextEngine) -> Frame {
        let layout = self.editor.layout(engine);
        let width = layout.width.max(self.editor.size() * MIN_WIDTH);
        let size = (width, layout.height);
        Frame::new(Rect::from_origin_size(self.origin, size), self.rotation)
    }

    /// World point to layout coordinates.
    fn layout_point(&mut self, engine: &mut TextEngine, world: Point) -> Point {
        let local = self.frame(engine).to_local(world);
        local - self.origin.to_vec2()
    }

    fn world_point(&mut self, engine: &mut TextEngine, p: Point) -> Point {
        self.frame(engine).to_world(self.origin + p.to_vec2())
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
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => return KeyOutcome::Finish,
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
    }

    /// The object as it looks now, including IME composition.
    pub fn data(&mut self, engine: &mut TextEngine) -> ObjectData {
        let frame = self.frame(engine);
        ObjectData {
            kind: ObjectKind::Text(Text {
                text: self.editor.display_text().into(),
                size: self.editor.size(),
            }),
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

    /// The command that keeps the edit, if anything changed. Text left empty removes the
    /// object.
    pub fn finish(mut self, engine: &mut TextEngine) -> Option<Command> {
        self.editor.clear_preedit(engine);
        let text = self.editor.text();
        let empty = text.trim().is_empty();
        match self.target {
            Target::Existing(id) if empty => Some(Command::Delete(vec![id])),
            Target::Existing(_) if text == self.original => None,
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

    fn text_object(text: &str) -> Object {
        let mut doc = pb_core::Document::new();
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
        doc.get(id).unwrap().clone()
    }

    #[test]
    fn editing_existing_text() {
        let mut e = engine();
        let object = text_object("abc");
        let unchanged = TextEdit::existing(&mut e, &object).unwrap();
        assert_eq!(unchanged.finish(&mut e), None);

        let mut edit = TextEdit::existing(&mut e, &object).unwrap();
        edit.editor.insert(&mut e, "d");
        match edit.finish(&mut e) {
            Some(Command::SetText { id, text, bounds }) => {
                assert_eq!(id, object.id);
                assert_eq!(&*text, "abcd");
                assert_eq!(bounds.origin(), Point::new(10.0, 10.0));
            }
            other => panic!("expected SetText, got {other:?}"),
        }

        let mut cleared = TextEdit::existing(&mut e, &object).unwrap();
        cleared.select_all(&mut e);
        cleared.editor.backspace(&mut e, false);
        assert_eq!(
            cleared.finish(&mut e),
            Some(Command::Delete(vec![object.id]))
        );
    }

    #[test]
    fn clicks_map_through_the_box() {
        let mut e = engine();
        let object = text_object("hello world");
        let mut edit = TextEdit::existing(&mut e, &object).unwrap();
        let frame = edit.frame(&mut e);
        assert!(edit.contains(&mut e, frame.center(), 0.0));
        assert!(!edit.contains(&mut e, Point::new(0.0, 0.0), 0.0));
        // Double click on the first word selects it.
        edit.press(&mut e, Point::new(15.0, 20.0), false, true);
        let overlay = edit.overlay(&mut e);
        assert_eq!(overlay.selection.len(), 1);
        assert!(overlay.caret.is_none());
    }
}
