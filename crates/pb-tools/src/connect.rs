//! Where connector ends go: onto an object's connection point, onto the object itself, or
//! free on the board.

use pb_core::connector::{Anchor, End, connection_points};
use pb_core::{Document, ObjectId};
use pb_geom::kurbo::Point;

use crate::{Pointer, hit};

/// Connection points catch the pointer within this distance, in logical pixels.
pub(crate) const SNAP_RADIUS: f64 = 12.0;

/// Which end of a connector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Start,
    End,
}

/// The end for the pointer: a connection point of the object under (or right next to) it,
/// the object itself with an automatic anchor, or a free point. `avoid` is an object the
/// end may not attach to (the connector's other end, so it does not loop onto itself).
pub(crate) fn end_at(doc: &Document, pointer: &Pointer, avoid: Option<ObjectId>) -> End {
    let p = pointer.world;
    let radius = SNAP_RADIUS / pointer.zoom;
    let Some(target) = hit::attachable_at(doc, p, radius).filter(|o| Some(o.id) != avoid) else {
        return End::Free(p);
    };
    let near = connection_points(&target.data)
        .into_iter()
        .map(|(anchor, at)| (anchor, at, (at - p).hypot()))
        .filter(|&(_, _, d)| d <= radius)
        .min_by(|a, b| a.2.total_cmp(&b.2));
    match near {
        Some((anchor, at, _)) => End::Attached {
            object: target.id,
            anchor,
            at,
        },
        None => End::Attached {
            object: target.id,
            anchor: Anchor::Auto,
            at: p,
        },
    }
}

/// What to show while picking an end: the target's connection points, and the one the end
/// snaps to.
pub(crate) fn hints(doc: &Document, end: &End) -> (Vec<Point>, Option<Point>) {
    let End::Attached { object, anchor, at } = *end else {
        return (Vec::new(), None);
    };
    let Some(target) = doc.get(object) else {
        return (Vec::new(), None);
    };
    let points = connection_points(&target.data).map(|(_, p)| p).to_vec();
    let active = matches!(anchor, Anchor::Fixed(_)).then_some(at);
    (points, active)
}
