use pb_geom::kurbo::{Point, Rect, Vec2};

use crate::CursorHint;

/// Objects never get smaller than this (world units) when resized.
const MIN_SIZE: f64 = 1.0;

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

    pub fn position(self, bounds: Rect) -> Point {
        let pick = |edge, min, max| match edge {
            Edge::Min => min,
            Edge::Max => max,
            Edge::Fixed => (min + max) / 2.0,
        };
        let (x, y) = self.edges();
        Point::new(pick(x, bounds.x0, bounds.x1), pick(y, bounds.y0, bounds.y1))
    }

    pub fn cursor(self) -> CursorHint {
        match self {
            Self::North | Self::South => CursorHint::ResizeNs,
            Self::East | Self::West => CursorHint::ResizeEw,
            Self::NorthWest | Self::SouthEast => CursorHint::ResizeNwse,
            Self::NorthEast | Self::SouthWest => CursorHint::ResizeNesw,
        }
    }
}

/// The handle within `radius` of `point`; corners win over edges.
pub fn handle_at(bounds: Rect, point: Point, radius: f64) -> Option<Handle> {
    let near = |h: &Handle| (h.position(bounds) - point).hypot() <= radius;
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
        assert_eq!(
            handle_at(R, Point::new(1.0, 1.0), 5.0),
            Some(Handle::NorthWest)
        );
        assert_eq!(
            handle_at(R, Point::new(50.0, 49.0), 5.0),
            Some(Handle::South)
        );
        assert_eq!(handle_at(R, Point::new(50.0, 25.0), 5.0), None);
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
