use kurbo::{Line, Rect, Vec2};

/// Features closer than this count as lined up (world units).
const EPSILON: f64 = 1e-6;

/// Snaps dragged edges and centers to the edges and centers of other objects, or else to
/// the grid.
///
/// Callers pass the features being dragged on each axis: all three of a moved box, just
/// the dragged edge of a resized one, or a single point.
#[derive(Clone, Debug, Default)]
pub struct Snapper {
    /// Bounds of the objects to snap to.
    pub targets: Vec<Rect>,
    /// Snap distance in world units.
    pub tolerance: f64,
    /// Grid spacing to snap to where no object is close, if grid snapping is on.
    pub grid: Option<f64>,
}

impl Snapper {
    /// The offset that brings the closest of `xs` (and of `ys`) onto a target feature.
    /// Along an axis with no target in reach, the first feature goes onto the grid.
    pub fn offset(&self, xs: &[f64], ys: &[f64]) -> Vec2 {
        Vec2::new(self.axis(xs, x_features), self.axis(ys, y_features))
    }

    fn axis(&self, values: &[f64], features: fn(&Rect) -> [f64; 3]) -> f64 {
        let nearest = self
            .targets
            .iter()
            .flat_map(features)
            .flat_map(|t| values.iter().map(move |v| t - v))
            .filter(|d| d.abs() <= self.tolerance)
            .min_by(|a, b| a.abs().total_cmp(&b.abs()));
        match (nearest, self.grid, values.first()) {
            (Some(d), _, _) => d,
            (None, Some(step), Some(&v)) if step > 0.0 => (v / step).round() * step - v,
            _ => 0.0,
        }
    }

    /// Guide lines for the features that line up with targets after snapping: vertical
    /// lines through `xs`, horizontal ones through `ys`, each spanning `extent` (the
    /// dragged bounds) and every target it touches.
    pub fn guides(&self, extent: Rect, xs: &[f64], ys: &[f64]) -> Vec<Line> {
        let mut lines = Vec::new();
        for &x in dedup(xs).iter() {
            if let Some((y0, y1)) =
                self.span(x, x_features, (extent.y0, extent.y1), |t| (t.y0, t.y1))
            {
                lines.push(Line::new((x, y0), (x, y1)));
            }
        }
        for &y in dedup(ys).iter() {
            if let Some((x0, x1)) =
                self.span(y, y_features, (extent.x0, extent.x1), |t| (t.x0, t.x1))
            {
                lines.push(Line::new((x0, y), (x1, y)));
            }
        }
        lines
    }

    /// The extent across the axis of everything lined up at `at`, if any target is.
    fn span(
        &self,
        at: f64,
        features: fn(&Rect) -> [f64; 3],
        extent: (f64, f64),
        across: fn(&Rect) -> (f64, f64),
    ) -> Option<(f64, f64)> {
        let mut span = None;
        for target in &self.targets {
            if features(target).iter().any(|f| (f - at).abs() < EPSILON) {
                let (a, b) = across(target);
                let (lo, hi) = span.unwrap_or(extent);
                span = Some((lo.min(a), hi.max(b)));
            }
        }
        span
    }
}

fn x_features(r: &Rect) -> [f64; 3] {
    [r.x0, r.center().x, r.x1]
}

fn y_features(r: &Rect) -> [f64; 3] {
    [r.y0, r.center().y, r.y1]
}

fn dedup(values: &[f64]) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::with_capacity(values.len());
    for &v in values {
        if !out.iter().any(|o| (o - v).abs() < EPSILON) {
            out.push(v);
        }
    }
    out
}

/// Edges and center of `r` along x, for [`Snapper`].
pub fn xs_of(r: Rect) -> [f64; 3] {
    x_features(&r)
}

/// Edges and center of `r` along y, for [`Snapper`].
pub fn ys_of(r: Rect) -> [f64; 3] {
    y_features(&r)
}

#[cfg(test)]
mod tests {
    use kurbo::Point;

    use super::*;

    fn snapper(grid: Option<f64>) -> Snapper {
        Snapper {
            targets: vec![Rect::new(100.0, 100.0, 200.0, 150.0)],
            tolerance: 5.0,
            grid,
        }
    }

    #[test]
    fn snaps_nearest_feature_within_tolerance() {
        let s = snapper(None);
        // Left edge 97 is 3 away from the target's left edge 100.
        let moving = Rect::new(97.0, 300.0, 117.0, 320.0);
        let offset = s.offset(&xs_of(moving), &ys_of(moving));
        assert_eq!(offset, Vec2::new(3.0, 0.0));
        // Centers line up too: 148 -> 150.
        let moving = Rect::new(138.0, 0.0, 158.0, 10.0);
        assert_eq!(s.offset(&xs_of(moving), &[]).x, 2.0);
    }

    #[test]
    fn falls_back_to_grid() {
        let s = snapper(Some(20.0));
        let offset = s.offset(&[33.0], &[124.0]);
        // x: nothing near 33, grid 40. y: target center 125 is closer than any grid line.
        assert_eq!(offset, Vec2::new(7.0, 1.0));
        assert_eq!(snapper(None).offset(&[33.0], &[]), Vec2::ZERO);
    }

    #[test]
    fn guides_span_dragged_and_target() {
        let s = snapper(None);
        let moved = Rect::new(100.0, 300.0, 120.0, 320.0);
        let guides = s.guides(moved, &xs_of(moved), &ys_of(moved));
        assert_eq!(
            guides,
            [Line::new(
                Point::new(100.0, 100.0),
                Point::new(100.0, 320.0)
            )]
        );
    }
}
