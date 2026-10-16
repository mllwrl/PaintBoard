//! Connector geometry: where a connector meets an object's outline, and the path between
//! its two ends.
//!
//! TODO: route elbow connectors around other objects (A* on a sparse grid).

use kurbo::{BezPath, Line, ParamCurve, ParamCurveNearest, PathSeg, Point, Rect, Shape, Vec2};

/// How a connector runs between its ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Route {
    #[default]
    Straight,
    /// Horizontal and vertical segments only.
    Elbow,
    Curved,
}

impl Route {
    pub const ALL: [Route; 3] = [Self::Straight, Self::Elbow, Self::Curved];

    pub fn name(self) -> &'static str {
        match self {
            Self::Straight => "Straight",
            Self::Elbow => "Elbow",
            Self::Curved => "Curved",
        }
    }
}

/// What a connector end looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cap {
    #[default]
    None,
    Arrow,
}

/// A resolved connector end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndPoint {
    pub point: Point,
    /// For ends on an object: the outward direction of its outline there (unit length).
    pub normal: Option<Vec2>,
}

impl EndPoint {
    pub fn free(point: Point) -> Self {
        Self {
            point,
            normal: None,
        }
    }
}

/// Elbow connectors leave and enter objects straight for this long, in world units.
const ELBOW_MARGIN: f64 = 24.0;
/// Curves leave their ends along the outline's normal, this far relative to their length.
const CURVE_TENSION: f64 = 0.4;
/// Arrowheads: length relative to the line width, plus a minimum, in world units.
const ARROW_LENGTH: f64 = 4.0;
const ARROW_MIN: f64 = 12.0;
/// Half the arrowhead's width, relative to its length.
const ARROW_SPREAD: f64 = 0.5;

/// Where a ray from `center` through `toward` leaves `outline`: its outermost crossing.
/// `None` if `toward` is `center` or the ray misses the outline.
pub fn ray_exit(outline: &BezPath, center: Point, toward: Point) -> Option<Point> {
    let dir = toward - center;
    let len = dir.hypot();
    if len < 1e-9 {
        return None;
    }
    // Long enough to leave any outline around the center.
    let reach = outline.bounding_box().size().to_vec2().hypot() * 2.0 + 1.0;
    let ray = Line::new(center, center + dir / len * reach);
    outline
        .segments()
        .flat_map(|seg| seg.intersect_line(ray))
        .map(|hit| hit.line_t)
        .fold(None, |best: Option<f64>, t| {
            Some(best.map_or(t, |b| b.max(t)))
        })
        .map(|t| ray.eval(t))
}

/// The outward normal of the side of `rect` nearest to `point`, measured relative to the
/// rectangle's size so that a wide box's ends are its left and right sides.
pub fn side_normal(rect: Rect, point: Point) -> Vec2 {
    let c = rect.center();
    let (hw, hh) = (rect.width().max(1e-9) / 2.0, rect.height().max(1e-9) / 2.0);
    let (dx, dy) = ((point.x - c.x) / hw, (point.y - c.y) / hh);
    if dx.abs() >= dy.abs() {
        Vec2::new(dx.signum(), 0.0)
    } else {
        Vec2::new(0.0, dy.signum())
    }
}

/// A drawable connector: its line and its arrowheads.
#[derive(Clone, Debug, Default)]
pub struct ConnectorPath {
    pub line: BezPath,
    pub heads: Vec<BezPath>,
}

impl ConnectorPath {
    /// Everything it paints, for culling and selection bounds.
    pub fn bounds(&self) -> Rect {
        self.heads
            .iter()
            .fold(self.line.bounding_box(), |r, h| r.union(h.bounding_box()))
    }

    /// Distance from `p` to the line or inside an arrowhead (zero).
    pub fn distance(&self, p: Point) -> f64 {
        if self.heads.iter().any(|h| h.contains(p)) {
            return 0.0;
        }
        self.line
            .segments()
            .map(|seg| seg.nearest(p, 1e-6).distance_sq.sqrt())
            .fold(f64::INFINITY, f64::min)
    }
}

