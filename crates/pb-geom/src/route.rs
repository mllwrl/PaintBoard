//! Orthogonal routing around obstacles: A* over a sparse grid whose lines run along the
//! obstacles' edges, so a path hugs them at a fixed distance and bends as little as it can.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use kurbo::{Point, Rect, Vec2};

/// Space kept between a routed line and the obstacles it passes, in world units.
pub const CLEARANCE: f64 = 12.0;
/// What a bend costs, as extra length, so paths prefer fewer of them.
const BEND_COST: f64 = 40.0;
/// Grids bigger than this per side are not searched (too many obstacles around).
const MAX_LINES: usize = 160;

/// Whether the axis-aligned polyline `points` runs through any of `obstacles` (inflated
/// by `margin`). Segments only touching an edge do not count.
pub fn blocked(points: &[Point], obstacles: &[Rect], margin: f64) -> bool {
    points.windows(2).any(|w| {
        obstacles
            .iter()
            .any(|r| crosses(w[0], w[1], r.inflate(margin, margin)))
    })
}

/// Whether segment `a`-`b` (horizontal or vertical) enters the inside of `r`.
fn crosses(a: Point, b: Point, r: Rect) -> bool {
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    let eps = 1e-9;
    x1 > r.x0 + eps && x0 < r.x1 - eps && y1 > r.y0 + eps && y0 < r.y1 - eps
}

/// A path of horizontal and vertical segments from `from`, leaving along `out`, to `to`,
/// arriving along `into` (both unit axis vectors, or zero for any direction), that keeps
/// `CLEARANCE` away from `obstacles`. `None` if there is none on the grid.
pub fn route(
    from: Point,
    out: Vec2,
    to: Point,
    into: Vec2,
    obstacles: &[Rect],
) -> Option<Vec<Point>> {
    let walls: Vec<Rect> = obstacles
        .iter()
        .map(|r| r.inflate(CLEARANCE, CLEARANCE))
        .collect();
    // Grid lines: the ends, the walls' edges, and halfway between neighbors, so a path
    // can pass between two obstacles that are close together.
    let mut xs = vec![from.x, to.x];
    let mut ys = vec![from.y, to.y];
    for w in &walls {
        xs.extend([w.x0, w.x1]);
        ys.extend([w.y0, w.y1]);
    }
    let lines = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let mids: Vec<f64> = v.windows(2).map(|w| (w[0] + w[1]) / 2.0).collect();
        v.extend(mids);
        v.sort_by(f64::total_cmp);
        v
    };
    let (xs, ys) = (lines(xs), lines(ys));
    if xs.len() > MAX_LINES || ys.len() > MAX_LINES {
        return None;
    }
    let index = |v: &[f64], x: f64| v.iter().position(|&c| (c - x).abs() < 1e-6);
    let start = (index(&xs, from.x)?, index(&ys, from.y)?);
    let goal = (index(&xs, to.x)?, index(&ys, to.y)?);
    let point = |(i, j): (usize, usize)| Point::new(xs[i], ys[j]);
    let inside = |p: Point| {
        walls.iter().any(|w| {
            p.x > w.x0 + 1e-9 && p.x < w.x1 - 1e-9 && p.y > w.y0 + 1e-9 && p.y < w.y1 - 1e-9
        })
    };
    // Directions: right, left, down, up.
    const DIRS: [(isize, isize); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let dir_of = |v: Vec2| {
        DIRS.iter()
            .position(|&(dx, dy)| (dx as f64 - v.x).abs() < 1e-9 && (dy as f64 - v.y).abs() < 1e-9)
    };
    let (out, into) = (dir_of(out), dir_of(into));
    let heuristic = |n: (usize, usize)| {
        let (p, g) = (point(n), point(goal));
        (p.x - g.x).abs() + (p.y - g.y).abs()
    };
    // Costs are kept as integers (thousandths) for the heap.
    let key = |c: f64| (c * 1000.0).round() as u64;
    type State = ((usize, usize), usize);
    let mut best: HashMap<State, f64> = HashMap::new();
    let mut came: HashMap<State, State> = HashMap::new();
    let mut open = BinaryHeap::new();
    let first_dirs: Vec<usize> = match out {
        Some(d) => vec![d],
        None => (0..4).collect(),
    };
    for d in first_dirs {
        best.insert((start, d), 0.0);
        open.push(Reverse((key(heuristic(start)), start, d)));
    }
    let mut reached = None;
    while let Some(Reverse((_, node, dir))) = open.pop() {
        let cost = best[&(node, dir)];
        // Arriving the wrong way was charged on the last step, so the first arrival is
        // the cheapest.
        if node == goal {
            reached = Some((node, dir));
            break;
        }
        for (d, &(dx, dy)) in DIRS.iter().enumerate() {
            // No going straight back.
            if DIRS[dir] == (-dx, -dy) {
                continue;
            }
            let (Some(i), Some(j)) = (node.0.checked_add_signed(dx), node.1.checked_add_signed(dy))
            else {
                continue;
            };
            if i >= xs.len() || j >= ys.len() {
                continue;
            }
            let next = (i, j);
            let (p, q) = (point(node), point(next));
            if (next != goal && inside(q)) || walls.iter().any(|w| crosses(p, q, *w)) {
                continue;
            }
            let mut step = (q - p).hypot();
            if d != dir {
                step += BEND_COST;
            }
            if next == goal && into.is_some_and(|e| e != d) {
                step += BEND_COST;
            }
            let total = cost + step;
            if best.get(&(next, d)).is_none_or(|&c| total < c - 1e-9) {
                best.insert((next, d), total);
                came.insert((next, d), (node, dir));
                open.push(Reverse((key(total + heuristic(next)), next, d)));
            }
        }
    }
    let mut state = reached?;
    let mut points = vec![point(state.0)];
    while let Some(&prev) = came.get(&state) {
        points.push(point(prev.0));
        state = prev;
    }
    points.reverse();
    Some(straighten(points))
}

