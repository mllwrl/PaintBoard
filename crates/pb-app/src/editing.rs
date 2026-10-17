//! Typing on the canvas: one text object, sticky note, shape or connector label being
//! edited at a time.
//!
//! Edits stay local until editing ends, then become a single command, so one undo reverts
//! the whole edit and autosave and sync see finished text only.

use pb_core::{
    Card, Cell, Command, Document, FrameData, Kanban, KanbanPart, Label, ObjectData, ObjectId,
    ObjectKind, Rgba, Shape, Span, Sticky, Style, Table, Text, TextBox, TextMarks,
};
use pb_geom::kurbo::{Point, Rect, Size, Vec2};
use pb_geom::{Frame, ShapeKind};
use pb_platform::{Duration, Instant, clipboard};
use pb_render::{TextEditOverlay, fitted_font_size, text_align};
use pb_text::{Mark, Motion, TextAlign, TextEditor, TextEngine, TextRect};
use winit::event::{ElementState, Ime, KeyEvent};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

/// Font size of new text, in logical pixels at the zoom it is created at.
pub const NEW_TEXT_SIZE: f64 = 24.0;
/// An empty text box is still this wide, relative to its font size, so it can be clicked.
const MIN_WIDTH: f64 = 0.5;
/// Text objects wrap a hair wider than their bounds, so text measured to fit exactly does
/// not wrap; the same as in `pb_render`.
const WRAP_SLACK: f64 = 0.5;
/// Space between a sticky note and the next one added with Tab, relative to its size.
const STICKY_GAP: f64 = 0.1;
/// How long the caret is shown, then hidden, while it blinks (the Windows default).
const BLINK: Duration = Duration::from_millis(530);
/// The caret stops blinking and stays shown this long after the last input, so an idle
/// edit does not keep waking the app.
const BLINK_FOR: Duration = Duration::from_secs(20);

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
    /// Tab in a table cell: stop editing and go on in the next cell, or the previous one
    /// with Shift.
    NextCell {
        back: bool,
    },
    /// Tab on a kanban card: stop editing and start a new card below it.
    NextCard,
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
    /// rotation). With a `width`, lines wrap there and only the height grows.
    Free { origin: Point, width: Option<f64> },
    /// A sticky note, or a shape of the given kind: the text is fitted and centered in the
    /// object's fixed bounds.
    Fitted {
        bounds: Rect,
        shape: Option<ShapeKind>,
    },
    /// A connector's label: centered on `center`, `at` along the line.
    Label { center: Point, at: f64 },
    /// A frame's title, above the frame's `bounds`, sized for `zoom`.
    Title { bounds: Rect, zoom: f64 },
    /// Cell (`row`, `col`) of a table with `bounds`: the text is fitted into `text_box`
    /// inside the cell's rectangle `rect`.
    Cell {
        bounds: Rect,
        rect: Rect,
        row: usize,
        col: usize,
        text_box: TextBox,
    },
    /// A card or column title of a kanban board with `bounds`: the text is fitted into
    /// `text_box` inside `rect`.
    Board {
        bounds: Rect,
        rect: Rect,
        part: BoardPart,
        text_box: TextBox,
    },
}

/// What of a kanban board is edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoardPart {
    /// Card `i` of the board as edited (see `TextEdit::kanban`).
    Card(usize),
    /// The title of column `i`.
    Title(usize),
}

impl Placement {
    /// Where fitted text goes, for an object with `style`; `None` unless it is fitted.
    fn text_box(self, style: &Style) -> Option<TextBox> {
        match self {
            Self::Fitted {
                bounds,
                shape: Some(kind),
            } => Some(Shape::text_box(kind, bounds, style)),
            Self::Fitted {
                bounds,
                shape: None,
            } => Some(Sticky::text_box(bounds)),
            Self::Cell { text_box, .. } | Self::Board { text_box, .. } => Some(text_box),
            Self::Free { .. } | Self::Label { .. } | Self::Title { .. } => None,
        }
    }
}