/// The path of a connector between `start` and `end`, with arrowheads per `caps` sized for
/// a line `width` wide.
pub fn layout(
    route: Route,
    start: EndPoint,
    end: EndPoint,
    caps: (Cap, Cap),
    width: f64,
) -> ConnectorPath {
    let mut raw = match route {
        Route::Straight => Raw::Poly(vec![start.point, end.point]),
        Route::Elbow => Raw::Poly(elbow(start, end)),
        Route::Curved => curve(start, end),
    };
    let length = (width * ARROW_LENGTH).max(ARROW_MIN);
    let mut heads = Vec::new();
    if caps.0 == Cap::Arrow
        && let Some(dir) = raw.start_dir()
    {
        heads.push(arrow_head(start.point, dir, length));
        raw.pull_start(dir * length * 0.8);
    }
    if caps.1 == Cap::Arrow
        && let Some(dir) = raw.end_dir()
    {
        heads.push(arrow_head(end.point, dir, length));
        raw.pull_end(dir * length * 0.8);
    }
    ConnectorPath {
        line: raw.path(),
        heads,
    }
}

/// A connector path before it becomes a `BezPath`.
enum Raw {
    Poly(Vec<Point>),
    Cubic([Point; 4]),
}

impl Raw {
    /// Direction the path points into at its start (from the line toward the tip).
    fn start_dir(&self) -> Option<Vec2> {
        match self {
            Self::Poly(points) => unit(points[0] - *points.get(1)?),
            Self::Cubic([p0, c1, _, p3]) => unit(*p0 - *c1).or_else(|| unit(*p0 - *p3)),
        }
    }

    /// Direction the path travels in at its end.
    fn end_dir(&self) -> Option<Vec2> {
        match self {
            Self::Poly(points) => {
                let n = points.len();
                unit(points[n - 1] - *points.get(n.checked_sub(2)?)?)
            }
            Self::Cubic([p0, _, c2, p3]) => unit(*p3 - *c2).or_else(|| unit(*p3 - *p0)),
        }
    }

    /// Moves the start back by `by` (pointing toward the tip), so the line ends under an
    /// arrowhead instead of poking through its point.
    fn pull_start(&mut self, by: Vec2) {
        match self {
            Self::Poly(points) => {
                let first = points[0] - points[1];
                points[0] -= limit(by, first);
            }
            Self::Cubic(c) => c[0] -= by,
        }
    }

    fn pull_end(&mut self, by: Vec2) {
        match self {
            Self::Poly(points) => {
                let n = points.len();
                let last = points[n - 1] - points[n - 2];
                points[n - 1] -= limit(by, last);
            }
            Self::Cubic(c) => c[3] -= by,
        }
    }

    fn path(&self) -> BezPath {
        let mut path = BezPath::new();
        match self {
            Self::Poly(points) => {
                path.move_to(points[0]);
                points[1..].iter().for_each(|&p| path.line_to(p));
            }
            Self::Cubic([p0, c1, c2, p3]) => {
                path.move_to(*p0);
                path.curve_to(*c1, *c2, *p3);
            }
        }
        path
    }
}

fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.hypot();
    (len > 1e-9).then(|| v / len)
}

/// `by`, but no longer than most of `segment`, so short segments are not reversed.
fn limit(by: Vec2, segment: Vec2) -> Vec2 {
    let max = segment.hypot() * 0.9;
    let len = by.hypot();
    if len > max && len > 0.0 {
        by * (max / len)
    } else {
        by
    }
}

/// A filled arrowhead with its point at `tip`, pointing along `dir` (unit).
fn arrow_head(tip: Point, dir: Vec2, length: f64) -> BezPath {
    let base = tip - dir * length;
    let side = dir.turn_90() * (length * ARROW_SPREAD);
    let mut head = BezPath::new();
    head.move_to(tip);
    head.line_to(base + side);
    head.line_to(base - side);
    head.close_path();
    head
}

/// The axis direction closest to `v`.
fn axis(v: Vec2) -> Vec2 {
    if v.x.abs() >= v.y.abs() {
        Vec2::new(if v.x < 0.0 { -1.0 } else { 1.0 }, 0.0)
    } else {
        Vec2::new(0.0, if v.y < 0.0 { -1.0 } else { 1.0 })
    }
}

