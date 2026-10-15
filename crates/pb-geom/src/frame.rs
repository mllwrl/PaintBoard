use kurbo::{Affine, Point, Rect, Vec2};

/// A rectangle rotated by `rotation` radians (clockwise on screen, since y points down)
/// around its own center.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub rect: Rect,
    pub rotation: f64,
}

impl Frame {
    pub fn new(rect: Rect, rotation: f64) -> Self {
        Self { rect, rotation }
    }

    pub fn axis_aligned(rect: Rect) -> Self {
        Self::new(rect, 0.0)
    }

    pub fn center(&self) -> Point {
        self.rect.center()
    }

    /// Local (unrotated) space to world space.
    pub fn transform(&self) -> Affine {
        Affine::rotate_about(self.rotation, self.center())
    }

    pub fn to_world(&self, local: Point) -> Point {
        self.transform() * local
    }

    pub fn to_local(&self, world: Point) -> Point {
        Affine::rotate_about(-self.rotation, self.center()) * world
    }

    /// Rotates a world-space direction into local space.
    pub fn vec_to_local(&self, v: Vec2) -> Vec2 {
        rotate(v, -self.rotation)
    }

    /// Corners in world space: top-left, top-right, bottom-right, bottom-left (local order).
    pub fn corners(&self) -> [Point; 4] {
        let r = self.rect;
        [
            Point::new(r.x0, r.y0),
            Point::new(r.x1, r.y0),
            Point::new(r.x1, r.y1),
            Point::new(r.x0, r.y1),
        ]
        .map(|p| self.to_world(p))
    }

    /// Axis-aligned bounding box in world space.
    pub fn aabb(&self) -> Rect {
        let [a, b, c, d] = self.corners();
        Rect::from_points(a, a).union_pt(b).union_pt(c).union_pt(d)
    }

    pub fn contains(&self, world: Point, pad: f64) -> bool {
        self.rect.inflate(pad, pad).contains(self.to_local(world))
    }

    /// Whether this frame touches the axis-aligned `area` (separating axis test).
    pub fn overlaps(&self, area: Rect) -> bool {
        if !self.aabb().overlaps(area) {
            return false;
        }
        if self.rotation == 0.0 {
            return true;
        }
        // The world axes are covered by the AABB check; test the frame's own two axes.
        let local = Frame::new(area, 0.0).corners().map(|p| self.to_local(p));
        let (mut min, mut max) = (local[0], local[0]);
        for p in &local[1..] {
            min = Point::new(min.x.min(p.x), min.y.min(p.y));
            max = Point::new(max.x.max(p.x), max.y.max(p.y));
        }
        Rect::from_points(min, max).overlaps(self.rect)
    }

    /// The frame after rotating it by `angle` around `pivot`.
    pub fn rotated_about(&self, pivot: Point, angle: f64) -> Frame {
        let center = Affine::rotate_about(angle, pivot) * self.center();
        Frame::new(
            Rect::from_center_size(center, self.rect.size()),
            normalize_angle(self.rotation + angle),
        )
    }
}

pub fn rotate(v: Vec2, angle: f64) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

/// Maps an angle into `(-PI, PI]`.
pub fn normalize_angle(a: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    let a = a.rem_euclid(TAU);
    if a > PI { a - TAU } else { a }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_2;
    use std::f64::consts::FRAC_PI_4;

    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a - b).hypot() < 1e-9
    }

    #[test]
    fn quarter_turn_swaps_extent() {
        let f = Frame::new(Rect::new(0.0, 0.0, 100.0, 20.0), FRAC_PI_2);
        let aabb = f.aabb();
        assert!((aabb.width() - 20.0).abs() < 1e-9);
        assert!((aabb.height() - 100.0).abs() < 1e-9);
        assert!(close(aabb.center(), Point::new(50.0, 10.0)));
    }

    #[test]
    fn local_world_roundtrip() {
        let f = Frame::new(Rect::new(-5.0, 3.0, 40.0, 70.0), 0.7);
        let p = Point::new(12.0, -8.0);
        assert!(close(f.to_world(f.to_local(p)), p));
    }

    #[test]
    fn contains_respects_rotation() {
        // A thin bar rotated 90 degrees is vertical.
        let f = Frame::new(Rect::new(0.0, 0.0, 100.0, 20.0), FRAC_PI_2);
        assert!(f.contains(Point::new(50.0, 50.0), 0.0));
        assert!(!f.contains(Point::new(5.0, 10.0), 0.0));
    }

    #[test]
    fn overlap_uses_real_shape_not_aabb() {
        // A diamond: its AABB corner region is empty.
        let f = Frame::new(Rect::new(0.0, 0.0, 100.0, 100.0), FRAC_PI_4);
        let corner = Rect::new(-15.0, -15.0, -5.0, -5.0);
        assert!(f.aabb().overlaps(corner));
        assert!(!f.overlaps(corner));
        assert!(f.overlaps(Rect::new(45.0, 45.0, 55.0, 55.0)));
    }

    #[test]
    fn rotating_about_pivot_moves_center() {
        let f = Frame::axis_aligned(Rect::new(10.0, -5.0, 30.0, 5.0));
        let r = f.rotated_about(Point::ZERO, FRAC_PI_2);
        assert!(close(r.center(), Point::new(0.0, 20.0)));
        assert!((r.rotation - FRAC_PI_2).abs() < 1e-9);
        assert_eq!(r.rect.size(), f.rect.size());
    }

    #[test]
    fn angles_normalize() {
        use std::f64::consts::PI;
        assert!((normalize_angle(3.0 * PI) - PI).abs() < 1e-9);
        assert!((normalize_angle(-FRAC_PI_2) + FRAC_PI_2).abs() < 1e-9);
    }
}
