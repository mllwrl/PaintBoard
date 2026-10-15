use super::*;

fn at(x: f64, y: f64) -> Pointer {
    Pointer {
        world: Point::new(x, y),
        zoom: 1.0,
        shift: false,
        ctrl: false,
    }
}

fn shift(p: Pointer) -> Pointer {
    Pointer { shift: true, ..p }
}

fn doc_with(rects: &[Rect]) -> (Document, Vec<ObjectId>) {
    let mut doc = Document::new();
    let ids = rects
        .iter()
        .map(|&bounds| {
            doc.apply(Command::Create(ObjectData {
                kind: ObjectKind::Rectangle,
                bounds,
                style: Style::default(),
            }))
            .unwrap()
            .created[0]
        })
        .collect();
    (doc, ids)
}

fn click(tools: &mut Tools, doc: &Document, p: Pointer) -> Option<Command> {
    tools.pointer_down(doc, p);
    tools.pointer_up(doc, p)
}

fn drag(tools: &mut Tools, doc: &Document, from: Pointer, to: Pointer) -> Option<Command> {
    tools.pointer_down(doc, from);
    tools.pointer_move(doc, to);
    tools.pointer_up(doc, to)
}

fn created_bounds(cmd: Option<Command>) -> Rect {
    match cmd {
        Some(Command::Create(data)) => data.bounds,
        other => panic!("expected Create, got {other:?}"),
    }
}

// Shape tool

#[test]
fn drag_creates_rectangle_and_returns_to_select() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Rectangle);
    tools.pointer_down(&doc, at(50.0, 50.0));
    tools.pointer_move(&doc, at(10.0, 80.0));
    assert_eq!(
        tools.preview().unwrap().bounds,
        Rect::new(10.0, 50.0, 50.0, 80.0)
    );
    let bounds = created_bounds(tools.pointer_up(&doc, at(10.0, 80.0)));
    assert_eq!(bounds, Rect::new(10.0, 50.0, 50.0, 80.0));
    assert_eq!(tools.active(), ToolKind::Select);
    assert!(tools.preview().is_none());
}

#[test]
fn click_creates_default_size_centered() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Rectangle);
    let bounds = created_bounds(drag(&mut tools, &doc, at(0.0, 0.0), at(1.0, 1.0)));
    assert_eq!(bounds.center(), Point::ZERO);
    assert_eq!(bounds.width(), DEFAULT_SHAPE_SIZE);
}

#[test]
fn click_slop_is_measured_on_screen() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Rectangle);
    // 3 world units at 10x zoom are 30 screen pixels: a drag, not a click.
    let zoomed = |x, y| Pointer {
        zoom: 10.0,
        ..at(x, y)
    };
    let bounds = created_bounds(drag(&mut tools, &doc, zoomed(0.0, 0.0), zoomed(3.0, 3.0)));
    assert_eq!(bounds, Rect::new(0.0, 0.0, 3.0, 3.0));
}

#[test]
fn shift_constrains_to_square() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Rectangle);
    let bounds = created_bounds(drag(&mut tools, &doc, at(0.0, 0.0), shift(at(-40.0, 10.0))));
    assert_eq!(bounds, Rect::new(-40.0, 0.0, 0.0, 40.0));
}

#[test]
fn cancel_drops_shape_gesture() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Rectangle);
    tools.pointer_down(&doc, at(0.0, 0.0));
    assert!(tools.cancel());
    assert!(tools.pointer_up(&doc, at(50.0, 50.0)).is_none());
    assert_eq!(tools.active(), ToolKind::Rectangle);
}

// Selection

#[test]
fn click_selects_and_empty_click_clears() {
    let (doc, ids) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    assert!(click(&mut tools, &doc, at(50.0, 50.0)).is_none());
    assert_eq!(tools.selection().ids(), [ids[0]]);
    click(&mut tools, &doc, at(500.0, 500.0));
    assert!(tools.selection().is_empty());
}

#[test]
fn shift_click_toggles() {
    let (doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 0.0, 300.0, 100.0),
    ]);
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(50.0, 50.0));
    click(&mut tools, &doc, shift(at(250.0, 50.0)));
    assert_eq!(tools.selection().ids(), [ids[0], ids[1]]);
    click(&mut tools, &doc, shift(at(50.0, 50.0)));
    assert_eq!(tools.selection().ids(), [ids[1]]);
}