/// Drops the grid points in the middle of straight runs.
fn straighten(points: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for p in points {
        if let [.., a, b] = out[..]
            && ((a.x == b.x && b.x == p.x) || (a.y == b.y && b.y == p.y))
        {
            out.pop();
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goes_around_a_box_in_the_way() {
        let wall = Rect::new(100.0, -50.0, 200.0, 50.0);
        let path = route(
            Point::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Point::new(300.0, 0.0),
            Vec2::new(1.0, 0.0),
            &[wall],
        )
        .unwrap();
        assert_eq!(path.first(), Some(&Point::new(0.0, 0.0)));
        assert_eq!(path.last(), Some(&Point::new(300.0, 0.0)));
        assert!(!blocked(&path, &[wall], CLEARANCE - 1e-6), "{path:?}");
        for w in path.windows(2) {
            assert!(
                w[0].x == w[1].x || w[0].y == w[1].y,
                "not orthogonal: {path:?}"
            );
        }
        // Around the near side, not far away: four bends at most.
        assert!(path.len() <= 6, "{path:?}");
    }

    #[test]
    fn straight_when_nothing_is_in_the_way() {
        let path = route(
            Point::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Point::new(300.0, 0.0),
            Vec2::new(1.0, 0.0),
            &[Rect::new(100.0, 40.0, 200.0, 90.0)],
        )
        .unwrap();
        assert_eq!(path.len(), 2, "{path:?}");
    }

    #[test]
    fn blocked_ignores_touching_edges() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(blocked(
            &[Point::new(-5.0, 5.0), Point::new(15.0, 5.0)],
            &[r],
            0.0
        ));
        assert!(!blocked(
            &[Point::new(-5.0, 10.0), Point::new(15.0, 10.0)],
            &[r],
            0.0
        ));
        assert!(blocked(
            &[Point::new(-5.0, 11.0), Point::new(15.0, 11.0)],
            &[r],
            2.0
        ));
    }
}
