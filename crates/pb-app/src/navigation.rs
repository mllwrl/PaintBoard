//! Canvas navigation: pan and zoom from mouse, touchpad and keyboard.

use pb_geom::Camera;
use pb_geom::kurbo::{Point, Size, Vec2};
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::keyboard::ModifiersState;
use winit::window::CursorIcon;

/// Zoom factor for one mouse wheel notch.
const WHEEL_ZOOM_STEP: f64 = 1.15;
/// Zoom factor for the keyboard zoom shortcuts.
const KEY_ZOOM_STEP: f64 = 1.25;
/// Pixels of pan per mouse wheel notch.
const WHEEL_PAN_STEP: f64 = 60.0;

/// Tracks pointer state and turns input into camera moves. All positions are logical pixels.
#[derive(Default)]
pub struct Navigation {
    /// Last pointer position. Kept after the pointer leaves the window, so a drag released
    /// outside still ends where the pointer was last seen.
    cursor: Option<Point>,
    cursor_inside: bool,
    pan_button: Option<MouseButton>,
    space_down: bool,
    modifiers: ModifiersState,
}

impl Navigation {
    pub fn set_modifiers(&mut self, modifiers: ModifiersState) {
        self.modifiers = modifiers;
    }

    pub fn modifiers(&self) -> ModifiersState {
        self.modifiers
    }

    /// Last known pointer position, in logical pixels.
    pub fn cursor(&self) -> Option<Point> {
        self.cursor
    }

    /// Pointer position, only while the pointer is over the window.
    pub fn hovered_cursor(&self) -> Option<Point> {
        self.cursor.filter(|_| self.cursor_inside)
    }

    /// Returns true if the camera moved.
    pub fn cursor_moved(&mut self, camera: &mut Camera, pos: Point) -> bool {
        self.cursor_inside = true;
        let previous = self.cursor.replace(pos);
        match (self.pan_button, previous) {
            (Some(_), Some(prev)) => {
                camera.pan(pos - prev);
                true
            }
            _ => false,
        }
    }

    pub fn cursor_left(&mut self) {
        self.cursor_inside = false;
    }

    /// Returns true if the event was consumed by navigation.
    pub fn mouse_button(&mut self, button: MouseButton, state: ElementState) -> bool {
        match state {
            ElementState::Pressed => {
                let starts_pan = button == MouseButton::Middle
                    || (button == MouseButton::Left && self.space_down);
                if starts_pan && self.pan_button.is_none() {
                    self.pan_button = Some(button);
                }
                starts_pan
            }
            ElementState::Released => {
                if self.pan_button == Some(button) {
                    self.pan_button = None;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn set_space(&mut self, down: bool) {
        self.space_down = down;
    }

    /// Mouse wheels and touchpads scroll the board (Shift+wheel sideways); with Ctrl they
    /// zoom.
    ///
    /// On Windows touchpads also report line deltas, so two-finger scroll zooms there.
    /// TODO: a "mouse / touchpad" navigation setting, like Miro has.
    pub fn wheel(&mut self, camera: &mut Camera, delta: MouseScrollDelta, scale: f64) {
        let anchor = self.cursor.unwrap_or_default();
        match delta {
            // A mouse wheel scrolls like in a document (Shift: sideways); Ctrl+wheel zooms.
            MouseScrollDelta::LineDelta(x, y) => {
                if self.modifiers.control_key() {
                    camera.zoom_at(anchor, WHEEL_ZOOM_STEP.powf(f64::from(y)));
                } else if self.modifiers.shift_key() {
                    let notches = if x != 0.0 { x } else { y };
                    camera.pan(Vec2::new(f64::from(notches) * WHEEL_PAN_STEP, 0.0));
                } else {
                    let step = Vec2::new(f64::from(x), f64::from(y)) * WHEEL_PAN_STEP;
                    camera.pan(step);
                }
            }
            MouseScrollDelta::PixelDelta(pos) => {
                let delta = Vec2::new(pos.x, pos.y) / scale;
                if self.modifiers.control_key() {
                    camera.zoom_at(anchor, (delta.y * 0.01).exp());
                } else {
                    camera.pan(delta);
                }
            }
        }
    }

    pub fn pinch(&mut self, camera: &mut Camera, delta: f64) {
        camera.zoom_at(self.cursor.unwrap_or_default(), 1.0 + delta);
    }

    pub fn zoom_in(&self, camera: &mut Camera, viewport: Size) {
        camera.zoom_at(self.zoom_anchor(viewport), KEY_ZOOM_STEP);
    }

    pub fn zoom_out(&self, camera: &mut Camera, viewport: Size) {
        camera.zoom_at(self.zoom_anchor(viewport), 1.0 / KEY_ZOOM_STEP);
    }

    pub fn zoom_reset(&self, camera: &mut Camera, viewport: Size) {
        camera.set_zoom_at(self.zoom_anchor(viewport), 1.0);
    }

    /// Keyboard zoom centers on the cursor when it is over the canvas, otherwise on the viewport.
    fn zoom_anchor(&self, viewport: Size) -> Point {
        self.hovered_cursor()
            .unwrap_or_else(|| (viewport.to_vec2() / 2.0).to_point())
    }

    pub fn cursor_icon(&self) -> CursorIcon {
        if self.pan_button.is_some() {
            CursorIcon::Grabbing
        } else if self.space_down {
            CursorIcon::Grab
        } else {
            CursorIcon::Default
        }
    }
}