#[test]
fn click_inside_multi_selection_selects_only_that() {
    let (doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 0.0, 300.0, 100.0),
    ]);
    let mut tools = Tools::default();
    tools.select_all(&doc);
    click(&mut tools, &doc, at(250.0, 50.0));
    assert_eq!(tools.selection().ids(), [ids[1]]);
}

#[test]
fn marquee_selects_live_and_cancel_restores() {
    let (doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Rect::new(100.0, 100.0, 110.0, 110.0),
    ]);
    let mut tools = Tools::default();
    tools.pointer_down(&doc, at(-50.0, -50.0));
    tools.pointer_move(&doc, at(50.0, 50.0));
    assert_eq!(tools.selection().ids(), [ids[0]]);
    assert_eq!(tools.marquee(), Some(Rect::new(-50.0, -50.0, 50.0, 50.0)));
    tools.pointer_move(&doc, at(200.0, 200.0));
    assert_eq!(tools.selection().ids(), [ids[0], ids[1]]);
    tools.cancel();
    assert!(tools.selection().is_empty());
}

#[test]
fn shift_marquee_adds_to_selection() {
    let (doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Rect::new(100.0, 100.0, 110.0, 110.0),
    ]);
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(5.0, 5.0));
    drag(
        &mut tools,
        &doc,
        shift(at(90.0, 90.0)),
        shift(at(120.0, 120.0)),
    );
    assert_eq!(tools.selection().ids(), [ids[0], ids[1]]);
}

// Move

#[test]
fn drag_moves_selection_as_one_command() {
    let (doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 0.0, 300.0, 100.0),
    ]);
    let mut tools = Tools::default();
    tools.select_all(&doc);
    tools.pointer_down(&doc, at(50.0, 50.0));
    tools.pointer_move(&doc, at(60.0, 70.0));
    let preview = tools.overrides(&doc);
    assert_eq!(preview.len(), 2);
    assert_eq!(preview[1].data.bounds, Rect::new(210.0, 20.0, 310.0, 120.0));
    let cmd = tools.pointer_up(&doc, at(60.0, 70.0));
    assert_eq!(
        cmd,
        Some(Command::Move {
            ids: ids.clone(),
            delta: Vec2::new(10.0, 20.0),
        })
    );
    assert!(tools.overrides(&doc).is_empty());
}

#[test]
fn tiny_drag_does_not_move() {
    let (doc, _) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    assert!(drag(&mut tools, &doc, at(50.0, 50.0), at(51.0, 51.0)).is_none());
}

#[test]
fn nudge_and_delete_use_selection() {
    let (doc, ids) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    assert!(tools.delete_selected().is_none());
    click(&mut tools, &doc, at(50.0, 50.0));
    assert_eq!(tools.delete_selected(), Some(Command::Delete(ids.clone())));
    assert_eq!(
        tools.nudge_selected(Vec2::new(1.0, 0.0)),
        Some(Command::Move {
            ids,
            delta: Vec2::new(1.0, 0.0)
        })
    );
}

// Resize

#[test]
fn handle_drag_resizes_selection() {
    let (doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(100.0, 100.0, 200.0, 200.0),
    ]);
    let mut tools = Tools::default();
    tools.select_all(&doc);
    // Drag the south-east corner of the 200x200 selection out to 400x400.
    let cmd = drag(&mut tools, &doc, at(200.0, 200.0), at(400.0, 400.0));
    assert_eq!(
        cmd,
        Some(Command::SetBounds(vec![
            (ids[0], Rect::new(0.0, 0.0, 200.0, 200.0)),
            (ids[1], Rect::new(200.0, 200.0, 400.0, 400.0)),
        ]))
    );
}

#[test]
fn hovering_a_handle_sets_resize_cursor() {
    let (doc, _) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(50.0, 50.0));
    tools.pointer_move(&doc, at(100.0, 50.0));
    assert_eq!(tools.cursor(), CursorHint::ResizeEw);
    tools.pointer_move(&doc, at(50.0, 50.0));
    assert_eq!(tools.cursor(), CursorHint::Default);
}

#[test]
fn sync_drops_deleted_objects() {
    let (mut doc, ids) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(50.0, 50.0));
    doc.apply(Command::Delete(ids)).unwrap();
    tools.sync(&doc);
    assert!(tools.selection().is_empty());
}
