//! Property tests: random edits keep the cached view right, and replicas that edit apart
//! agree once they have each other's changes.

use kurbo::Vec2;
use loro::ExportMode;
use proptest::prelude::*;

use super::Document;
use crate::{
    Anchor, Cap, Command, Connector, End, Label, ObjectData, ObjectId, Rgba, Route, ShapeKind,
    Style, StyleChange, ZOrder,
};

/// An edit, with objects named by their place in the current lists (wrapped around).
#[derive(Clone, Debug)]
enum Op {
    Create {
        x: i16,
        y: i16,
        w: u8,
        h: u8,
    },
    /// Several in one command, e.g. pasted.
    CreateMany(u8, i16),
    /// A pen stroke.
    Ink(i16, i16, u8),
    DeleteMany(usize, usize, usize),
    /// Adds (or reorders) layer `n` at an order around the base layer's.
    DefineLayer(u8, i8),
    /// New objects go to layer `n` (0 is the base layer).
    ActiveLayer(u8),
    SetLayer(usize, u8),
    Connect(usize, usize),
    Move(usize, i8, i8),
    Fill(usize, u8),
    Text(usize, String),
    Delete(usize),
    Group(usize, usize),
    Ungroup(usize),
    Lock(usize, bool),
    Hide(usize, bool),
    Reorder(usize, bool),
    Undo,
    Redo,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (any::<i16>(), any::<i16>(), 1..200u8, 1..200u8)
            .prop_map(|(x, y, w, h)| Op::Create { x: x / 64, y: y / 64, w, h }),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(a, b)| Op::Connect(a, b)),
        3 => (any::<usize>(), any::<i8>(), any::<i8>()).prop_map(|(i, x, y)| Op::Move(i, x, y)),
        1 => (any::<usize>(), any::<u8>()).prop_map(|(i, c)| Op::Fill(i, c)),
        1 => (any::<usize>(), "[a-z ]{0,8}").prop_map(|(i, t)| Op::Text(i, t)),
        2 => any::<usize>().prop_map(Op::Delete),
        1 => (1..5u8, any::<i16>()).prop_map(|(n, x)| Op::CreateMany(n, x / 64)),
        3 => (any::<i16>(), any::<i16>(), 2..20u8).prop_map(|(x, y, n)| Op::Ink(x / 64, y / 64, n)),
        1 => (any::<usize>(), any::<usize>(), any::<usize>())
            .prop_map(|(a, b, c)| Op::DeleteMany(a, b, c)),
        1 => (1..3u8, any::<i8>()).prop_map(|(n, order)| Op::DefineLayer(n, order)),
        1 => (0..3u8).prop_map(Op::ActiveLayer),
        1 => (any::<usize>(), 0..3u8).prop_map(|(i, n)| Op::SetLayer(i, n)),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(a, b)| Op::Group(a, b)),
        1 => any::<usize>().prop_map(Op::Ungroup),
        1 => (any::<usize>(), any::<bool>()).prop_map(|(i, on)| Op::Lock(i, on)),
        1 => (any::<usize>(), any::<bool>()).prop_map(|(i, on)| Op::Hide(i, on)),
        1 => (any::<usize>(), any::<bool>()).prop_map(|(i, front)| Op::Reorder(i, front)),
        1 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> ObjectData {
    ObjectData {
        kind: ShapeKind::Rectangle.into(),
        bounds: kurbo::Rect::new(x, y, x + w, y + h),
        rotation: 0.0,
        style: Style::default(),
        tags: Default::default(),
        link: None,
    }
}

fn connector(a: ObjectId, b: ObjectId) -> ObjectData {
    let end = |object| End::Attached {
        object,
        anchor: Anchor::Auto,
        at: kurbo::Point::ZERO,
    };
    ObjectData {
        kind: crate::ObjectKind::Connector(Connector {
            bends: Default::default(),
            start: end(a),
            end: end(b),
            route: Route::Elbow,
            start_cap: Cap::None,
            end_cap: Cap::Arrow,
            label: Label::default(),
        }),
        bounds: kurbo::Rect::ZERO,
        rotation: 0.0,
        style: Style::default(),
        tags: Default::default(),
        link: None,
    }
}

