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
    Create { x: i16, y: i16, w: u8, h: u8 },
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
        1 => any::<usize>().prop_map(Op::Delete),
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

/// What a board shows: its objects, how they stack and group, and what is found where.
fn view(doc: &Document) -> (Vec<crate::Object>, Vec<ObjectId>, Vec<Vec<ObjectId>>) {
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
    (doc.objects().to_vec(), doc.roots().to_vec(), found)
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
