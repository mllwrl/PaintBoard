//! Geometry: hit-testing, snapping, spatial index, connector routing.

mod camera;
mod frame;

pub use camera::Camera;
pub use frame::{Frame, normalize_angle, rotate};
pub use kurbo;
