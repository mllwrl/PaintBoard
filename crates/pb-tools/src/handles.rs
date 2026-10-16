use std::f64::consts::FRAC_PI_4;

use pb_geom::kurbo::{Point, Rect, Vec2};
use pb_geom::{Frame, rotate};

use crate::CursorHint;

/// Objects never get smaller than this (world units) when resized.
const MIN_SIZE: f64 = 1.0;
/// Distance of the rotation knob above the selection frame, in logical pixels.
const KNOB_OFFSET: f64 = 24.0;

/// A resize handle on the selection bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

/// Which edge of the bounds a handle drags along one axis.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Min,
    Max,
    Fixed,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Self::NorthWest,
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
    ];

    fn edges(self) -> (Edge, Edge) {
        use Edge::*;
        match self {
            Self::NorthWest => (Min, Min),
            Self::North => (Fixed, Min),
            Self::NorthEast => (Max, Min),
            Self::East => (Max, Fixed),
            Self::SouthEast => (Max, Max),
            Self::South => (Fixed, Max),
            Self::SouthWest => (Min, Max),
            Self::West => (Min, Fixed),
        }
    }

    pub fn is_corner(self) -> bool {
        let (x, y) = self.edges();
        x != Edge::Fixed && y != Edge::Fixed
    }

    /// Coordinates of the edges this handle drags in `bounds`: x of the vertical edge and y
    /// of the horizontal one, where the handle moves them.
    pub fn dragged_edges(self, bounds: Rect) -> (Option<f64>, Option<f64>) {
        let pick = |edge, min, max| match edge {
            Edge::Min => Some(min),
            Edge::Max => Some(max),
            Edge::Fixed => None,
        };
        let (x, y) = self.edges();
        (pick(x, bounds.x0, bounds.x1), pick(y, bounds.y0, bounds.y1))
    }

    pub fn position(self, bounds: Rect) -> Point {
        let pick = |edge, min, max| match edge {
            Edge::Min => min,
            Edge::Max => max,
            Edge::Fixed => (min + max) / 2.0,
        };
        let (x, y) = self.edges();
        Point::new(pick(x, bounds.x0, bounds.x1), pick(y, bounds.y0, bounds.y1))
    }

    /// Resize cursor for this handle on a frame rotated by `rotation`.
    pub fn cursor(self, rotation: f64) -> CursorHint {
        let sign = |edge| match edge {
            Edge::Min => -1.0,
            Edge::Max => 1.0,
            Edge::Fixed => 0.0,
        };
        let (x, y) = self.edges();
        let direction = rotate(Vec2::new(sign(x), sign(y)), rotation);
        // Cursors are symmetric, so only the direction modulo 180 degrees matters.
        let sector = (direction.atan2().rem_euclid(std::f64::consts::PI) / FRAC_PI_4).round();
        match sector as u8 % 4 {
            0 => CursorHint::ResizeEw,
            1 => CursorHint::ResizeNwse,
            2 => CursorHint::ResizeNs,
            _ => CursorHint::ResizeNesw,
        }
    }
}

/// World position of the rotation knob: above the top edge, a fixed screen distance away.
pub fn rotation_knob(frame: Frame, zoom: f64) -> Point {
    let top = frame.to_world(Handle::North.position(frame.rect));
    top + rotate(Vec2::new(0.0, -KNOB_OFFSET / zoom), frame.rotation)
}

/// The handle within `radius` of `point`; corners win over edges.
pub fn handle_at(frame: Frame, point: Point, radius: f64) -> Option<Handle> {
    // Rotation preserves distances, so compare in the frame's local space.
    let local = frame.to_local(point);
    let near = |h: &Handle| (h.position(frame.rect) - local).hypot() <= radius;
    let (corners, edges): (Vec<_>, Vec<_>) = Handle::ALL.iter().partition(|h| h.is_corner());
    corners.into_iter().chain(edges).find(near)
}

/// Bounds after dragging `handle` by `delta`. The result may be flipped (`x0 > x1`) when a
/// handle is dragged past the opposite edge; [`map_rect`] turns that into mirroring.
pub fn resize(bounds: Rect, handle: Handle, delta: Vec2, keep_aspect: bool) -> Rect {
    let (ex, ey) = handle.edges();
    let mut r = bounds;
    match ex {
        Edge::Min => r.x0 += delta.x,
        Edge::Max => r.x1 += delta.x,
        Edge::Fixed => {}
    }
    match ey {
        Edge::Min => r.y0 += delta.y,
        Edge::Max => r.y1 += delta.y,
        Edge::Fixed => {}
    }

    let (w0, h0) = (bounds.width(), bounds.height());
    if keep_aspect && handle.is_corner() && w0 > 0.0 && h0 > 0.0 {
        let scale = (r.width().abs() / w0).max(r.height().abs() / h0);
        let w = (w0 * scale).copysign(r.width());
        let h = (h0 * scale).copysign(r.height());
        // Keep the corner opposite to the handle where it was.
        if ex == Edge::Min {
            r.x0 = r.x1 - w;
        } else {
            r.x1 = r.x0 + w;
        }
        if ey == Edge::Min {
            r.y0 = r.y1 - h;
        } else {
            r.y1 = r.y0 + h;
        }
    }

    // Enforce a minimum size by moving the dragged edge, never the fixed one.
    if r.width().abs() < MIN_SIZE {
        let w = MIN_SIZE.copysign(r.width());
        if ex == Edge::Min {
            r.x0 = r.x1 - w;
        } else {
            r.x1 = r.x0 + w;
        }
    }
    if r.height().abs() < MIN_SIZE {
        let h = MIN_SIZE.copysign(r.height());
        if ey == Edge::Min {
            r.y0 = r.y1 - h;
        } else {
            r.y1 = r.y0 + h;
        }
    }
    r
}

