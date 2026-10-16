//! Geometry: hit-testing, snapping, spatial index, connector routing.

mod camera;
mod frame;
mod grid;
pub mod ink;
mod shape;
mod snap;

pub use camera::Camera;
pub use frame::{Frame, normalize_angle, rotate};
pub use grid::GridLevel;
pub use kurbo;
pub use shape::ShapeKind;
pub use snap::{Snapper, xs_of, ys_of};