/// Corner points of an elbow connector. Ends on objects leave along their normal for a
/// short stub; the middle is one or two bends.
fn elbow(start: EndPoint, end: EndPoint) -> Vec<Point> {
    let (s, e) = (start.point, end.point);
    let ds = start.normal.map_or_else(|| axis(e - s), axis);
    let de = end.normal.map_or_else(|| axis(s - e), axis);
    let a = if start.normal.is_some() {
        s + ds * ELBOW_MARGIN
    } else {
        s
    };
    let b = if end.normal.is_some() {
        e + de * ELBOW_MARGIN
    } else {
        e
    };
    let horizontal = |d: Vec2| d.x != 0.0;
    let mut points = vec![s, a];
    match (horizontal(ds), horizontal(de)) {
        (true, true) => {
            let facing = (ds.x > 0.0) == (a.x <= b.x);
            if ds.x == de.x {
                // Both leave the same way: go around the farther one.
                let x = if ds.x > 0.0 {
                    a.x.max(b.x)
                } else {
                    a.x.min(b.x)
                };
                points.extend([Point::new(x, a.y), Point::new(x, b.y)]);
            } else if facing {
                let x = (a.x + b.x) / 2.0;
                points.extend([Point::new(x, a.y), Point::new(x, b.y)]);
            } else {
                let y = (a.y + b.y) / 2.0;
                points.extend([Point::new(a.x, y), Point::new(b.x, y)]);
            }
        }
        (false, false) => {
            let facing = (ds.y > 0.0) == (a.y <= b.y);
            if ds.y == de.y {
                let y = if ds.y > 0.0 {
                    a.y.max(b.y)
                } else {
                    a.y.min(b.y)
                };
                points.extend([Point::new(a.x, y), Point::new(b.x, y)]);
            } else if facing {
                let y = (a.y + b.y) / 2.0;
                points.extend([Point::new(a.x, y), Point::new(b.x, y)]);
            } else {
                let x = (a.x + b.x) / 2.0;
                points.extend([Point::new(x, a.y), Point::new(x, b.y)]);
            }
        }
        (true, false) => points.push(Point::new(b.x, a.y)),
        (false, true) => points.push(Point::new(a.x, b.y)),
    }
    points.extend([b, e]);
    simplify(points)
}

/// Drops repeated points and points in the middle of a straight run.
fn simplify(points: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_some_and(|&q| (p - q).hypot() < 1e-9) {
            continue;
        }
        if let [.., a, b] = out[..] {
            let (u, v) = (b - a, p - b);
            if u.cross(v).abs() < 1e-9 && u.dot(v) > 0.0 {
                out.pop();
            }
        }
        out.push(p);
    }
    if out.len() == 1 {
        out.push(out[0]);
    }
    out
}

fn curve(start: EndPoint, end: EndPoint) -> Raw {
    let (s, e) = (start.point, end.point);
    let k = (e - s).hypot() * CURVE_TENSION;
    let ds = start.normal.or_else(|| unit(e - s)).unwrap_or_default();
    let de = end.normal.or_else(|| unit(s - e)).unwrap_or_default();
    Raw::Cubic([s, s + ds * k, e + de * k, e])
}