/// Layer `n`'s id; 0 is the base layer.
fn layer(n: u8) -> std::sync::Arc<str> {
    match n {
        0 => crate::Layer::BASE.into(),
        n => format!("layer {n}").into(),
    }
}

/// Applies `op`; commands that do not apply (a locked object, nothing there) are fine.
fn apply(doc: &mut Document, op: &Op) {
    let object = |i: usize| {
        let objects = doc.objects();
        (!objects.is_empty()).then(|| objects[i % objects.len()].id)
    };
    let root = |i: usize| {
        let roots = doc.roots();
        (!roots.is_empty()).then(|| roots[i % roots.len()])
    };
    let command = match op {
        Op::Create { x, y, w, h } => Some(Command::Create(vec![
            rect(
                f64::from(*x) * 10.0,
                f64::from(*y) * 10.0,
                f64::from(*w),
                f64::from(*h),
            )
            .into(),
        ])),
        Op::CreateMany(n, x) => Some(Command::Create(
            (0..*n)
                .map(|k| rect(f64::from(*x) * 10.0 + f64::from(k) * 15.0, 0.0, 10.0, 10.0).into())
                .collect(),
        )),
        Op::Ink(x, y, n) => {
            let points: Vec<_> = (0..*n)
                .map(|k| {
                    let k = f64::from(k);
                    let at = kurbo::Point::new(
                        f64::from(*x) * 10.0 + k * 4.0,
                        f64::from(*y) * 10.0 + (k * 0.7).sin() * 6.0,
                    );
                    (at, 0.5)
                })
                .collect();
            crate::Ink::from_world(&points).map(|(ink, bounds)| {
                let mut data = rect(0.0, 0.0, 0.0, 0.0);
                data.kind = crate::ObjectKind::Ink(ink);
                data.bounds = bounds;
                Command::Create(vec![data.into()])
            })
        }
        Op::DeleteMany(a, b, c) => {
            let ids: Vec<_> = [a, b, c].into_iter().filter_map(|&i| object(i)).collect();
            (!ids.is_empty()).then_some(Command::Delete(ids))
        }
        Op::DefineLayer(n, order) => Some(Command::DefineLayer(crate::Layer {
            id: layer(*n),
            name: layer(*n),
            order: f64::from(*order) / 16.0,
            visible: true,
            locked: false,
        })),
        Op::ActiveLayer(n) => {
            doc.set_active_layer(layer(*n));
            None
        }
        Op::SetLayer(i, n) => root(*i).map(|id| Command::SetLayer {
            ids: vec![id],
            layer: layer(*n),
        }),
        Op::Connect(a, b) => object(*a)
            .zip(object(*b))
            .map(|(a, b)| Command::Create(vec![connector(a, b).into()])),
        Op::Move(i, x, y) => object(*i).map(|id| Command::Move {
            ids: vec![id],
            delta: Vec2::new(f64::from(*x), f64::from(*y)),
        }),
        Op::Fill(i, c) => object(*i).map(|id| Command::SetStyle {
            ids: vec![id],
            change: StyleChange {
                fill: Some(Rgba::rgb(*c, 255 - *c, 128)),
                ..StyleChange::default()
            },
        }),
        Op::Text(i, text) => object(*i).and_then(|id| {
            let bounds = doc.get(id)?.data.bounds;
            Some(Command::SetText {
                id,
                text: text.as_str().into(),
                spans: Default::default(),
                bounds,
            })
        }),
        Op::Delete(i) => object(*i).map(|id| Command::Delete(vec![id])),
        Op::Group(a, b) => root(*a)
            .zip(root(*b))
            .filter(|(a, b)| a != b)
            .map(|(a, b)| Command::Group(vec![a, b])),
        Op::Ungroup(i) => root(*i)
            .filter(|id| doc.group(*id).is_some())
            .map(|id| Command::Ungroup(vec![id])),
        Op::Lock(i, locked) => object(*i).map(|id| Command::SetLocked {
            ids: vec![id],
            locked: *locked,
        }),
        Op::Hide(i, hidden) => object(*i).map(|id| Command::SetHidden {
            ids: vec![id],
            hidden: *hidden,
        }),
        Op::Reorder(i, front) => root(*i).map(|id| Command::Reorder {
            ids: vec![id],
            to: if *front { ZOrder::Front } else { ZOrder::Back },
        }),
        Op::Undo => {
            let _ = doc.undo();
            None
        }
        Op::Redo => {
            let _ = doc.redo();
            None
        }
    };
    if let Some(command) = command {
        let _ = doc.apply(command);
    }
}

