//! Canvas tools: select, pen, shapes, connectors, ...
//!
//! Tools turn pointer input (in world coordinates) into [`Command`]s and previews. They never
//! touch the document directly, so the same input logic will drive local and remote edits.

use pb_core::{Command, ObjectData, ObjectKind, Style};
use pb_geom::kurbo::{Point, Rect, Vec2};

/// Drags shorter than this on screen (logical pixels) count as clicks.
const CLICK_SLOP: f64 = 4.0;
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

#[derive(Clone, Copy, Debug)]
pub struct Pointer {
    pub world: Point,
    /// Current camera zoom, to measure distances in screen pixels.
    pub zoom: f64,
    /// Constrain proportions (Shift).
    pub constrain: bool,
}

#[derive(Default)]
pub struct Tools {
    active: ToolKind,
    drag: Option<Drag>,
}

struct Drag {
    start: Point,
    current: Pointer,
}

impl Tools {
    pub fn active(&self) -> ToolKind {
        self.active
    }

    pub fn set_active(&mut self, tool: ToolKind) {
        self.active = tool;
        self.drag = None;
    }

    /// Abandons the current gesture. Returns true if there was one.
    pub fn cancel(&mut self) -> bool {
        self.drag.take().is_some()
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Returns true if the active tool took the press.
    pub fn pointer_down(&mut self, pointer: Pointer) -> bool {
        match self.active {
            ToolKind::Select => false,
            ToolKind::Rectangle => {
                self.drag = Some(Drag {
                    start: pointer.world,
                    current: pointer,
                });
                true
            }
        }
    }

    /// Returns true if the preview changed.
    pub fn pointer_move(&mut self, pointer: Pointer) -> bool {
        match &mut self.drag {
            Some(drag) => {
                drag.current = pointer;
                true
            }
            None => false,
        }
    }

    pub fn pointer_up(&mut self, pointer: Pointer) -> Option<Command> {
        let mut drag = self.drag.take()?;
        drag.current = pointer;
        match self.active {
            ToolKind::Select => None,
            ToolKind::Rectangle => {
                // Like Miro, a shape tool is one-shot: go back to selecting afterwards.
                self.active = ToolKind::Select;
                Some(Command::Create(shape(
                    ObjectKind::Rectangle,
                    drag.final_rect(),
                )))
            }
        }
    }

    /// What the gesture in progress would create, drawn on top of the board.
    pub fn preview(&self) -> Option<ObjectData> {
        let drag = self.drag.as_ref()?;
        match self.active {
            ToolKind::Select => None,
            ToolKind::Rectangle => Some(shape(ObjectKind::Rectangle, drag.drag_rect())),
        }
    }
}

impl Drag {
    fn is_click(&self) -> bool {
        (self.current.world - self.start).hypot() * self.current.zoom < CLICK_SLOP
    }

    fn drag_rect(&self) -> Rect {
        let mut delta = self.current.world - self.start;
        if self.current.constrain {
            let side = delta.x.abs().max(delta.y.abs());
            delta = Vec2::new(side.copysign(delta.x), side.copysign(delta.y));
        }
        Rect::from_points(self.start, self.start + delta)
    }

    /// A click creates a default-sized shape centered on the click point.
    fn final_rect(&self) -> Rect {
        if self.is_click() {
            Rect::from_center_size(self.start, (DEFAULT_SHAPE_SIZE, DEFAULT_SHAPE_SIZE))
        } else {
            self.drag_rect()
        }
    }
}

fn shape(kind: ObjectKind, bounds: Rect) -> ObjectData {
    ObjectData {
        kind,
        bounds,
        style: Style::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64) -> Pointer {
        Pointer {
            world: Point::new(x, y),
            zoom: 1.0,
            constrain: false,
        }
    }

    fn created_bounds(cmd: Option<Command>) -> Rect {
        match cmd {
            Some(Command::Create(data)) => data.bounds,
            other => panic!("expected Create, got {other:?}"),
        }
    }

    #[test]
    fn select_tool_ignores_presses() {
        let mut tools = Tools::default();
        assert!(!tools.pointer_down(at(0.0, 0.0)));
        assert!(tools.pointer_up(at(10.0, 10.0)).is_none());
    }

    #[test]
    fn drag_creates_rectangle_and_returns_to_select() {
        let mut tools = Tools::default();
        tools.set_active(ToolKind::Rectangle);
        tools.pointer_down(at(50.0, 50.0));
        tools.pointer_move(at(10.0, 80.0));
        assert_eq!(
            tools.preview().unwrap().bounds,
            Rect::new(10.0, 50.0, 50.0, 80.0)
        );
        let bounds = created_bounds(tools.pointer_up(at(10.0, 80.0)));
        assert_eq!(bounds, Rect::new(10.0, 50.0, 50.0, 80.0));
        assert_eq!(tools.active(), ToolKind::Select);
        assert!(tools.preview().is_none());
    }

    #[test]
    fn click_creates_default_size_centered() {
        let mut tools = Tools::default();
        tools.set_active(ToolKind::Rectangle);
        tools.pointer_down(at(0.0, 0.0));
        let bounds = created_bounds(tools.pointer_up(at(1.0, 1.0)));
        assert_eq!(bounds.center(), Point::ZERO);
        assert_eq!(bounds.width(), DEFAULT_SHAPE_SIZE);
    }

    #[test]
    fn click_slop_is_measured_on_screen() {
        let mut tools = Tools::default();
        tools.set_active(ToolKind::Rectangle);
        // 3 world units at 10x zoom are 30 screen pixels: a drag, not a click.
        tools.pointer_down(Pointer {
            zoom: 10.0,
            ..at(0.0, 0.0)
        });
        let bounds = created_bounds(tools.pointer_up(Pointer {
            zoom: 10.0,
            ..at(3.0, 3.0)
        }));
        assert_eq!(bounds, Rect::new(0.0, 0.0, 3.0, 3.0));
    }

    #[test]
    fn shift_constrains_to_square() {
        let mut tools = Tools::default();
        tools.set_active(ToolKind::Rectangle);
        tools.pointer_down(at(0.0, 0.0));
        let bounds = created_bounds(tools.pointer_up(Pointer {
            constrain: true,
            ..at(-40.0, 10.0)
        }));
        assert_eq!(bounds, Rect::new(-40.0, 0.0, 0.0, 40.0));
    }

    #[test]
    fn cancel_drops_gesture() {
        let mut tools = Tools::default();
        tools.set_active(ToolKind::Rectangle);
        tools.pointer_down(at(0.0, 0.0));
        assert!(tools.cancel());
        assert!(tools.pointer_up(at(50.0, 50.0)).is_none());
        assert_eq!(tools.active(), ToolKind::Rectangle);
    }
}
