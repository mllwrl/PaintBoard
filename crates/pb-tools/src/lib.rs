//! Canvas tools: select, pen, shapes, connectors, ...
//!
//! Tools turn pointer input (in world coordinates) into [`Command`]s and previews. They only
//! read the document, so the same input logic will drive local and remote edits.

mod arrange;
mod clipboard;
mod handles;
mod hit;
mod ink;
pub mod math;
mod selection;

pub use arrange::{Align, Axis, align, distribute};
pub use clipboard::{Clipboard, duplicate};
pub use handles::{Handle, rotation_knob};
pub use ink::{ERASER_RADIUS, InkStyle};
pub use selection::{Selection, selection_frame, unit_frame};

use pb_core::{
    Command, Content, Document, Object, ObjectData, ObjectId, ObjectKind, ShapeKind, Style,
};
use pb_geom::kurbo::{Line, Point, Rect, Vec2};
use pb_geom::{Frame, GridLevel, Snapper, xs_of, ys_of};

/// Drags shorter than this on screen (logical pixels) count as clicks.
const CLICK_SLOP: f64 = 4.0;
/// Extra hit area around objects, in logical pixels.
const HIT_SLOP: f64 = 3.0;
/// Hit radius of resize and rotation handles, in logical pixels.
const HANDLE_HIT_RADIUS: f64 = 7.0;
/// Size of a shape created by a click instead of a drag, in world units.
const DEFAULT_SHAPE_SIZE: f64 = 100.0;
/// Dragged edges snap to other objects within this distance, in logical pixels.
const SNAP_DISTANCE: f64 = 6.0;
/// Rotation snaps to multiples of this with Shift held.
const ROTATION_SNAP: f64 = std::f64::consts::PI / 12.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolKind {
    #[default]
    Select,
    Shape(ShapeKind),
    Pen,
    /// A wide, see-through pen, like a highlighter.
    Marker,
    /// Removes whole ink strokes it touches.
    Eraser,
    /// Click to type. The app runs the text editor; the tool only marks the mode.
    Text,
    /// Click to place a sticky note and type on it; run by the app like [`Self::Text`].
    Sticky,
}

impl ToolKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Shape(shape) => shape.name(),
            Self::Pen => "Pen",
            Self::Marker => "Marker",
            Self::Eraser => "Eraser",
            Self::Text => "Text",
            Self::Sticky => "Sticky note",
        }
    }

    /// Pen or marker. Like Miro, these stay active after a stroke, and strokes are not
    /// selected.
    pub fn draws_ink(self) -> bool {
        matches!(self, Self::Pen | Self::Marker)
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
    /// The part of the world on screen; only visible objects are snapped to.
    pub visible: Rect,
    /// Pen pressure in `[0, 1]`; 1 for a mouse.
    pub pressure: f64,
    pub shift: bool,
    /// Held during a drag, turns snapping off.
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
        kind: ShapeKind,
        start: Point,
        /// `start` after snapping: the corner the shape grows from.
        anchor: Point,
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
    Ink {
        marker: bool,
        /// World points with pressure.
        points: Vec<(Point, f64)>,
        /// Zoom when the stroke started; its width and smoothing follow it.
        zoom: f64,
    },
    Erase {
        last: Point,
        /// Strokes touched so far; hidden until the eraser is released.
        erased: Vec<ObjectId>,
    },
}

pub struct Tools {
    active: ToolKind,
    selection: Selection,
    gesture: Option<Gesture>,
    hover: Option<ObjectId>,
    hover_grip: Option<Grip>,
    /// Rotation of the frame owning `hover_grip`, to orient its resize cursor.
    hover_rotation: f64,
    snap_to_grid: bool,
    /// Shape the shape tool starts with.
    last_shape: ShapeKind,
    pen: InkStyle,
    marker: InkStyle,
    /// Where the eraser hovers, to show its size.
    eraser_at: Option<(Point, f64)>,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            active: ToolKind::default(),
            selection: Selection::default(),
            gesture: None,
            hover: None,
            hover_grip: None,
            hover_rotation: 0.0,
            snap_to_grid: false,
            last_shape: ShapeKind::Rectangle,
            pen: InkStyle::PEN,
            marker: InkStyle::MARKER,
            eraser_at: None,
        }
    }
}

impl Tools {
    pub fn active(&self) -> ToolKind {
        self.active
    }

    pub fn set_active(&mut self, tool: ToolKind) {
        if let ToolKind::Shape(shape) = tool {
            self.last_shape = shape;
        }
        self.active = tool;
        self.gesture = None;
        self.hover = None;
        self.hover_grip = None;
        self.eraser_at = None;
    }

