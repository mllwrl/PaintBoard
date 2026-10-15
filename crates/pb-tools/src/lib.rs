//! Canvas tools: select, pen, shapes, connectors, ...
//!
//! Tools turn pointer input (in world coordinates) into [`Command`]s and previews. They only
//! read the document, so the same input logic will drive local and remote edits.

mod clipboard;
mod handles;
mod hit;
mod selection;

pub use clipboard::{Clipboard, duplicate};
pub use handles::{Handle, rotation_knob};
pub use selection::{Selection, frame_of};

use pb_core::{Command, Document, Object, ObjectData, ObjectId, ObjectKind, Style};
use pb_geom::Frame;
use pb_geom::kurbo::{Point, Rect, Vec2};

/// Drags shorter than this on screen (logical pixels) count as clicks.
const CLICK_SLOP: f64 = 4.0;
/// Extra hit area around objects, in logical pixels.
const HIT_SLOP: f64 = 3.0;
/// Hit radius of resize and rotation handles, in logical pixels.
const HANDLE_HIT_RADIUS: f64 = 7.0;
/// Size of a shape created by a click instead of a drag, in world units.
const DEFAULT_SHAPE_SIZE: f64 = 100.0;
/// Rotation snaps to multiples of this with Shift held.
const ROTATION_SNAP: f64 = std::f64::consts::PI / 12.0;

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
    Copy,
    Rotate,
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
    pub alt: bool,
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

/// A grabbable part of the selection frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grip {
    Resize(Handle),
    Rotate,
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
        /// Alt was held at the start: drop copies and leave the originals in place.
        duplicate: bool,
        /// Clicking (not dragging) an object of a multi-selection selects just that object.
        select_only_on_click: Option<ObjectId>,
    },
    Resize {
        handle: Handle,
        from: Frame,
        start: Point,
        current: Pointer,
    },
    Rotate {
        from: Frame,
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
    hover_grip: Option<Grip>,
    /// Rotation of the frame owning `hover_grip`, to orient its resize cursor.
    hover_rotation: f64,
}

impl Tools {
    pub fn active(&self) -> ToolKind {
        self.active
    }