/// Maps `rect` from the `from` frame into the (possibly flipped) `to` frame, normalized.
pub fn map_rect(rect: Rect, from: Rect, to: Rect) -> Rect {
    let axis = |v: f64, f0: f64, f1: f64, t0: f64, t1: f64| {
        if f1 - f0 == 0.0 {
            t0 + (v - f0)
        } else {
            t0 + (v - f0) * (t1 - t0) / (f1 - f0)
        }
    };
    let map = |p: Point| {
        Point::new(
            axis(p.x, from.x0, from.x1, to.x0, to.x1),
            axis(p.y, from.y0, from.y1, to.y0, to.y1),
        )
    };
    Rect::from_points(map(rect.origin()), map(Point::new(rect.x1, rect.y1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect::new(0.0, 0.0, 100.0, 50.0);

    #[test]
    fn corners_win_over_edges() {
        let f = Frame::axis_aligned(R);
        assert_eq!(
            handle_at(f, Point::new(1.0, 1.0), 5.0),
            Some(Handle::NorthWest)
        );
        assert_eq!(
            handle_at(f, Point::new(50.0, 49.0), 5.0),
            Some(Handle::South)
        );
        assert_eq!(handle_at(f, Point::new(50.0, 25.0), 5.0), None);
    }

    #[test]
    fn handles_follow_rotation() {
        use std::f64::consts::FRAC_PI_2;
        // Rotated a quarter turn clockwise, the east handle points down.
        let f = Frame::new(R, FRAC_PI_2);
        let east = f.to_world(Handle::East.position(R));
        assert!((east - Point::new(50.0, 75.0)).hypot() < 1e-9);
        assert_eq!(handle_at(f, east, 1.0), Some(Handle::East));
        assert_eq!(Handle::East.cursor(FRAC_PI_2), CursorHint::ResizeNs);
        assert_eq!(Handle::East.cursor(0.0), CursorHint::ResizeEw);
        assert_eq!(Handle::SouthEast.cursor(FRAC_PI_2), CursorHint::ResizeNesw);
    }

    #[test]
    fn knob_sits_above_top_edge_at_any_zoom() {
        let f = Frame::axis_aligned(R);
        assert_eq!(rotation_knob(f, 1.0), Point::new(50.0, -KNOB_OFFSET));
        assert_eq!(rotation_knob(f, 2.0), Point::new(50.0, -KNOB_OFFSET / 2.0));
    }

    #[test]
    fn edge_handle_moves_one_side() {
        let r = resize(R, Handle::East, Vec2::new(20.0, 999.0), false);
        assert_eq!(r, Rect::new(0.0, 0.0, 120.0, 50.0));
        let r = resize(R, Handle::North, Vec2::new(999.0, 10.0), false);
        assert_eq!(r, Rect::new(0.0, 10.0, 100.0, 50.0));
    }

    #[test]
    fn corner_keeps_aspect_with_shift() {
        let r = resize(R, Handle::SouthEast, Vec2::new(100.0, 0.0), true);
        assert_eq!(r, Rect::new(0.0, 0.0, 200.0, 100.0));
        let r = resize(R, Handle::NorthWest, Vec2::new(-100.0, 0.0), true);
        assert_eq!(r, Rect::new(-100.0, -50.0, 100.0, 50.0));
    }

    #[test]
    fn dragging_past_opposite_edge_mirrors() {
        let to = resize(R, Handle::East, Vec2::new(-150.0, 0.0), false);
        assert_eq!(to.x1, -50.0);
        let child = Rect::new(0.0, 0.0, 10.0, 10.0);
        // The child at the left edge ends up at the right edge of the mirrored frame.
        assert_eq!(map_rect(child, R, to), Rect::new(-5.0, 0.0, 0.0, 10.0));
    }

    #[test]
    fn min_size_is_enforced() {
        let r = resize(R, Handle::East, Vec2::new(-100.0, 0.0), false);
        assert_eq!(r.width(), MIN_SIZE);
        assert_eq!(r.x0, 0.0);
    }

    #[test]
    fn map_rect_scales_children() {
        let to = Rect::new(0.0, 0.0, 200.0, 100.0);
        let child = Rect::new(50.0, 25.0, 100.0, 50.0);
        assert_eq!(map_rect(child, R, to), Rect::new(100.0, 50.0, 200.0, 100.0));
    }
}