    /// Color and width of the pen, or of the marker.
    pub fn ink_style(&self, marker: bool) -> InkStyle {
        if marker { self.marker } else { self.pen }
    }

    pub fn set_ink_style(&mut self, marker: bool, style: InkStyle) {
        if marker {
            self.marker = style;
        } else {
            self.pen = style;
        }
    }

    /// The shape the shape tool starts with.
    pub fn last_shape(&self) -> ShapeKind {
        self.last_shape
    }

    /// Whether a drag (move, resize, marquee, drawing) is in progress.
    pub fn is_busy(&self) -> bool {
        self.gesture.is_some()
    }

    /// Forgets the hovered object, e.g. when the pointer moves onto the UI.
    pub fn clear_hover(&mut self) -> bool {
        let had = self.hover.is_some() || self.hover_grip.is_some();
        self.hover = None;
        self.hover_grip = None;
        had
    }

    /// Picks the shape tool: the next kind if it is already active, else the last one used.
    pub fn cycle_shape(&mut self) {
        let shape = match self.active {
            ToolKind::Shape(shape) => shape.next(),
            _ => self.last_shape,
        };
        self.set_active(ToolKind::Shape(shape));
    }

    pub fn snap_to_grid(&self) -> bool {
        self.snap_to_grid
    }