    pub fn set_active(&mut self, tool: ToolKind) {
        self.active = tool;
        self.gesture = None;
        self.hover = None;
        self.hover_grip = None;
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

        if let Some((from, grip)) = self.grip_at(doc, pointer) {
            self.gesture = Some(match grip {
                Grip::Resize(handle) => Gesture::Resize {
                    handle,
                    from,
                    start: p,
                    current: pointer,
                },
                Grip::Rotate => Gesture::Rotate {
                    from,
                    start: p,
                    current: pointer,
                },
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
                    duplicate: pointer.alt,
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
            Some(Gesture::Shape { current, .. })
            | Some(Gesture::Resize { current, .. })
            | Some(Gesture::Rotate { current, .. }) => {
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
        let command = match self.gesture.as_ref()? {
            Gesture::Shape {
                kind,
                start,
                current,
            } => {
                // Like Miro, a shape tool is one-shot: go back to selecting afterwards.
                self.active = ToolKind::Select;
                Some(Command::Create(vec![shape(
                    *kind,
                    shape_rect(*start, *current, true),
                )]))
            }
            Gesture::Move {
                dragging: true,
                duplicate: true,
                ..
            } => Some(Command::Create(self.moved_copies(doc))),
            Gesture::Move {
                start,
                current,
                dragging: true,
                ..
            } => Some(Command::Move {
                ids: self.selection.ids().to_vec(),
                delta: current.world - *start,
            }),
            Gesture::Move {
                select_only_on_click,
                ..
            } => {
                if let Some(id) = *select_only_on_click {
                    self.selection.set([id]);
                }
                None
            }
            Gesture::Resize { .. } | Gesture::Rotate { .. } => {
                let changes = self.geometry_changes(doc);
                let changed = changes
                    .iter()
                    .any(|(id, f)| doc.get(*id).is_some_and(|o| o.data.frame() != *f));
                changed.then_some(Command::SetGeometry(changes))
            }
            Gesture::Marquee { .. } => None,
        };
        self.gesture = None;
        command
    }

    /// Objects as they look mid-gesture (moved, resized, rotated), replacing stored state.
    pub fn overrides(&self, doc: &Document) -> Vec<Object> {
        match &self.gesture {
            Some(Gesture::Move {
                start,
                current,
                dragging: true,
                duplicate: false,
                ..
            }) => {
                let delta = current.world - *start;
                self.selected(doc)
                    .map(|o| {
                        let mut o = *o;
                        o.data.bounds = o.data.bounds + delta;
                        o
                    })
                    .collect()
            }
            Some(Gesture::Resize { .. } | Gesture::Rotate { .. }) => self
                .geometry_changes(doc)
                .into_iter()
                .filter_map(|(id, frame)| {
                    let mut o = *doc.get(id)?;
                    o.data.set_frame(frame);
                    Some(o)
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// New objects the gesture would create (shape being drawn, Alt-drag copies).
    pub fn previews(&self, doc: &Document) -> Vec<ObjectData> {
        match &self.gesture {
            Some(Gesture::Shape {
                kind,
                start,
                current,
            }) => vec![shape(*kind, shape_rect(*start, *current, false))],
            Some(Gesture::Move {
                dragging: true,
                duplicate: true,
                ..
            }) => self.moved_copies(doc),
            _ => Vec::new(),
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

    /// Resize and rotation handles are shown only when nothing else is going on.
    pub fn shows_handles(&self) -> bool {
        self.active == ToolKind::Select
            && !matches!(
                self.gesture,
                Some(Gesture::Move { dragging: true, .. })
                    | Some(Gesture::Marquee { .. })
                    | Some(Gesture::Rotate { .. })
            )
    }

    pub fn cursor(&self) -> CursorHint {
        match &self.gesture {
            Some(Gesture::Resize { handle, from, .. }) => handle.cursor(from.rotation),
            Some(Gesture::Rotate { .. }) => CursorHint::Rotate,
            Some(Gesture::Move {
                dragging: true,
                duplicate,
                ..
            }) => {
                if *duplicate {
                    CursorHint::Copy
                } else {
                    CursorHint::Move
                }
            }
            Some(Gesture::Shape { .. }) => CursorHint::Crosshair,
            _ if self.active != ToolKind::Select => CursorHint::Crosshair,
            _ => match self.hover_grip {
                Some(Grip::Resize(handle)) => handle.cursor(self.hover_rotation),
                Some(Grip::Rotate) => CursorHint::Rotate,
                None => CursorHint::Default,
            },
        }
    }

    fn selected<'a>(&'a self, doc: &'a Document) -> impl Iterator<Item = &'a Object> + 'a {
        self.selection.ids().iter().filter_map(|&id| doc.get(id))
    }

    /// The selection-frame grip under the pointer, with the frame it belongs to.
    fn grip_at(&self, doc: &Document, pointer: Pointer) -> Option<(Frame, Grip)> {
        let frame = self.selection.frame(doc)?;
        let radius = HANDLE_HIT_RADIUS / pointer.zoom;
        if (rotation_knob(frame, pointer.zoom) - pointer.world).hypot() <= radius {
            return Some((frame, Grip::Rotate));
        }
        handles::handle_at(frame, pointer.world, radius).map(|h| (frame, Grip::Resize(h)))
    }

    fn update_hover(&mut self, doc: &Document, pointer: Pointer) -> bool {
        if self.active != ToolKind::Select {
            return false;
        }
        let grip = self.grip_at(doc, pointer);
        let hover = match grip {
            Some(_) => None,
            None => hit::object_at(doc.objects(), pointer.world, HIT_SLOP / pointer.zoom),
        };
        let grip_kind = grip.map(|(_, g)| g);
        let changed = hover != self.hover || grip_kind != self.hover_grip;
        self.hover = hover;
        self.hover_grip = grip_kind;
        self.hover_rotation = grip.map_or(0.0, |(f, _)| f.rotation);
        changed
    }

    fn moved_copies(&self, doc: &Document) -> Vec<ObjectData> {
        let Some(Gesture::Move { start, current, .. }) = &self.gesture else {
            return Vec::new();
        };
        let delta = current.world - *start;
        self.selected(doc)
            .map(|o| {
                let mut data = o.data;
                data.bounds = data.bounds + delta;
                data
            })
            .collect()
    }

    /// New frames of the selected objects for the resize or rotate gesture in progress.
    fn geometry_changes(&self, doc: &Document) -> Vec<(ObjectId, Frame)> {
        match &self.gesture {
            Some(Gesture::Resize {
                handle,
                from,
                start,
                current,
            }) => {
                // Resize happens in the frame's own (unrotated) space.
                let delta = from.vec_to_local(current.world - *start);
                let to = handles::resize(from.rect, *handle, delta, current.shift);
                if self.selection.ids().len() == 1 {
                    let center = from.to_world(to.center());
                    let rect = Rect::from_center_size(center, to.size()).abs();
                    self.selected(doc)
                        .map(|o| (o.id, Frame::new(rect, from.rotation)))
                        .collect()
                } else {
                    // A multi-selection frame is axis-aligned; children keep their rotation.
                    self.selected(doc)
                        .map(|o| {
                            let rect = handles::map_rect(o.data.bounds, from.rect, to);
                            (o.id, Frame::new(rect, o.data.rotation))
                        })
                        .collect()
                }
            }
            Some(Gesture::Rotate {
                from,
                start,
                current,
            }) => {
                let pivot = from.center();
                let angle_of = |p: Point| (p - pivot).atan2();
                let mut delta = angle_of(current.world) - angle_of(*start);
                if current.shift {
                    // Snap the resulting angle of a single object, or the turn of a group.
                    let base = if self.selection.ids().len() == 1 {
                        from.rotation
                    } else {
                        0.0
                    };
                    delta = ((base + delta) / ROTATION_SNAP).round() * ROTATION_SNAP - base;
                }
                self.selected(doc)
                    .map(|o| (o.id, o.data.frame().rotated_about(pivot, delta)))
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}

fn shape(kind: ObjectKind, bounds: Rect) -> ObjectData {
    ObjectData {
        kind,
        bounds,
        rotation: 0.0,
        style: Style::default(),
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