/// Whether any part of the connector's line lies inside `area` (marquee selection).
pub fn touches(path: &ConnectorPath, area: Rect) -> bool {
    let mut inside = false;
    kurbo::flatten(&path.line, 0.25, |el| {
        if let kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) = el {
            inside |= area.contains(p);
        }
    });
    inside
        || path.line.segments().any(|seg| {
            let edges = [
                Line::new((area.x0, area.y0), (area.x1, area.y0)),
                Line::new((area.x1, area.y0), (area.x1, area.y1)),
                Line::new((area.x1, area.y1), (area.x0, area.y1)),
                Line::new((area.x0, area.y1), (area.x0, area.y0)),
            ];
            edges
                .iter()
                .any(|&e| !PathSeg::intersect_line(&seg, e).is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(path: &BezPath) -> Vec<Point> {
        path.elements()
            .iter()
            .filter_map(|el| match el {
                kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    fn on(point: Point, normal: Vec2) -> EndPoint {
        EndPoint {
            point,
            normal: Some(normal),
        }
    }

    #[test]
    fn ray_leaves_rect_and_ellipse_outlines() {
        let rect = Rect::new(0.0, 0.0, 100.0, 50.0);
        let c = rect.center();
        let exit = ray_exit(&rect.to_path(0.1), c, Point::new(500.0, 25.0)).unwrap();
        assert!((exit - Point::new(100.0, 25.0)).hypot() < 1e-6);
        let ellipse = kurbo::Ellipse::from_rect(rect).to_path(0.01);
        let exit = ray_exit(&ellipse, c, Point::new(50.0, -100.0)).unwrap();
        // Cubic approximations of the ellipse stray a little from it.
        assert!((exit - Point::new(50.0, 0.0)).hypot() < 0.01, "{exit:?}");
        assert_eq!(ray_exit(&ellipse, c, c), None);
    }

    #[test]
    fn side_normals_follow_the_aspect_ratio() {
        let r = Rect::new(0.0, 0.0, 200.0, 50.0);
        assert_eq!(side_normal(r, Point::new(190.0, 40.0)), Vec2::new(1.0, 0.0));
        assert_eq!(side_normal(r, Point::new(100.0, 0.0)), Vec2::new(0.0, -1.0));
    }

    #[test]
    fn straight_line_with_an_arrow_stops_under_its_head() {
        let path = layout(
            Route::Straight,
            EndPoint::free(Point::new(0.0, 0.0)),
            EndPoint::free(Point::new(100.0, 0.0)),
            (Cap::None, Cap::Arrow),
            2.0,
        );
        let line = points(&path.line);
        assert_eq!(line[0], Point::ZERO);
        assert!(line[1].x < 100.0 && line[1].x > 80.0, "{:?}", line[1]);
        assert_eq!(path.heads.len(), 1);
        assert!(path.heads[0].contains(Point::new(99.0, 0.0)));
        assert!(path.distance(Point::new(50.0, 3.0)) - 3.0 < 1e-9);
    }

    #[test]
    fn elbow_between_facing_sides_bends_in_the_middle() {
        let path = layout(
            Route::Elbow,
            on(Point::new(0.0, 0.0), Vec2::new(1.0, 0.0)),
            on(Point::new(200.0, 100.0), Vec2::new(-1.0, 0.0)),
            (Cap::None, Cap::None),
            2.0,
        );
        let line = points(&path.line);
        assert_eq!(
            line,
            [
                Point::new(0.0, 0.0),
                Point::new(100.0, 0.0),
                Point::new(100.0, 100.0),
                Point::new(200.0, 100.0),
            ]
        );
    }

    #[test]
    fn elbow_segments_are_axis_aligned_in_every_case() {
        let normals = [
            Vec2::new(1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(0.0, -1.0),
        ];
        for &ns in &normals {
            for &ne in &normals {
                for end in [Point::new(150.0, 80.0), Point::new(-150.0, -80.0)] {
                    let path = layout(
                        Route::Elbow,
                        on(Point::ZERO, ns),
                        on(end, ne),
                        (Cap::None, Cap::None),
                        2.0,
                    );
                    let line = points(&path.line);
                    assert_eq!(line[0], Point::ZERO);
                    assert_eq!(*line.last().unwrap(), end);
                    for w in line.windows(2) {
                        let d = w[1] - w[0];
                        assert!(d.x.abs() < 1e-9 || d.y.abs() < 1e-9, "{line:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn curves_leave_along_the_normals() {
        let path = layout(
            Route::Curved,
            on(Point::ZERO, Vec2::new(0.0, 1.0)),
            on(Point::new(100.0, 0.0), Vec2::new(0.0, 1.0)),
            (Cap::None, Cap::None),
            2.0,
        );
        // Both ends point down, so the curve dips below them.
        assert!(path.line.bounding_box().y1 > 20.0);
        assert!(path.bounds().y0 > -1e-9);
    }

    #[test]
    fn marquee_touches_lines_crossing_it() {
        let path = layout(
            Route::Straight,
            EndPoint::free(Point::new(-100.0, 5.0)),
            EndPoint::free(Point::new(100.0, 5.0)),
            (Cap::None, Cap::None),
            2.0,
        );
        assert!(touches(&path, Rect::new(-10.0, 0.0, 10.0, 10.0)));
        assert!(!touches(&path, Rect::new(-10.0, 20.0, 10.0, 30.0)));
    }
}