    pub fn set_snap_to_grid(&mut self, on: bool) {
        self.snap_to_grid = on;
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn select_only(&mut self, ids: impl IntoIterator<Item = ObjectId>) {
        self.selection.set(ids);
    }

    /// Selects everything that is not locked.
    pub fn select_all(&mut self, doc: &Document) {
        self.set_active(ToolKind::Select);
        let ids = doc.roots().iter().filter(|&&id| !doc.is_locked(id));
        self.selection.set(ids.copied());
    }

    /// Returns true if something was selected.
    pub fn clear_selection(&mut self) -> bool {
        let had = !self.selection.is_empty();
        self.selection.clear();
        had
    }

    /// Call after every document change: objects may have disappeared or been grouped.
    pub fn sync(&mut self, doc: &Document) {
        self.selection.retain_existing(doc);
        if self.hover.is_some_and(|id| !doc.contains(id)) {
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

    /// Deletes the selection, except locked objects.
    pub fn delete_selected(&self, doc: &Document) -> Option<Command> {
        let ids = self.unlocked(doc);
        (!ids.is_empty()).then_some(Command::Delete(ids))
    }

    pub fn nudge_selected(&self, doc: &Document, delta: Vec2) -> Option<Command> {
        let ids = self.unlocked(doc);
        (!ids.is_empty() && self.gesture.is_none()).then_some(Command::Move { ids, delta })
    }

    /// Groups the selection (Ctrl+G). Needs two or more objects, none of them locked.
    pub fn group_selected(&self, doc: &Document) -> Option<Command> {
        let ids = self.selection.ids();
        (ids.len() > 1 && !self.selection.has_locked(doc)).then(|| Command::Group(ids.to_vec()))
    }

    /// Dissolves the selected groups that are not locked (Ctrl+Shift+G).
    pub fn ungroup_selected(&self, doc: &Document) -> Option<Command> {
        let ids: Vec<_> = self
            .unlocked(doc)
            .into_iter()
            .filter(|&id| doc.group(id).is_some())
            .collect();
        (!ids.is_empty()).then_some(Command::Ungroup(ids))
    }

    /// Locks the selection, or unlocks it if everything in it is locked (Ctrl+Shift+L).
    pub fn toggle_lock_selected(&self, doc: &Document) -> Option<Command> {
        let ids = self.selection.ids();
        let locked = !ids.iter().all(|&id| doc.is_locked(id));
        (!ids.is_empty()).then(|| Command::SetLocked {
            ids: ids.to_vec(),
            locked,
        })
    }

    /// Returns true if the view needs a redraw.
    pub fn pointer_down(&mut self, doc: &Document, pointer: Pointer) -> bool {
        let p = pointer.world;
        match self.active {
            ToolKind::Pen | ToolKind::Marker => {
                let marker = self.active == ToolKind::Marker;
                // A highlighter has an even width.
                let pressure = if marker { 1.0 } else { pointer.pressure };
                self.gesture = Some(Gesture::Ink {
                    marker,
                    points: vec![(p, pressure)],
                    zoom: pointer.zoom,
                });
                return true;
            }
            // Text editing is handled by the app.
            ToolKind::Text | ToolKind::Sticky => return false,
            ToolKind::Eraser => {
                let radius = ERASER_RADIUS / pointer.zoom;
                self.gesture = Some(Gesture::Erase {
                    last: p,
                    erased: ink::erased(doc, p, p, radius),
                });
                return true;
            }
            _ => {}
        }
        if let ToolKind::Shape(shape) = self.active {
            self.gesture = Some(Gesture::Shape {
                kind: shape,
                start: p,
                anchor: self.snap_point(doc, pointer).0,
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
        match hit::object_at(doc, p, HIT_SLOP / pointer.zoom) {
            Some(id) => {
                let mut select_only_on_click = None;
                if toggle && self.selection.contains(id) {
                    self.selection.remove(id);
                    return true;
                } else if doc.is_locked(id) && !toggle {
                    // Locked objects can be selected (to unlock or copy them), not dragged.
                    self.selection.set([id]);
                    self.hover = None;
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
                let touched = hit::objects_in(doc, area);
                let ids: Vec<_> = base.iter().copied().chain(touched).collect();
                self.selection.set(ids);
                true
            }
            Some(Gesture::Ink { marker, points, .. }) => {
                let last = points.last().map(|(p, _)| *p);
                if last.is_some_and(|last| pointer.screen_distance(last) < ink::MIN_STEP) {
                    return false;
                }
                let pressure = if *marker { 1.0 } else { pointer.pressure };
                points.push((pointer.world, pressure));
                true
            }
            Some(Gesture::Erase { last, erased }) => {
                let radius = ERASER_RADIUS / pointer.zoom;
                for id in ink::erased(doc, *last, pointer.world, radius) {
                    if !erased.contains(&id) {
                        erased.push(id);
                    }
                }
                *last = pointer.world;
                self.eraser_at = Some((pointer.world, radius));
                true
            }
            None if self.active == ToolKind::Eraser => {
                self.eraser_at = Some((pointer.world, ERASER_RADIUS / pointer.zoom));
                true
            }
            None => self.update_hover(doc, pointer),
        }
    }

    pub fn pointer_up(&mut self, doc: &Document, pointer: Pointer) -> Option<Command> {
        self.pointer_move(doc, pointer);
        let command = match self.gesture.as_ref()? {
            Gesture::Shape { kind, .. } => {
                let kind = *kind;
                // Like Miro, a shape tool is one-shot: go back to selecting afterwards.
                self.active = ToolKind::Select;
                let (bounds, _) = self.shape_bounds(doc, true);
                Some(Command::Create(vec![shape(kind.into(), bounds).into()]))
            }
            Gesture::Ink {
                marker,
                points,
                zoom,
            } => ink::stroke(points, self.ink_style(*marker), *marker, *zoom, true)
                .map(|data| Command::Create(vec![data.into()])),
            Gesture::Erase { erased, .. } => {
                (!erased.is_empty()).then(|| Command::Delete(erased.clone()))
            }
            Gesture::Move {
                dragging: true,
                duplicate: true,
                ..
            } => Some(Command::Create(self.moved_copies(doc))),
            Gesture::Move { dragging: true, .. } => {
                let ids = self.unlocked(doc);
                (!ids.is_empty()).then(|| Command::Move {
                    ids,
                    delta: self.move_delta(doc).0,
                })
            }
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
                dragging: true,
                duplicate: false,
                ..
            }) => {
                let delta = self.move_delta(doc).0;
                self.unlocked(doc)
                    .into_iter()
                    .flat_map(|id| doc.leaves(id))
                    .map(|o| {
                        let mut o = o.clone();
                        o.data.bounds = o.data.bounds + delta;
                        o
                    })
                    .collect()
            }
            Some(Gesture::Resize { .. } | Gesture::Rotate { .. }) => self
                .geometry_changes(doc)
                .into_iter()
                .filter_map(|(id, frame)| {
                    let mut o = doc.get(id)?.clone();
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
            Some(Gesture::Shape { kind, .. }) => {
                vec![shape((*kind).into(), self.shape_bounds(doc, false).0)]
            }
            Some(Gesture::Ink {
                marker,
                points,
                zoom,
            }) => ink::stroke(points, self.ink_style(*marker), *marker, *zoom, false)
                .into_iter()
                .collect(),
            Some(Gesture::Move {
                dragging: true,
                duplicate: true,
                ..
            }) => self
                .moved_copies(doc)
                .iter()
                .flat_map(Content::shapes)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Smart guides of the gesture in progress, in world space.
    pub fn guides(&self, doc: &Document) -> Vec<Line> {
        match &self.gesture {
            Some(Gesture::Move { dragging: true, .. }) => self.move_delta(doc).1,
            Some(Gesture::Resize { .. }) => self.resize_target(doc).map_or(Vec::new(), |t| t.1),
            Some(Gesture::Shape { .. }) => self.shape_bounds(doc, false).1,
            _ => Vec::new(),
        }
    }

    /// Objects the gesture is about to delete (the eraser), hidden until it ends.
    pub fn hidden(&self) -> &[ObjectId] {
        match &self.gesture {
            Some(Gesture::Erase { erased, .. }) => erased,
            _ => &[],
        }
    }

    /// The eraser's outline: center and radius in world units.
    pub fn eraser(&self) -> Option<(Point, f64)> {
        self.eraser_at.filter(|_| self.active == ToolKind::Eraser)
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

    /// Resize and rotation handles are shown only when nothing else is going on, and never
    /// on locked objects.
    pub fn shows_handles(&self, doc: &Document) -> bool {
        self.active == ToolKind::Select
            && !self.selection.has_locked(doc)
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

    /// All objects making up the selection, groups expanded.
    fn selected<'a>(&'a self, doc: &'a Document) -> impl Iterator<Item = &'a Object> + 'a {
        self.selection.ids().iter().flat_map(|&id| doc.leaves(id))
    }

    /// The top-level object or group under the pointer.
    pub fn object_at(&self, doc: &Document, pointer: Pointer) -> Option<ObjectId> {
        hit::object_at(doc, pointer.world, HIT_SLOP / pointer.zoom)
    }

    /// The selected objects and groups that are not locked.
    fn unlocked(&self, doc: &Document) -> Vec<ObjectId> {
        let ids = self.selection.ids().iter().copied();
        ids.filter(|&id| !doc.is_locked(id)).collect()
    }

    /// A single object (not a group) is selected: handles follow its own rotation.
    fn single_object(&self, doc: &Document) -> bool {
        matches!(self.selection.ids(), [id] if doc.get(*id).is_some())
    }

    /// The selection-frame grip under the pointer, with the frame it belongs to.
    fn grip_at(&self, doc: &Document, pointer: Pointer) -> Option<(Frame, Grip)> {
        if self.selection.has_locked(doc) {
            return None;
        }
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
            None => hit::object_at(doc, pointer.world, HIT_SLOP / pointer.zoom),
        };
        let grip_kind = grip.map(|(_, g)| g);
        let changed = hover != self.hover || grip_kind != self.hover_grip;
        self.hover = hover;
        self.hover_grip = grip_kind;
        self.hover_rotation = grip.map_or(0.0, |(f, _)| f.rotation);
        changed
    }

    fn moved_copies(&self, doc: &Document) -> Vec<Content> {
        if !matches!(self.gesture, Some(Gesture::Move { .. })) {
            return Vec::new();
        }
        let items = clipboard::in_z_order(doc, &self.selection);
        clipboard::offset(&items, self.move_delta(doc).0)
    }

    /// What to snap to: visible top-level objects except `exclude`, and the grid if on.
    /// None while Ctrl is held.
    fn snapper(&self, doc: &Document, pointer: &Pointer, exclude: &[ObjectId]) -> Option<Snapper> {
        if pointer.ctrl {
            return None;
        }
        // TODO: query an R-tree for the visible objects.
        let targets = doc
            .roots()
            .iter()
            .filter(|id| !exclude.contains(id))
            .filter_map(|&id| unit_frame(doc, id, |o| o.data.frame()))
            .map(|f| f.aabb())
            .filter(|r| r.overlaps(pointer.visible))
            .collect();
        Some(Snapper {
            targets,
            tolerance: SNAP_DISTANCE / pointer.zoom,
            grid: self
                .snap_to_grid
                .then(|| GridLevel::for_zoom(pointer.zoom).snap_spacing()),
        })
    }

    /// Offset of the move gesture in progress after snapping, with its guides.
    fn move_delta(&self, doc: &Document) -> (Vec2, Vec<Line>) {
        let Some(Gesture::Move {
            start,
            current,
            duplicate,
            ..
        }) = &self.gesture
        else {
            return (Vec2::ZERO, Vec::new());
        };
        let raw = current.world - *start;
        let ids = if *duplicate {
            self.selection.ids().to_vec()
        } else {
            self.unlocked(doc)
        };
        // Copies leave the originals in place, so those are targets too.
        let exclude: &[ObjectId] = if *duplicate { &[] } else { &ids };
        let snapper = self.snapper(doc, current, exclude);
        let frame = selection_frame(doc, &ids, |o| o.data.frame());
        let (Some(snapper), Some(frame)) = (snapper, frame) else {
            return (raw, Vec::new());
        };
        let bounds = frame.aabb();
        let dragged = bounds + raw;
        let delta = raw + snapper.offset(&xs_of(dragged), &ys_of(dragged));
        let moved = bounds + delta;
        (delta, snapper.guides(moved, &xs_of(moved), &ys_of(moved)))
    }

    /// Frame-local bounds the resize gesture in progress leads to, with snapping guides.
    ///
    /// Only axis-aligned frames snap, and not while a corner keeps the aspect ratio: with
    /// Shift held, or by default when images are selected (then Shift stretches them).
    fn resize_target(&self, doc: &Document) -> Option<(Rect, Vec<Line>)> {
        let Some(Gesture::Resize {
            handle,
            from,
            start,
            current,
        }) = &self.gesture
        else {
            return None;
        };
        // Resize happens in the frame's own (unrotated) space.
        let mut delta = from.vec_to_local(current.world - *start);
        let has_image = self.selection.ids().iter().any(|&id| {
            doc.leaves(id)
                .iter()
                .any(|o| matches!(o.data.kind, ObjectKind::Image(_)))
        });
        let keep_aspect = current.shift != has_image;
        let snapper = (from.rotation == 0.0 && !(keep_aspect && handle.is_corner()))
            .then(|| self.snapper(doc, current, self.selection.ids()))
            .flatten();
        let Some(snapper) = snapper else {
            return Some((
                handles::resize(from.rect, *handle, delta, keep_aspect),
                Vec::new(),
            ));
        };
        let (x, y) = handle.dragged_edges(handles::resize(from.rect, *handle, delta, keep_aspect));
        delta += snapper.offset(&Vec::from_iter(x), &Vec::from_iter(y));
        let to = handles::resize(from.rect, *handle, delta, keep_aspect);
        let (x, y) = handle.dragged_edges(to);
        let guides = snapper.guides(to.abs(), &Vec::from_iter(x), &Vec::from_iter(y));
        Some((to, guides))
    }

    /// `pointer.world` snapped to objects and the grid, with guides through it.
    fn snap_point(&self, doc: &Document, pointer: Pointer) -> (Point, Vec<Line>) {
        let p = pointer.world;
        let Some(snapper) = self.snapper(doc, &pointer, &[]) else {
            return (p, Vec::new());
        };
        let p = p + snapper.offset(&[p.x], &[p.y]);
        (p, snapper.guides(Rect::from_points(p, p), &[p.x], &[p.y]))
    }

    /// Bounds of the shape being drawn, with guides through its snapped corner. On release
    /// (`finished`), a click instead of a drag makes a default-sized shape centered on the
    /// click point.
    fn shape_bounds(&self, doc: &Document, finished: bool) -> (Rect, Vec<Line>) {
        let Some(Gesture::Shape {
            start,
            anchor,
            current,
            ..
        }) = &self.gesture
        else {
            return (Rect::ZERO, Vec::new());
        };
        if finished && current.screen_distance(*start) < CLICK_SLOP {
            let size = (DEFAULT_SHAPE_SIZE, DEFAULT_SHAPE_SIZE);
            return (Rect::from_center_size(*anchor, size), Vec::new());
        }
        let (end, guides) = self.snap_point(doc, *current);
        (shape_rect(*anchor, end, current.shift), guides)
    }

    /// New frames of the selected objects for the resize or rotate gesture in progress.
    fn geometry_changes(&self, doc: &Document) -> Vec<(ObjectId, Frame)> {
        match &self.gesture {
            Some(Gesture::Resize { from, .. }) => {
                let Some((to, _)) = self.resize_target(doc) else {
                    return Vec::new();
                };
                if self.single_object(doc) {
                    let center = from.to_world(to.center());
                    let rect = Rect::from_center_size(center, to.size()).abs();
                    self.selected(doc)
                        .map(|o| (o.id, Frame::new(rect, from.rotation)))
                        .collect()
                } else {
                    // A group or multi-selection frame is axis-aligned; children keep their
                    // rotation.
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
                    let base = if self.single_object(doc) {
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

/// Rectangle from `anchor` to `end`; `square` (Shift) makes it a square.
fn shape_rect(anchor: Point, end: Point, square: bool) -> Rect {
    let mut delta = end - anchor;
    if square {
        let side = delta.x.abs().max(delta.y.abs());
        delta = Vec2::new(side.copysign(delta.x), side.copysign(delta.y));
    }
    Rect::from_points(anchor, anchor + delta)
}

#[cfg(test)]
mod tests;