pub struct TextEdit {
    pub target: Target,
    editor: TextEditor,
    placement: Placement,
    rotation: f64,
    style: Style,
    /// Text when editing started, to skip edits that change nothing.
    original: String,
    original_spans: Vec<Span>,
    selecting: bool,
    /// Last input; the caret blinks from then on, starting shown.
    active_at: Instant,
    /// Whether the last overlay showed the caret.
    caret_drawn: bool,
    /// The connector whose label this is.
    connector: Option<ObjectData>,
    /// The table whose cell this is.
    table: Option<Table>,
    /// The kanban board whose card or title this is, with the card if it is new.
    kanban: Option<Kanban>,
    /// The card is not on the board yet: it is added only if it gets some text.
    new_card: bool,
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
                width: None,
            },
            rotation: 0.0,
            style: Style {
                fill: Rgba(0),
                stroke: color,
                stroke_width: 0.0,
                opacity: 1.0,
                ..Style::default()
            },
            original: String::new(),
            original_spans: Vec::new(),
            selecting: false,
            active_at: Instant::now(),
            caret_drawn: false,
            connector: None,
            table: None,
            kanban: None,
            new_card: false,
        }
    }

    /// A new sticky note with `bounds` (world) and `style`, empty until typed into.
    pub fn new_sticky(engine: &mut TextEngine, bounds: Rect, style: Style) -> Self {
        let placement = Placement::Fitted {
            bounds,
            shape: None,
        };
        Self::fitted(engine, Target::New, ("", &[]), placement, 0.0, style)
    }

    /// Text fitted into a sticky note or shape; `placement` is [`Placement::Fitted`].
    fn fitted(
        engine: &mut TextEngine,
        target: Target,
        (text, spans): (&str, &[Span]),
        placement: Placement,
        rotation: f64,
        style: Style,
    ) -> Self {
        let text_box = placement.text_box(&style).expect("fitted placement");
        let mut editor = TextEditor::with_spans(engine, text, spans, text_box.max_size);
        editor.set_wrap(Some(text_box.area.width()), text_align(text_box.align));
        let mut this = Self {
            target,
            editor,
            placement,
            rotation,
            style,
            original: text.to_owned(),
            original_spans: spans.to_vec(),
            selecting: false,
            active_at: Instant::now(),
            caret_drawn: false,
            connector: None,
            table: None,
            kanban: None,
            new_card: false,
        };
        this.refit(engine);
        this
    }

    /// The width a text object with `bounds` wraps at: its own when it was made narrower
    /// than its longest line, otherwise none, so it grows as it is typed into.
    fn wrap_width(engine: &mut TextEngine, text: &Text, bounds: Rect) -> Option<f64> {
        let natural = engine.layout(&text.text, text.size, None).content_width;
        (natural > bounds.width() + WRAP_SLACK).then_some(bounds.width())
    }

    fn text_editor(engine: &mut TextEngine, text: &Text, bounds: Rect) -> TextEditor {
        let mut editor = TextEditor::with_spans(engine, &text.text, &text.spans, text.size);
        if let Some(width) = Self::wrap_width(engine, text, bounds) {
            // Wrapped as drawn, see `pb_render`.
            editor.set_wrap(Some(width + WRAP_SLACK), TextAlign::Start);
        }
        editor
    }

    /// Edits an existing text object, sticky note, shape, connector label or frame title;
    /// `None` for other objects. A connector without a label gets one where its line is
    /// nearest to `near` (a double click), or in its middle. A title is sized for `zoom`.
    pub fn existing(
        engine: &mut TextEngine,
        doc: &Document,
        id: ObjectId,
        near: Option<Point>,
        zoom: f64,
    ) -> Option<Self> {
        let data = &doc.get(id)?.data;
        let target = Target::Existing(id);
        match &data.kind {
            ObjectKind::Text(text) => Some(Self {
                target,
                editor: Self::text_editor(engine, text, data.bounds),
                placement: Placement::Free {
                    origin: data.bounds.origin(),
                    width: Self::wrap_width(engine, text, data.bounds),
                },
                rotation: data.rotation,
                style: data.style,
                original: text.text.to_string(),
                original_spans: text.spans.to_vec(),
                selecting: false,
                active_at: Instant::now(),
                caret_drawn: false,
                connector: None,
                table: None,
                kanban: None,
                new_card: false,
            }),
            ObjectKind::Sticky(sticky) => Some(Self::fitted(
                engine,
                target,
                (&sticky.text, &sticky.spans),
                Placement::Fitted {
                    bounds: data.bounds,
                    shape: None,
                },
                data.rotation,
                data.style,
            )),
            ObjectKind::Shape(shape) => Some(Self::fitted(
                engine,
                target,
                (&shape.text, &shape.spans),
                Placement::Fitted {
                    bounds: data.bounds,
                    shape: Some(shape.kind),
                },
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
                    original_spans: Vec::new(),
                    selecting: false,
                    active_at: Instant::now(),
                    caret_drawn: false,
                    connector: Some(data.clone()),
                    table: None,
                    kanban: None,
                    new_card: false,
                })
            }
            ObjectKind::Frame(frame) => Some(Self {
                target,
                editor: TextEditor::new(engine, &frame.title, FrameData::TITLE_SIZE / zoom),
                placement: Placement::Title {
                    bounds: data.bounds,
                    zoom,
                },
                rotation: 0.0,
                style: data.style,
                original: frame.title.to_string(),
                original_spans: Vec::new(),
                selecting: false,
                active_at: Instant::now(),
                caret_drawn: false,
                connector: None,
                table: None,
                kanban: None,
                new_card: false,
            }),
            ObjectKind::Table(table) => {
                let local = near.map(|p| data.frame().to_local(p));
                let (row, col) = local
                    .and_then(|p| table.cell_at(data.bounds, p))
                    .unwrap_or((0, 0));
                Self::cell(engine, target, data, table, row, col)
            }
            ObjectKind::Kanban(kanban) => {
                let local = data.frame().to_local(near?);
                let part = match kanban.part_at(data.bounds, local)? {
                    KanbanPart::Card(i) => BoardPart::Card(i),
                    KanbanPart::Title(col) => BoardPart::Title(col),
                    KanbanPart::Add(_) => return None,
                };
                Self::board(engine, target, data, kanban.clone(), part, false)
            }
            ObjectKind::Ink(_) | ObjectKind::Image(_) => None,
        }
    }

    /// Edits a card or column title of the kanban board object `data`, laid out as in
    /// `kanban` (which has the card when it is `new_card`).
    fn board(
        engine: &mut TextEngine,
        target: Target,
        data: &ObjectData,
        kanban: Kanban,
        part: BoardPart,
        new_card: bool,
    ) -> Option<Self> {
        let bounds = if new_card {
            kanban.fitted(data.bounds)
        } else {
            data.bounds
        };
        let (rect, text_box, text, spans) = match part {
            BoardPart::Card(i) => {
                let card = kanban.cards.get(i)?;
                (
                    kanban.card_rect(bounds, i)?,
                    kanban.card_text_box(bounds, i)?,
                    card.text.clone(),
                    card.spans.clone(),
                )
            }
            BoardPart::Title(col) => {
                let title = kanban.columns.get(col)?.title.clone();
                let spans = Kanban::title_spans(&title);
                (
                    kanban.title_rect(bounds, col)?,
                    kanban.title_text_box(bounds, col)?,
                    title,
                    spans,
                )
            }
        };
        let placement = Placement::Board {
            bounds,
            rect,
            part,
            text_box,
        };
        let mut this = Self::fitted(
            engine,
            target,
            (&text, &spans),
            placement,
            data.rotation,
            data.style,
        );
        this.kanban = Some(kanban);
        this.new_card = new_card;
        Some(this)
    }

    /// Edits a new, empty card at place `at` of column `col` of kanban board `id`.
    pub fn new_card(
        engine: &mut TextEngine,
        doc: &Document,
        id: ObjectId,
        col: usize,
        at: usize,
    ) -> Option<Self> {
        let data = &doc.get(id)?.data;
        let ObjectKind::Kanban(kanban) = &data.kind else {
            return None;
        };
        let card = Card {
            id: doc.new_line_id().into(),
            column: kanban.columns.get(col)?.id.clone(),
            order: kanban.order_at(col, at, None),
            text: "".into(),
            spans: Default::default(),
        };
        let kanban = kanban.with_card(card);
        let i = kanban.cards.len() - 1;
        Self::board(
            engine,
            Target::Existing(id),
            data,
            kanban,
            BoardPart::Card(i),
            true,
        )
    }

    /// Edits card `card` (by id) of kanban board `id`.
    pub fn card(engine: &mut TextEngine, doc: &Document, id: ObjectId, card: &str) -> Option<Self> {
        let data = &doc.get(id)?.data;
        let ObjectKind::Kanban(kanban) = &data.kind else {
            return None;
        };
        let i = kanban.card_index(card)?;
        Self::board(
            engine,
            Target::Existing(id),
            data,
            kanban.clone(),
            BoardPart::Card(i),
            false,
        )
    }

    /// The board and the id of the card being edited, when editing a kanban card.
    pub fn edited_card(&self) -> Option<(ObjectId, std::sync::Arc<str>)> {
        match (self.target, self.placement, &self.kanban) {
            (
                Target::Existing(id),
                Placement::Board {
                    part: BoardPart::Card(i),
                    ..
                },
                Some(kanban),
            ) => Some((id, kanban.cards.get(i)?.id.clone())),
            _ => None,
        }
    }

    /// Where a card added after the edited one goes: its board, column and place.
    pub fn next_card(&self) -> Option<(ObjectId, usize, usize)> {
        let (id, card) = self.edited_card()?;
        let kanban = self.kanban.as_ref()?;
        let (col, at) = kanban.place_of(kanban.card_index(&card)?)?;
        Some((id, col, at + 1))
    }

    /// Edits cell (`row`, `col`) of the table object `data`.
    fn cell(
        engine: &mut TextEngine,
        target: Target,
        data: &ObjectData,
        table: &Table,
        row: usize,
        col: usize,
    ) -> Option<Self> {
        let bounds = data.bounds;
        let placement = Placement::Cell {
            bounds,
            rect: table.cell_rect(bounds, row, col)?,
            row,
            col,
            text_box: table.text_box(bounds, row, col, &data.style)?,
        };
        let content = table.text(row, col);
        let mut this = Self::fitted(
            engine,
            target,
            content,
            placement,
            data.rotation,
            data.style,
        );
        this.table = Some(table.clone());
        Some(this)
    }

    /// Edits cell (`row`, `col`) of table `id`; `None` if there is no such cell.
    pub fn table_cell(
        engine: &mut TextEngine,
        doc: &Document,
        id: ObjectId,
        (row, col): (usize, usize),
    ) -> Option<Self> {
        let data = &doc.get(id)?.data;
        let ObjectKind::Table(table) = &data.kind else {
            return None;
        };
        Self::cell(engine, Target::Existing(id), data, table, row, col)
    }

    /// The table and cell being edited, when editing a table cell.
    pub fn edited_cell(&self) -> Option<(ObjectId, (usize, usize))> {
        match (self.target, self.placement) {
            (Target::Existing(id), Placement::Cell { row, col, .. }) => Some((id, (row, col))),
            _ => None,
        }
    }

    /// The cell after this one (before it with `back`), when editing a table cell.
    pub fn next_cell(&self, back: bool) -> Option<(ObjectId, (usize, usize))> {
        let (Target::Existing(id), Placement::Cell { row, col, .. }, Some(table)) =
            (self.target, self.placement, &self.table)
        else {
            return None;
        };
        Some((id, table.next_cell(row, col, back)))
    }

    /// Sizes fitted text to fit, after it changed.
    fn refit(&mut self, engine: &mut TextEngine) {
        if let Some(text_box) = self.placement.text_box(&self.style) {
            let (text, spans) = (self.editor.display_text(), self.editor.display_spans());
            let size = fitted_font_size(engine, &text, &spans, text_box);
            self.editor.set_size(size);
        }
    }

    /// A new, empty sticky note next to this one, with the same size and style; `None` when
    /// this is not a sticky note.
    pub fn next_sticky(&mut self, engine: &mut TextEngine, below: bool) -> Option<Self> {
        let Placement::Fitted {
            bounds,
            shape: None,
        } = self.placement
        else {
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
            Placement::Free { origin, width } => {
                let layout = self.editor.layout(engine);
                let width = width.unwrap_or(layout.width.max(self.editor.size() * MIN_WIDTH));
                Rect::from_origin_size(origin, (width, layout.height))
            }
            Placement::Fitted { bounds, .. } => bounds,
            Placement::Cell { rect, .. } | Placement::Board { rect, .. } => rect,
            Placement::Label { center, .. } => {
                let layout = self.editor.layout(engine);
                let width = layout.width.max(self.editor.size() * MIN_WIDTH);
                Rect::from_center_size(center, (width, layout.height))
            }
            // Drawn the same way, see `pb_render`.
            Placement::Title { bounds, zoom } => {
                let layout = self.editor.layout(engine);
                let width = layout.width.max(self.editor.size() * MIN_WIDTH);
                let bottom = bounds.y0 - FrameData::TITLE_GAP / zoom;
                Rect::new(bounds.x0, bottom - layout.height, bounds.x0 + width, bottom)
            }
        };
        Frame::new(rect, self.rotation)
    }

    /// The object's bounds as they are now: the text box, except for a frame.
    fn object_bounds(&mut self, engine: &mut TextEngine) -> Rect {
        match self.placement {
            Placement::Title { bounds, .. }
            | Placement::Cell { bounds, .. }
            | Placement::Board { bounds, .. } => bounds,
            _ => self.frame(engine).rect,
        }
    }

    /// Top-left corner of the text layout, in the box's unrotated coordinates.
    fn text_origin(&mut self, engine: &mut TextEngine) -> Point {
        match self.placement {
            Placement::Free { origin, .. } => origin,
            Placement::Title { .. } => self.frame(engine).rect.origin(),
            Placement::Fitted { .. } | Placement::Cell { .. } | Placement::Board { .. } => {
                let text_box = self.placement.text_box(&self.style);
                let text_box = text_box.expect("fitted placement");
                let height = self.editor.layout(engine).height;
                Point::new(text_box.area.x0, text_box.top(height))
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

    /// Whether the text takes inline styles: connector labels, frame titles and column
    /// titles do not.
    pub fn is_rich(&self) -> bool {
        matches!(
            self.placement,
            Placement::Free { .. }
                | Placement::Fitted { .. }
                | Placement::Cell { .. }
                | Placement::Board {
                    part: BoardPart::Card(_),
                    ..
                }
        )
    }

    /// The marks of the selection or of what is typed next; `None` for plain text.
    pub fn marks(&self) -> Option<TextMarks> {
        self.is_rich().then(|| self.editor.marks())
    }

    /// Turns a mark on or off for the selection, or for what is typed next.
    pub fn toggle_mark(&mut self, engine: &mut TextEngine, mark: Mark) {
        if self.is_rich() {
            self.touch();
            self.editor.toggle(engine, mark);
            self.refit(engine);
        }
    }

    /// Colors the selection, or what is typed next; `None` for the object's text color.
    pub fn set_mark_color(&mut self, engine: &mut TextEngine, color: Option<Rgba>) {
        if self.is_rich() {
            self.touch();
            self.editor.set_color(engine, color);
            self.refit(engine);
        }
    }

    /// Restarts the caret's blinking, shown, after input.
    fn touch(&mut self) {
        self.active_at = Instant::now();
    }

    /// Whether the blinking caret is in its shown phase at `now`.
    fn caret_shown(&self, now: Instant) -> bool {
        let t = now.saturating_duration_since(self.active_at);
        t >= BLINK_FOR || (t.as_millis() / BLINK.as_millis()).is_multiple_of(2)
    }

    /// When the caret next appears or disappears; `None` once it has stopped blinking.
    pub fn next_blink(&self, now: Instant) -> Option<Instant> {
        let t = now.saturating_duration_since(self.active_at);
        if t >= BLINK_FOR {
            return None;
        }
        let phases = (t.as_millis() / BLINK.as_millis()) as u32 + 1;
        Some(self.active_at + BLINK * phases)
    }

    /// Whether the caret should be drawn differently from the last overlay.
    pub fn needs_blink_redraw(&self, now: Instant) -> bool {
        self.caret_shown(now) != self.caret_drawn
    }

    /// A press inside the box: places the cursor, or selects a word on a double click.
    pub fn press(&mut self, engine: &mut TextEngine, world: Point, shift: bool, double: bool) {
        self.touch();
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
        self.touch();
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
        self.touch();
        let ctrl = modifiers.control_key();
        let shift = modifiers.shift_key();
        let card = self.edited_card().is_some();
        let ed = &mut self.editor;
        // Board shortcuts by physical key, like elsewhere.
        if ctrl && let PhysicalKey::Code(code) = event.physical_key {
            match code {
                KeyCode::KeyA => ed.select_all(engine),
                KeyCode::KeyB | KeyCode::KeyI | KeyCode::KeyU | KeyCode::KeyX
                    if (code != KeyCode::KeyX) != shift =>
                {
                    let mark = match code {
                        KeyCode::KeyB => Mark::Bold,
                        KeyCode::KeyI => Mark::Italic,
                        KeyCode::KeyU => Mark::Underline,
                        _ => Mark::Strike,
                    };
                    self.toggle_mark(engine, mark);
                    return KeyOutcome::Handled;
                }
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
                // Undo and redo work inside the edit; before anything is typed, they
                // reach the board.
                KeyCode::KeyZ | KeyCode::KeyY => {
                    if !ed.can_undo() && !ed.can_redo() {
                        return KeyOutcome::FinishAndPass;
                    }
                    if code == KeyCode::KeyZ && !shift {
                        ed.undo(engine);
                    } else {
                        ed.redo(engine);
                    }
                }
                KeyCode::KeyS | KeyCode::KeyO | KeyCode::KeyN => {
                    return KeyOutcome::FinishAndPass;
                }
                _ => {}
            }
        }
        let word = ctrl;
        let sticky = matches!(self.placement, Placement::Fitted { shape: None, .. });
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => return KeyOutcome::Finish,
            Key::Named(NamedKey::Tab) if sticky && !ctrl && !ed.is_composing() => {
                return KeyOutcome::NextSticky { below: shift };
            }
            Key::Named(NamedKey::Tab)
                if matches!(self.placement, Placement::Cell { .. })
                    && !ctrl
                    && !ed.is_composing() =>
            {
                return KeyOutcome::NextCell { back: shift };
            }
            Key::Named(NamedKey::Tab) if card && !ctrl && !shift && !ed.is_composing() => {
                return KeyOutcome::NextCard;
            }
            Key::Named(NamedKey::Backspace) => ed.backspace(engine, word),
            Key::Named(NamedKey::Delete) => ed.delete(engine, word),
            // A title is one line: Enter keeps it.
            Key::Named(NamedKey::Enter)
                if matches!(
                    self.placement,
                    Placement::Title { .. }
                        | Placement::Board {
                            part: BoardPart::Title(_),
                            ..
                        }
                ) && !ed.is_composing() =>
            {
                return KeyOutcome::Finish;
            }
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
        self.touch();
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
        let bounds = self.object_bounds(engine);
        let text = display.into();
        let spans: pb_core::Spans = self.editor.display_spans().into();
        ObjectData {
            kind: match self.placement {
                Placement::Free { .. } => ObjectKind::Text(Text {
                    spans: spans.clone(),
                    text,
                    size: self.editor.size(),
                }),
                Placement::Fitted {
                    shape: Some(kind), ..
                } => ObjectKind::Shape(Shape {
                    spans: spans.clone(),
                    kind,
                    text,
                }),
                Placement::Title { .. } => ObjectKind::Frame(FrameData { title: text }),
                Placement::Cell { row, col, .. } => {
                    let table = self.table.clone().expect("cell of a table");
                    ObjectKind::Table(table.with_cell(row, col, Cell { text, spans }))
                }
                Placement::Board { part, .. } => {
                    let kanban = self.kanban.as_ref().expect("part of a board");
                    ObjectKind::Kanban(edited_board(kanban, part, text, spans))
                }
                Placement::Fitted { shape: None, .. } | Placement::Label { .. } => {
                    ObjectKind::Sticky(Sticky {
                        spans: spans.clone(),
                        text,
                    })
                }
            },
            bounds,
            rotation: frame.rotation,
            style: self.style,
            tags: Default::default(),
        }
    }

    /// Box, selection and caret for the overlay at `now`, in world space.
    pub fn overlay(&mut self, engine: &mut TextEngine, now: Instant) -> TextEditOverlay {
        let rects = self.editor.selection(engine);
        let selection: Vec<_> = rects.iter().map(|r| self.quad(engine, *r)).collect();
        // Like most editors, no caret while text is selected.
        let shown = selection.is_empty() && self.caret_shown(now);
        self.caret_drawn = shown;
        let caret = self.editor.caret(engine).filter(|_| shown);
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
    /// removed; a sticky note or shape stays, even a new note; a label left blank goes.
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
        let spans = self.editor.spans();
        if let (Target::Existing(id), Placement::Cell { row, col, .. }, Some(table)) =
            (self.target, self.placement, &self.table)
        {
            if text == self.original && spans == self.original_spans {
                return None;
            }
            return Some(Command::SetCell {
                id,
                row: table.rows.get(row)?.id.clone(),
                col: table.cols.get(col)?.id.clone(),
                cell: Cell {
                    text: text.into(),
                    spans: spans.into(),
                },
            });
        }
        if let (Target::Existing(id), Placement::Board { part, bounds, .. }, Some(kanban)) =
            (self.target, self.placement, &self.kanban)
        {
            let unchanged = text == self.original && spans == self.original_spans;
            let edited = edited_board(kanban, part, text.into(), spans.into());
            return match part {
                BoardPart::Card(_)
                    if self.new_card && edited.cards.last()?.text.trim().is_empty() =>
                {
                    None
                }
                BoardPart::Card(_) if unchanged && !self.new_card => None,
                BoardPart::Card(i) => Some(Command::SetCard {
                    id,
                    card: edited.cards.get(i)?.clone(),
                    bounds,
                }),
                BoardPart::Title(_) if text_unchanged(&self.original, &edited, part) => None,
                BoardPart::Title(_) => Some(Command::SetColumns {
                    id,
                    columns: edited.columns.clone(),
                    bounds,
                }),
            };
        }
        let empty = text.trim().is_empty() && matches!(self.placement, Placement::Free { .. });
        match self.target {
            Target::Existing(_) if text == self.original && spans == self.original_spans => None,
            Target::Existing(id) if empty => Some(Command::Delete(vec![id])),
            Target::Existing(id) => Some(Command::SetText {
                spans: spans.into(),
                id,
                text: text.into(),
                bounds: self.object_bounds(engine),
            }),
            Target::New if empty => None,
            Target::New => Some(Command::Create(vec![self.data(engine).into()])),
        }
    }
}

/// `kanban` with `text` in place of the card or column title `part`; titles stay plain.
fn edited_board(
    kanban: &Kanban,
    part: BoardPart,
    text: std::sync::Arc<str>,
    spans: pb_core::Spans,
) -> Kanban {
    match part {
        BoardPart::Card(i) => match kanban.cards.get(i) {
            Some(card) => kanban.with_card(Card {
                text,
                spans,
                ..card.clone()
            }),
            None => kanban.clone(),
        },
        BoardPart::Title(col) => {
            let mut columns = kanban.columns.to_vec();
            if let Some(column) = columns.get_mut(col) {
                column.title = text;
            }
            Kanban {
                columns: columns.into(),
                ..kanban.clone()
            }
        }
    }
}

/// Whether the title `part` of `edited` is still `original`.
fn text_unchanged(original: &str, edited: &Kanban, part: BoardPart) -> bool {
    match part {
        BoardPart::Title(col) => edited
            .columns
            .get(col)
            .is_some_and(|c| &*c.title == original),
        BoardPart::Card(_) => false,
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
                spans: Default::default(),
                text: text.into(),
                size: 20.0,
            }),
            bounds: Rect::new(10.0, 10.0, 60.0, 34.0),
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
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
        let unchanged = TextEdit::existing(&mut e, &doc, object, None, 1.0).unwrap();
        assert_eq!(unchanged.finish(&mut e), None);

        let mut edit = TextEdit::existing(&mut e, &doc, object, None, 1.0).unwrap();
        edit.editor.insert(&mut e, "d");
        match edit.finish(&mut e) {
            Some(Command::SetText {
                id, text, bounds, ..
            }) => {
                assert_eq!(id, object);
                assert_eq!(&*text, "abcd");
                assert_eq!(bounds.origin(), Point::new(10.0, 10.0));
            }
            other => panic!("expected SetText, got {other:?}"),
        }

        let mut cleared = TextEdit::existing(&mut e, &doc, object, None, 1.0).unwrap();
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
                spans: Default::default(),
                text: "idea".into(),
            }),
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            rotation: 0.0,
            style: sticky_style(),
            tags: Default::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        let mut edit = TextEdit::existing(&mut e, &doc, id, None, 1.0).unwrap();
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
    fn shape_text_is_fitted_into_the_shape() {
        let mut e = engine();
        let mut doc = Document::new();
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        let data = ObjectData {
            kind: pb_geom::ShapeKind::Ellipse.into(),
            bounds,
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        let mut edit = TextEdit::existing(&mut e, &doc, id, None, 1.0).unwrap();
        // Tab is for sticky notes only.
        assert!(edit.next_sticky(&mut e, false).is_none());
        edit.editor
            .insert(&mut e, &"text that has to shrink ".repeat(6));
        edit.refit(&mut e);
        let area = Shape::text_box(pb_geom::ShapeKind::Ellipse, bounds, &Style::default()).area;
        let layout = edit.editor.layout(&mut e);
        assert!(layout.height <= area.height() && layout.content_width <= area.width());
        assert_eq!(edit.frame(&mut e).rect, bounds);
        let shown = edit.data(&mut e);
        assert!(
            matches!(&shown.kind, ObjectKind::Shape(s) if s.kind == pb_geom::ShapeKind::Ellipse)
        );
        // Emptying it keeps the shape.
        edit.select_all(&mut e);
        edit.editor.backspace(&mut e, false);
        assert_eq!(edit.finish(&mut e), None);
    }

    #[test]
    fn aligned_shape_text_puts_the_caret_in_its_corner() {
        let mut e = engine();
        let mut doc = Document::new();
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        let style = Style {
            text_align: pb_core::HAlign::Right,
            text_valign: pb_core::VAlign::Top,
            ..Style::default()
        };
        let data = ObjectData {
            kind: ObjectKind::Shape(Shape {
                spans: Default::default(),
                kind: pb_geom::ShapeKind::Rectangle,
                text: "hi".into(),
            }),
            bounds,
            rotation: 0.0,
            style,
            tags: Default::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        let mut edit = TextEdit::existing(&mut e, &doc, id, None, 1.0).unwrap();
        edit.editor.motion(&mut e, Motion::TextEnd, false);
        let area = Shape::text_box(pb_geom::ShapeKind::Rectangle, bounds, &style).area;
        let (top, _) = edit.overlay(&mut e, Instant::now()).caret.unwrap();
        assert!((top.x - area.x1).abs() < 2.0, "{top:?} vs {area:?}");
        assert!((top.y - area.y0).abs() < 1.0, "{top:?} vs {area:?}");
    }

    #[test]
    fn caret_blinks_after_input_then_stays() {
        let mut e = engine();
        let (doc, object) = text_object("abc");
        let mut edit = TextEdit::existing(&mut e, &doc, object, None, 1.0).unwrap();
        let start = edit.active_at;
        let at = |ms| start + Duration::from_millis(ms);
        assert!(edit.overlay(&mut e, at(0)).caret.is_some());
        assert_eq!(edit.next_blink(at(0)), Some(at(530)));
        assert!(edit.needs_blink_redraw(at(600)));
        assert!(edit.overlay(&mut e, at(600)).caret.is_none());
        assert!(!edit.needs_blink_redraw(at(700)));
        assert!(edit.overlay(&mut e, at(1100)).caret.is_some());
        // Idle: it stays shown and stops waking the app.
        let idle = at(BLINK_FOR.as_millis() as u64 + 300);
        assert!(edit.overlay(&mut e, idle).caret.is_some());
        assert_eq!(edit.next_blink(idle), None);
    }

    #[test]
    fn narrowed_text_keeps_its_width_while_edited() {
        let mut e = engine();
        let (mut doc, object) = text_object("one two three four");
        // Wider than the text first, then narrowed to wrap.
        let natural = Rect::new(10.0, 10.0, 400.0, 34.0);
        doc.apply(Command::SetGeometry(vec![(
            object,
            Frame::new(natural, 0.0),
        )]))
        .unwrap();
        let mut edit = TextEdit::existing(&mut e, &doc, object, None, 1.0).unwrap();
        edit.editor.insert(&mut e, " five");
        let wide = edit.frame(&mut e).rect;
        assert!(
            wide.width() < natural.width(),
            "unwrapped text takes its own width"
        );
        assert!(wide.height() < 30.0, "on one line: {wide:?}");
        let narrow = Rect::new(10.0, 10.0, 70.0, 34.0);
        doc.apply(Command::SetGeometry(vec![(
            object,
            Frame::new(narrow, 0.0),
        )]))
        .unwrap();
        let mut edit = TextEdit::existing(&mut e, &doc, object, None, 1.0).unwrap();
        edit.editor.insert(&mut e, " five");
        let rect = edit.frame(&mut e).rect;
        assert_eq!(rect.width(), narrow.width());
        assert!(rect.height() > narrow.height(), "{rect:?}");
        let Some(Command::SetText { bounds, .. }) = edit.finish(&mut e) else {
            panic!("expected SetText");
        };
        assert_eq!(bounds.width(), narrow.width());
    }

    #[test]
    fn frame_titles_sit_above_and_end_with_enter() {
        let mut e = engine();
        let mut doc = Document::new();
        let bounds = Rect::new(0.0, 0.0, 400.0, 300.0);
        let data = ObjectData {
            kind: ObjectKind::Frame(FrameData {
                title: "Frame 1".into(),
            }),
            bounds,
            rotation: 0.0,
            style: Style::default(),
            tags: Default::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        let mut edit = TextEdit::existing(&mut e, &doc, id, None, 2.0).unwrap();
        let rect = edit.frame(&mut e).rect;
        assert!(rect.y1 < 0.0 && rect.x0 == 0.0, "{rect:?}");
        // Sized for the zoom: half as big in the world at 2x.
        assert_eq!(edit.editor.size(), FrameData::TITLE_SIZE / 2.0);
        edit.select_all(&mut e);
        edit.editor.insert(&mut e, "Roadmap");
        assert_eq!(edit.data(&mut e).bounds, bounds);
        let Some(Command::SetText {
            text, bounds: kept, ..
        }) = edit.finish(&mut e)
        else {
            panic!("expected SetText");
        };
        assert_eq!(&*text, "Roadmap");
        assert_eq!(kept, bounds);
    }

    #[test]
    fn styling_alone_is_an_edit_and_labels_take_none() {
        let mut e = engine();
        let (doc, object) = text_object("abc");
        let mut edit = TextEdit::existing(&mut e, &doc, object, None, 1.0).unwrap();
        assert_eq!(edit.marks(), Some(TextMarks::default()));
        edit.select_all(&mut e);
        edit.toggle_mark(&mut e, Mark::Bold);
        assert!(edit.marks().unwrap().bold);
        let Some(Command::SetText { text, spans, .. }) = edit.finish(&mut e) else {
            panic!("expected SetText");
        };
        assert_eq!(&*text, "abc");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].range(), 0..3);
    }

    #[test]
    fn table_cells_are_edited_one_at_a_time() {
        let mut e = engine();
        let mut doc = Document::new();
        let table = Table::new(2, 2, || doc.new_line_id());
        let data = ObjectData {
            kind: ObjectKind::Table(table.clone()),
            bounds: Rect::new(0.0, 0.0, 200.0, 100.0),
            rotation: 0.0,
            style: Table::style(),
            tags: Default::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        // A double click in the bottom-right cell edits it.
        let mut edit =
            TextEdit::existing(&mut e, &doc, id, Some(Point::new(150.0, 75.0)), 1.0).unwrap();
        assert_eq!(
            edit.frame(&mut e).rect,
            Rect::new(100.0, 50.0, 200.0, 100.0)
        );
        edit.editor.insert(&mut e, "Done");
        // What is drawn while typing is the table with the new text.
        let shown = edit.data(&mut e);
        let ObjectKind::Table(shown) = &shown.kind else {
            panic!("not a table");
        };
        assert_eq!(shown.text(1, 1).0, "Done");
        assert_eq!(edit.next_cell(false), Some((id, (0, 0))));
        assert_eq!(edit.next_cell(true), Some((id, (1, 0))));
        let Some(Command::SetCell { row, col, cell, .. }) = edit.finish(&mut e) else {
            panic!("expected SetCell");
        };
        assert_eq!(
            (row, col),
            (table.rows[1].id.clone(), table.cols[1].id.clone())
        );
        assert_eq!(&*cell.text, "Done");
        // Tab goes on in the next cell.
        let mut next = TextEdit::table_cell(&mut e, &doc, id, (0, 1)).unwrap();
        assert_eq!(next.frame(&mut e).rect, Rect::new(100.0, 0.0, 200.0, 50.0));
    }

    #[test]
    fn kanban_cards_and_titles_are_edited() {
        let mut e = engine();
        let mut doc = Document::new();
        let kanban = Kanban::starter(|| doc.new_line_id());
        let data = ObjectData {
            bounds: kanban.fitted(Rect::new(0.0, 0.0, 720.0, 1.0)),
            kind: ObjectKind::Kanban(kanban.clone()),
            rotation: 0.0,
            style: Kanban::style(),
            tags: Default::default(),
        };
        let id = doc
            .apply(Command::Create(vec![data.into()]))
            .unwrap()
            .created[0];
        // A new card left empty is not added.
        let edit = TextEdit::new_card(&mut e, &doc, id, 1, 0).unwrap();
        assert!(edit.finish(&mut e).is_none());
        // With text it is, and the board grows to hold it.
        let mut edit = TextEdit::new_card(&mut e, &doc, id, 1, 0).unwrap();
        edit.editor.insert(&mut e, "Ship it");
        let shown = edit.data(&mut e);
        let ObjectKind::Kanban(k) = &shown.kind else {
            panic!("not a kanban");
        };
        assert_eq!(k.cards_in(1).len(), 1);
        assert_eq!(edit.next_card(), Some((id, 1, 1)));
        let Some(cmd @ Command::SetCard { .. }) = edit.finish(&mut e) else {
            panic!("expected SetCard");
        };
        doc.apply(cmd).unwrap();
        let data = &doc.get(id).unwrap().data;
        let ObjectKind::Kanban(k) = &data.kind else {
            panic!("not a kanban");
        };
        assert_eq!(data.bounds.height(), k.fitted_height(720.0));
        assert!(data.bounds.height() > kanban.fitted_height(720.0));
        // A double click on a title renames the column; Enter ends it.
        let title = k.title_rect(data.bounds, 0).unwrap().center();
        let mut edit = TextEdit::existing(&mut e, &doc, id, Some(title), 1.0).unwrap();
        assert!(!edit.is_rich());
        edit.editor.select_all(&mut e);
        edit.editor.insert(&mut e, "Backlog");
        let Some(Command::SetColumns { columns, .. }) = edit.finish(&mut e) else {
            panic!("expected SetColumns");
        };
        assert_eq!(&*columns[0].title, "Backlog");
        assert_eq!(columns[1], k.columns[1]);
        // Nothing for the "add card" row.
        let add = k.add_rect(data.bounds, 2).unwrap().center();
        assert!(TextEdit::existing(&mut e, &doc, id, Some(add), 1.0).is_none());
    }

    #[test]
    fn clicks_map_through_the_box() {
        let mut e = engine();
        let (doc, object) = text_object("hello world");
        let mut edit = TextEdit::existing(&mut e, &doc, object, None, 1.0).unwrap();
        let frame = edit.frame(&mut e);
        assert!(edit.contains(&mut e, frame.center(), 0.0));
        assert!(!edit.contains(&mut e, Point::new(0.0, 0.0), 0.0));
        // Double click on the first word selects it.
        edit.press(&mut e, Point::new(15.0, 20.0), false, true);
        let overlay = edit.overlay(&mut e, Instant::now());
        assert_eq!(overlay.selection.len(), 1);
        assert!(overlay.caret.is_none());
    }

    #[test]
    fn labels_start_at_the_click_and_blank_ones_go() {
        let mut e = engine();
        let mut doc = Document::new();
        let line = ObjectData {
            kind: ObjectKind::Connector(pb_core::Connector {
                bends: Default::default(),
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
            tags: Default::default(),
        };
        let id = doc
            .apply(Command::Create(vec![line.into()]))
            .unwrap()
            .created[0];
        let near = Some(Point::new(100.0, 10.0));
        let untouched = TextEdit::existing(&mut e, &doc, id, near, 1.0).unwrap();
        assert_eq!(untouched.finish(&mut e), None);

        let mut edit = TextEdit::existing(&mut e, &doc, id, near, 1.0).unwrap();
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
        let mut edit = TextEdit::existing(&mut e, &doc, id, Some(Point::ZERO), 1.0).unwrap();
        edit.select_all(&mut e);
        edit.editor.insert(&mut e, "  ");
        let Some(Command::SetLabel { label, .. }) = edit.finish(&mut e) else {
            panic!("expected SetLabel");
        };
        assert!(label.is_empty());
        assert!((label.at - 0.25).abs() < 1e-6);
    }
}
