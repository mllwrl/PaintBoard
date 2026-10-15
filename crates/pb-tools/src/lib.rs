//! Canvas tools: select, pen, shapes, connectors, ...
//!
//! Tools turn pointer input (in world coordinates) into [`Command`]s and previews. They only
//! read the document, so the same input logic will drive local and remote edits.

mod handles;
mod hit;
mod selection;

pub use handles::Handle;
pub use selection::Selection;

use pb_core::{Command, Document, Object, ObjectData, ObjectId, ObjectKind, Style};
use pb_geom::kurbo::{Point, Rect, Vec2};

/// Drags shorter than this on screen (logical pixels) count as clicks.
const CLICK_SLOP: f64 = 4.0;
/// Extra hit area around objects, in logical pixels.
const HIT_SLOP: f64 = 3.0;
/// Hit radius of resize handles, in logical pixels.
const HANDLE_HIT_RADIUS: f64 = 7.0;
/// Size of a shape created by a click instead of a drag, in world units.
const DEFAULT_SHAPE_SIZE: f64 = 100.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolKind {
    #[default]
    Select,
    Rectangle,
}

impl ToolKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Rectangle => "Rectangle",
        }
    }
}

/// Platform-neutral mouse cursor the tools ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorHint {
    Default,
    Crosshair,
    Move,
    ResizeNs,
    ResizeEw,
    ResizeNwse,
    ResizeNesw,
}

#[derive(Clone, Copy, Debug)]
pub struct Pointer {
    pub world: Point,
    /// Current camera zoom, to measure distances in screen pixels.
    pub zoom: f64,
    pub shift: bool,
    pub ctrl: bool,
}

impl Pointer {
    /// Shift or Ctrl add to / remove from the selection.
    fn toggles_selection(&self) -> bool {
        self.shift || self.ctrl
    }

    fn screen_distance(&self, from: Point) -> f64 {
        (self.world - from).hypot() * self.zoom
    }
}

enum Gesture {
    Shape {
        kind: ObjectKind,
        start: Point,
        current: Pointer,
    },
    Move {
        start: Point,
        current: Pointer,
        /// Set once the pointer leaves the click slop; until then nothing moves.
        dragging: bool,
        /// Clicking (not dragging) an object of a multi-selection selects just that object.
        select_only_on_click: Option<ObjectId>,
    },
    Resize {
        handle: Handle,
        from: Rect,
        start: Point,
        current: Pointer,
    },
    Marquee {
        start: Point,
        current: Point,
        /// Selection before the marquee started (kept when adding with Shift/Ctrl).
        base: Vec<ObjectId>,
    },
}

#[derive(Default)]
pub struct Tools {
    active: ToolKind,
    selection: Selection,
    gesture: Option<Gesture>,
    hover: Option<ObjectId>,
    hover_handle: Option<Handle>,
}

impl Tools {
    pub fn active(&self) -> ToolKind {
        self.active
    }