/// What a board shows: its objects, how they stack, group and layer, and what is found
/// where.
#[allow(clippy::type_complexity)]
fn view(
    doc: &Document,
) -> (
    Vec<crate::Object>,
    Vec<ObjectId>,
    Vec<Vec<ObjectId>>,
    Vec<(Option<usize>, std::sync::Arc<str>)>,
    Vec<crate::Group>,
) {
    let areas = [
        kurbo::Rect::new(-1e6, -1e6, 1e6, 1e6),
        kurbo::Rect::new(-500.0, -500.0, 0.0, 0.0),
        kurbo::Rect::new(0.0, 0.0, 300.0, 300.0),
        kurbo::Rect::new(-50.0, -50.0, 50.0, 50.0),
    ];
    let found = areas
        .iter()
        .map(|&area| doc.objects_in(area).iter().map(|o| o.id).collect())
        .collect();
    let placed = doc
        .objects()
        .iter()
        .map(|o| (doc.paint_order(o.id), doc.layer_of(o.id).id.clone()))
        .collect();
    let groups = doc
        .roots()
        .iter()
        .filter_map(|&id| doc.group(id).cloned())
        .collect();
    assert_eq!(
        doc.index.len(),
        doc.objects().len(),
        "paint order of gone objects"
    );
    assert_eq!(
        doc.spatial.size(),
        doc.objects().len(),
        "spatial index size"
    );
    (
        doc.objects().to_vec(),
        doc.roots().to_vec(),
        found,
        placed,
        groups,
    )
}

fn fresh(doc: &Document) -> Document {
    Document::from_snapshot(&doc.snapshot()).expect("a snapshot reads back")
}

/// 64 cases a run; PROPTEST_CASES asks for more (e.g. 2000 for a long hunt).
fn config() -> ProptestConfig {
    let cases = if std::env::var_os("PROPTEST_CASES").is_some() {
        ProptestConfig::default().cases
    } else {
        64
    };
    ProptestConfig {
        cases,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    /// After any edits the cached view, kept up to date incrementally, is the one a board
    /// read afresh gives.
    #[test]
    fn cached_view_matches_a_fresh_read(ops in prop::collection::vec(op(), 1..30)) {
        let mut doc = Document::new();
        for op in &ops {
            apply(&mut doc, op);
            prop_assert_eq!(view(&doc), view(&fresh(&doc)), "after {:?}", op);
        }
    }

    /// Two replicas that edit the same board apart agree once each has the other's
    /// changes, and their cached views are right.
    #[test]
    fn replicas_converge(
        base in prop::collection::vec(op(), 0..10),
        left in prop::collection::vec(op(), 0..15),
        right in prop::collection::vec(op(), 0..15),
    ) {
        let mut start = Document::new();
        for op in &base {
            apply(&mut start, op);
        }
        let mut a = fresh(&start);
        let mut b = fresh(&start);
        a.doc.set_peer_id(1).unwrap();
        b.doc.set_peer_id(2).unwrap();
        for op in &left {
            apply(&mut a, op);
        }
        for op in &right {
            apply(&mut b, op);
        }
        let from_a = a.doc.export(ExportMode::all_updates()).unwrap();
        let from_b = b.doc.export(ExportMode::all_updates()).unwrap();
        a.doc.import(&from_b).unwrap();
        b.doc.import(&from_a).unwrap();
        a.refresh();
        b.refresh();
        prop_assert_eq!(view(&a), view(&b));
        prop_assert_eq!(view(&a), view(&fresh(&a)));
    }
}
