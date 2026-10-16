//! Geometry of freehand strokes: simplification, smoothing, outlines and hit-testing.

use kurbo::{BezPath, Circle, Line, ParamCurveNearest, Point, Shape};

/// Indices of the points to keep so that the polyline stays within `tolerance` of the
/// original (Ramer–Douglas–Peucker). The first and last points are always kept.
pub fn simplify(points: &[Point], tolerance: f64) -> Vec<usize> {
    if points.len() <= 2 {
        return (0..points.len()).collect();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        let chord = Line::new(points[first], points[last]);
        let (mut far, mut far_distance) = (first, 0.0);
        for (i, &p) in points.iter().enumerate().take(last).skip(first + 1) {
            let d = chord.nearest(p, 1e-6).distance_sq.sqrt();
            if d > far_distance {
                (far, far_distance) = (i, d);
            }
        }
        if far_distance > tolerance {
            keep[far] = true;
            stack.push((first, far));
            stack.push((far, last));
        }
    }
    (0..points.len()).filter(|&i| keep[i]).collect()
}

/// A smooth curve through a stroke's points: quadratic segments between the midpoints of
/// consecutive points, using the points themselves as control points.
pub fn smooth_path(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    let Some((&first, rest)) = points.split_first() else {
        return path;
    };
    path.move_to(first);
    match rest {
        [] => path.line_to(first),
        [only] => path.line_to(*only),
        _ => {
            for pair in rest.windows(2) {
                path.quad_to(pair[0], pair[0].midpoint(pair[1]));
            }
            path.line_to(*rest.last().expect("rest has two or more points"));
        }
    }
    path
}

/// Filled outline of a stroke whose half-width (`radius`) varies along it, e.g. with pen
/// pressure. Round at both ends.
pub fn variable_outline(points: &[(Point, f64)]) -> BezPath {
    match points {
        [] => return BezPath::new(),
        [(p, r)] => return Circle::new(*p, *r).to_path(0.1),
        _ => {}
    }
    let n = points.len();
    let normal = |i: usize| {
        let prev = points[i.saturating_sub(1)].0;
        let next = points[(i + 1).min(n - 1)].0;
        let d = next - prev;
        let len = d.hypot();
        if len == 0.0 {
            kurbo::Vec2::new(0.0, 1.0)
        } else {
            kurbo::Vec2::new(-d.y, d.x) / len
        }
    };
    let left: Vec<_> = (0..n)
        .map(|i| points[i].0 + normal(i) * points[i].1)
        .collect();
    let right: Vec<_> = (0..n)
        .map(|i| points[i].0 - normal(i) * points[i].1)
        .collect();

    let mut path = smooth_path(&left);
    // Round cap at the end: half circle from the left side to the right side.
    cap(&mut path, points[n - 1], normal(n - 1));
    let back: Vec<_> = right.iter().rev().copied().collect();
    let tail = smooth_path(&back);
    // Continue along the right side (skip its move_to, the cap ended there).
    for el in tail.elements().iter().skip(1) {
        path.push(*el);
    }
    cap(&mut path, points[0], -normal(0));
    path.close_path();
    path
}

/// Half circle around `center` from `center + normal * r` to `center - normal * r`, bulging
/// forward (to the right of the normal).
fn cap(path: &mut BezPath, (center, r): (Point, f64), normal: kurbo::Vec2) {
    let forward = kurbo::Vec2::new(normal.y, -normal.x);
    // Two quarter circles as cubics (k = 4/3 * tan(pi/8)).
    const K: f64 = 0.552_284_749_8;
    let a = center + normal * r;
    let tip = center + forward * r;
    let b = center - normal * r;
    path.curve_to(a + forward * (r * K), tip + normal * (r * K), tip);
    path.curve_to(tip - normal * (r * K), b + forward * (r * K), b);
}

/// Distance from `p` to the polyline through `points`.
pub fn polyline_distance(points: &[Point], p: Point) -> f64 {
    match points {
        [] => f64::INFINITY,
        [only] => (*only - p).hypot(),
        _ => points
            .windows(2)
            .map(|w| Line::new(w[0], w[1]).nearest(p, 1e-6).distance_sq)
            .fold(f64::INFINITY, f64::min)
            .sqrt(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_drops_collinear_points() {
        let points: Vec<_> = (0..10).map(|i| Point::new(i as f64, 0.0)).collect();
        assert_eq!(simplify(&points, 0.1), [0, 9]);
        let corner = [
            Point::new(0.0, 0.0),
            Point::new(5.0, 0.1),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
        ];
        assert_eq!(simplify(&corner, 0.5), [0, 2, 3]);
    }

    #[test]
    fn smooth_path_starts_and_ends_on_the_points() {
        let points = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
        ];
        let path = smooth_path(&points);
        let bbox = path.bounding_box();
        assert!(bbox.x0 >= 0.0 && bbox.x1 <= 10.0 && bbox.y1 <= 10.0);
        assert_eq!(
            path.elements().first(),
            Some(&kurbo::PathEl::MoveTo(points[0]))
        );
        assert_eq!(
            path.elements().last(),
            Some(&kurbo::PathEl::LineTo(points[2]))
        );
    }

    #[test]
    fn variable_outline_covers_the_stroke() {
        let points = [
            (Point::new(0.0, 0.0), 2.0),
            (Point::new(50.0, 0.0), 6.0),
            (Point::new(100.0, 0.0), 2.0),
        ];
        let outline = variable_outline(&points);
        // Smoothing rounds off the widest point a little.
        assert!(outline.contains(Point::new(50.0, 4.0)));
        assert!(!outline.contains(Point::new(50.0, 7.0)));
        // Round caps reach past the end points.
        assert!(outline.contains(Point::new(101.5, 0.0)));
        assert!(outline.contains(Point::new(-1.5, 0.0)));
        let dot = variable_outline(&[(Point::new(5.0, 5.0), 3.0)]);
        assert!(dot.contains(Point::new(7.0, 5.0)));
    }

    #[test]
    fn distance_to_polyline() {
        let points = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
        ];
        assert!((polyline_distance(&points, Point::new(5.0, 3.0)) - 3.0).abs() < 1e-9);
        assert!((polyline_distance(&points, Point::new(13.0, 5.0)) - 3.0).abs() < 1e-9);
        assert!((polyline_distance(&points[..1], Point::new(3.0, 4.0)) - 5.0).abs() < 1e-9);
    }
}