    pub fn set_active(&mut self, tool: ToolKind) {
        self.active = tool;
        self.gesture = None;
        self.hover = None;
        self.hover_handle = None;
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn select_only(&mut self, ids: impl IntoIterator<Item = ObjectId>) {
        self.selection.set(ids);
    }

    pub fn select_all(&mut self, doc: &Document) {
        self.set_active(ToolKind::Select);
        self.selection.set(doc.objects().iter().map(|o| o.id));
    }

    /// Returns true if something was selected.
    pub fn clear_selection(&mut self) -> bool {
        let had = !self.selection.is_empty();
        self.selection.clear();
        had
    }

    /// Call after every document change: objects may have disappeared.
    pub fn sync(&mut self, doc: &Document) {
        self.selection.retain_existing(doc);
        if self.hover.is_some_and(|id| doc.get(id).is_none()) {
            self.hover = None;
        }
    }

    /// Abandons the current gesture. Returns true if there was one.
    pub fn cancel(&mut self) -> bool {
        match self.gesture.take() {
            Some(Gesture::Marquee { base, .. }) => {
                self.selection.set(base);
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    pub fn delete_selected(&self) -> Option<Command> {
        (!self.selection.is_empty()).then(|| Command::Delete(self.selection.ids().to_vec()))
    }

    pub fn nudge_selected(&self, delta: Vec2) -> Option<Command> {
        (!self.selection.is_empty() && self.gesture.is_none()).then(|| Command::Move {
            ids: self.selection.ids().to_vec(),
            delta,
        })
    }

    /// Returns true if the view needs a redraw.
    pub fn pointer_down(&mut self, doc: &Document, pointer: Pointer) -> bool {
        let p = pointer.world;
        if let ToolKind::Rectangle = self.active {
            self.gesture = Some(Gesture::Shape {
                kind: ObjectKind::Rectangle,
                start: p,
                current: pointer,
            });
            return true;
        }

        if let Some(from) = self.selection.bounds(doc)
            && let Some(handle) = handles::handle_at(from, p, HANDLE_HIT_RADIUS / pointer.zoom)
        {
            self.gesture = Some(Gesture::Resize {
                handle,
                from,
                start: p,
                current: pointer,
            });
            return true;
        }

        let toggle = pointer.toggles_selection();
        match hit::object_at(doc.objects(), p, HIT_SLOP / pointer.zoom) {
            Some(id) => {
                let mut select_only_on_click = None;
                if toggle && self.selection.contains(id) {
                    self.selection.remove(id);
                    return true;
                } else if toggle {
                    self.selection.add(id);
                } else if self.selection.contains(id) {
                    select_only_on_click = Some(id);
                } else {
                    self.selection.set([id]);
                }
                self.gesture = Some(Gesture::Move {
                    start: p,
                    current: pointer,
                    dragging: false,
                    select_only_on_click,
                });
            }
            None => {
                if !toggle {
                    self.selection.clear();
                }
                self.gesture = Some(Gesture::Marquee {
                    start: p,
                    current: p,
                    base: self.selection.ids().to_vec(),
                });
            }
        }
        self.hover = None;
        true
    }

    /// Returns true if the view needs a redraw.
    pub fn pointer_move(&mut self, doc: &Document, pointer: Pointer) -> bool {
        match &mut self.gesture {
            Some(Gesture::Shape { current, .. }) | Some(Gesture::Resize { current, .. }) => {
                *current = pointer;
                true
            }
            Some(Gesture::Move {
                start,
                current,
                dragging,
                ..
            }) => {
                *current = pointer;
                *dragging |= pointer.screen_distance(*start) >= CLICK_SLOP;
                *dragging
            }
            Some(Gesture::Marquee {
                start,
                current,
                base,
            }) => {
                *current = pointer.world;
                let area = Rect::from_points(*start, *current);
                let touched = hit::objects_in(doc.objects(), area);
                let ids: Vec<_> = base.iter().copied().chain(touched).collect();
                self.selection.set(ids);
                true
            }
            None => self.update_hover(doc, pointer),
        }
    }

    pub fn pointer_up(&mut self, doc: &Document, pointer: Pointer) -> Option<Command> {
        self.pointer_move(doc, pointer);
        match self.gesture.take()? {
            Gesture::Shape {
                kind,
                start,
                current,
            } => {
                // Like Miro, a shape tool is one-shot: go back to selecting afterwards.
                self.active = ToolKind::Select;
                Some(Command::Create(ObjectData {
                    kind,
                    bounds: shape_rect(start, current, true),
                    style: Style::default(),
                }))
            }
            Gesture::Move {
                start,
                current,
                dragging,
                select_only_on_click,
            } => {
                if dragging {
                    Some(Command::Move {
                        ids: self.selection.ids().to_vec(),
                        delta: current.world - start,
                    })
                } else {
                    if let Some(id) = select_only_on_click {
                        self.selection.set([id]);
                    }
                    None
                }
            }
            Gesture::Resize {
                handle,
                from,
                start,
                current,
            } => {
                let to = handles::resize(from, handle, current.world - start, current.shift);
                (to != from).then(|| Command::SetBounds(self.resized(doc, from, to)))
            }
            Gesture::Marquee { .. } => None,
        }
    }

    /// Objects as they look mid-gesture (moved or resized), replacing their stored state.
    pub fn overrides(&self, doc: &Document) -> Vec<Object> {
        let selected = || self.selection.ids().iter().filter_map(|&id| doc.get(id));
        match &self.gesture {
            Some(Gesture::Move {
                start,
                current,
                dragging: true,
                ..
            }) => {
                let delta = current.world - *start;
                selected()
                    .map(|o| {
                        let mut o = *o;
                        o.data.bounds = o.data.bounds + delta;
                        o
                    })
                    .collect()
            }
            Some(Gesture::Resize {
                handle,
                from,
                start,
                current,
            }) => {
                let to = handles::resize(*from, *handle, current.world - *start, current.shift);
                self.resized(doc, *from, to)
                    .into_iter()
                    .filter_map(|(id, bounds)| {
                        let mut o = *doc.get(id)?;
                        o.data.bounds = bounds;
                        Some(o)
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// What the shape tool would create, drawn on top of the board.
    pub fn preview(&self) -> Option<ObjectData> {
        match &self.gesture {
            Some(Gesture::Shape {
                kind,
                start,
                current,
            }) => Some(ObjectData {
                kind: *kind,
                bounds: shape_rect(*start, *current, false),
                style: Style::default(),
            }),
            _ => None,
        }
    }

    pub fn marquee(&self) -> Option<Rect> {
        match &self.gesture {
            Some(Gesture::Marquee { start, current, .. }) => {
                Some(Rect::from_points(*start, *current))
            }
            _ => None,
        }
    }

    /// Object under the pointer that is not selected, for a hover outline.
    pub fn hover(&self) -> Option<ObjectId> {
        self.hover.filter(|&id| !self.selection.contains(id))
    }

    /// Resize handles are shown only when nothing else is going on.
    pub fn shows_handles(&self) -> bool {
        self.active == ToolKind::Select
            && !matches!(
                self.gesture,
                Some(Gesture::Move { dragging: true, .. }) | Some(Gesture::Marquee { .. })
            )
    }

    pub fn cursor(&self) -> CursorHint {
        match &self.gesture {
            Some(Gesture::Resize { handle, .. }) => handle.cursor(),
            Some(Gesture::Move { dragging: true, .. }) => CursorHint::Move,
            Some(Gesture::Shape { .. }) => CursorHint::Crosshair,
            _ if self.active != ToolKind::Select => CursorHint::Crosshair,
            _ => self
                .hover_handle
                .map_or(CursorHint::Default, Handle::cursor),
        }
    }

    fn update_hover(&mut self, doc: &Document, pointer: Pointer) -> bool {
        if self.active != ToolKind::Select {
            return false;
        }
        let handle = self
            .selection
            .bounds(doc)
            .and_then(|b| handles::handle_at(b, pointer.world, HANDLE_HIT_RADIUS / pointer.zoom));
        let hover = match handle {
            Some(_) => None,
            None => hit::object_at(doc.objects(), pointer.world, HIT_SLOP / pointer.zoom),
        };
        let changed = hover != self.hover || handle != self.hover_handle;
        self.hover = hover;
        self.hover_handle = handle;
        changed
    }

    fn resized(&self, doc: &Document, from: Rect, to: Rect) -> Vec<(ObjectId, Rect)> {
        self.selection
            .ids()
            .iter()
            .filter_map(|&id| doc.get(id))
            .map(|o| (o.id, handles::map_rect(o.data.bounds, from, to)))
            .collect()
    }
}

/// Rectangle drawn by a shape gesture; Shift makes it a square. On release (`finished`),
/// a click instead of a drag creates a default-sized shape centered on the click point.
fn shape_rect(start: Point, current: Pointer, finished: bool) -> Rect {
    if finished && current.screen_distance(start) < CLICK_SLOP {
        return Rect::from_center_size(start, (DEFAULT_SHAPE_SIZE, DEFAULT_SHAPE_SIZE));
    }
    let mut delta = current.world - start;
    if current.shift {
        let side = delta.x.abs().max(delta.y.abs());
        delta = Vec2::new(side.copysign(delta.x), side.copysign(delta.y));
    }
    Rect::from_points(start, start + delta)
}

#[cfg(test)]
mod tests;
