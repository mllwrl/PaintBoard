//! Pen, marker and eraser.

use pb_core::{Document, Ink, ObjectData, ObjectId, ObjectKind, Rgba, Style};
use pb_geom::ink::simplify;
use pb_geom::kurbo::{Point, Rect};

/// New points closer than this to the previous one are skipped, in logical pixels.
pub(crate) const MIN_STEP: f64 = 1.0;
/// Finished strokes are simplified within this distance, in logical pixels.
const SIMPLIFY_TOLERANCE: f64 = 0.3;
/// Eraser radius in logical pixels.
pub const ERASER_RADIUS: f64 = 8.0;

/// Color and thickness of the pen or the marker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkStyle {
    pub color: Rgba,
    /// In logical pixels at the zoom the stroke is drawn at, so tools feel the same at any
    /// zoom.
    pub width: f64,
}

impl InkStyle {
    pub const PEN: Self = Self {
        color: Rgba::rgb(0x1F, 0x29, 0x37),
        width: 3.0,
    };
    pub const MARKER: Self = Self {
        color: Rgba::rgb(0xFF, 0xD5, 0x4F),
        width: 18.0,
    };
}

/// Marker strokes are see-through, like a highlighter.
const MARKER_OPACITY: f64 = 0.5;

/// The stroke drawn so far as an object. `finished` simplifies it for storage.
pub(crate) fn stroke(
    points: &[(Point, f64)],
    style: InkStyle,
    marker: bool,
    zoom: f64,
    finished: bool,
) -> Option<ObjectData> {
    let simplified: Vec<_>;
    let points = if finished {
        let positions: Vec<_> = points.iter().map(|(p, _)| *p).collect();
        let keep = simplify(&positions, SIMPLIFY_TOLERANCE / zoom);
        simplified = keep.into_iter().map(|i| points[i]).collect();
        &simplified[..]
    } else {
        points
    };
    let (ink, bounds) = Ink::from_world(points)?;
    Some(ObjectData {
        kind: ObjectKind::Ink(ink),
        bounds,
        rotation: 0.0,
        style: Style {
            fill: Rgba(0),
            stroke: style.color,
            stroke_width: style.width / zoom,
            opacity: if marker { MARKER_OPACITY } else { 1.0 },
            ..Style::default()
        },
        tags: Default::default(),
        link: None,
    })
}

/// Unlocked ink objects touched by the eraser moving from `from` to `to`.
pub(crate) fn erased(doc: &Document, from: Point, to: Point, radius: f64) -> Vec<ObjectId> {
    // Sample the sweep densely enough that no stroke slips between two samples.
    let steps = ((to - from).hypot() / (radius / 2.0)).ceil().max(1.0) as usize;
    let samples: Vec<_> = (0..=steps)
        .map(|i| from.lerp(to, i as f64 / steps as f64))
        .collect();
    let swept = Rect::from_points(from, to).inflate(radius, radius);
    doc.objects_in(swept)
        .into_iter()
        .filter(|o| o.data.kind.is_ink() && !doc.is_locked(o.id) && doc.reachable(o.id))
        .filter(|o| {
            samples
                .iter()
                .any(|&p| crate::hit::hits(&o.data, p, radius))
        })
        .map(|o| o.id)
        .collect()
}
