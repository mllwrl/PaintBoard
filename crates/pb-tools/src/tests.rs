use super::*;

fn at(x: f64, y: f64) -> Pointer {
    Pointer {
        world: Point::new(x, y),
        zoom: 1.0,
        shift: false,
        ctrl: false,
        alt: false,
    }
}

fn alt(p: Pointer) -> Pointer {
    Pointer { alt: true, ..p }
}

fn shift(p: Pointer) -> Pointer {
    Pointer { shift: true, ..p }
}

fn doc_with(rects: &[Rect]) -> (Document, Vec<ObjectId>) {
    let mut doc = Document::new();
    let ids = rects
        .iter()
        .map(|&bounds| {
            doc.apply(Command::Create(vec![shape(ObjectKind::Rectangle, bounds)]))
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
        Some(Command::Create(items)) if items.len() == 1 => items[0].bounds,
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
        tools.previews(&doc)[0].bounds,
        Rect::new(10.0, 50.0, 50.0, 80.0)
    );
    let bounds = created_bounds(tools.pointer_up(&doc, at(10.0, 80.0)));
    assert_eq!(bounds, Rect::new(10.0, 50.0, 50.0, 80.0));
    assert_eq!(tools.active(), ToolKind::Select);
    assert!(tools.previews(&doc).is_empty());
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
        Some(Command::SetGeometry(vec![
            (
                ids[0],
                Frame::axis_aligned(Rect::new(0.0, 0.0, 200.0, 200.0))
            ),
            (
                ids[1],
                Frame::axis_aligned(Rect::new(200.0, 200.0, 400.0, 400.0))
            ),
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

// Rotation

fn frame_of_cmd(cmd: Option<Command>) -> Vec<Frame> {
    match cmd {
        Some(Command::SetGeometry(changes)) => changes.into_iter().map(|(_, f)| f).collect(),
        other => panic!("expected SetGeometry, got {other:?}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn knob_drag_rotates_single_object_around_center() {
    use std::f64::consts::FRAC_PI_2;
    let (doc, _) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(50.0, 50.0));
    // The knob is straight above the center; dragging it to the right is a quarter turn.
    let knob = rotation_knob(tools.selection().frame(&doc).unwrap(), 1.0);
    assert_eq!(knob, Point::new(50.0, -24.0));
    let frames = frame_of_cmd(drag(&mut tools, &doc, at(knob.x, knob.y), at(150.0, 50.0)));
    assert!(close(frames[0].rotation, FRAC_PI_2));
    assert_eq!(frames[0].rect, Rect::new(0.0, 0.0, 100.0, 100.0));
}

#[test]
fn shift_snaps_rotation_to_15_degrees() {
    let (doc, _) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(50.0, 50.0));
    // About 20 degrees clockwise from straight up snaps to 15.
    let target = Point::new(50.0 + 74.0 * 0.342, 50.0 - 74.0 * 0.940);
    let frames = frame_of_cmd(drag(
        &mut tools,
        &doc,
        at(50.0, -24.0),
        shift(at(target.x, target.y)),
    ));
    assert!(close(frames[0].rotation, 15f64.to_radians()));
}

#[test]
fn rotating_group_orbits_children() {
    use std::f64::consts::PI;
    let (doc, _) = doc_with(&[
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Rect::new(90.0, 0.0, 100.0, 10.0),
    ]);
    let mut tools = Tools::default();
    tools.select_all(&doc);
    // Group frame 0..100 x 0..10, center (50, 5); knob at (50, -24). Drag to below: half turn.
    let frames = frame_of_cmd(drag(&mut tools, &doc, at(50.0, -24.0), at(50.0, 100.0)));
    assert!((frames[0].center() - Point::new(95.0, 5.0)).hypot() < 1e-9);
    assert!(close(frames[0].rotation.abs(), PI));
}

#[test]
fn resizing_rotated_object_keeps_opposite_edge() {
    use std::f64::consts::FRAC_PI_2;
    let mut doc = Document::new();
    let mut data = shape(ObjectKind::Rectangle, Rect::new(0.0, 0.0, 100.0, 20.0));
    data.rotation = FRAC_PI_2;
    let id = doc.apply(Command::Create(vec![data])).unwrap().created[0];
    let mut tools = Tools::default();
    tools.select_only([id]);
    // Rotated a quarter turn, the east handle sits at the bottom: (50, 60).
    let frames = frame_of_cmd(drag(&mut tools, &doc, at(50.0, 60.0), at(50.0, 110.0)));
    let f = frames[0];
    assert!(close(f.rect.width(), 150.0));
    // The west edge (now the top, at y = -40) did not move.
    let top = f.to_world(Point::new(f.rect.x0, f.rect.center().y));
    assert!((top - Point::new(50.0, -40.0)).hypot() < 1e-9);
}

#[test]
fn rotated_objects_hit_by_real_shape() {
    use std::f64::consts::FRAC_PI_2;
    let mut doc = Document::new();
    let mut data = shape(ObjectKind::Rectangle, Rect::new(0.0, 0.0, 100.0, 20.0));
    data.rotation = FRAC_PI_2;
    doc.apply(Command::Create(vec![data])).unwrap();
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(90.0, 10.0));
    assert!(tools.selection().is_empty());
    click(&mut tools, &doc, at(50.0, 50.0));
    assert_eq!(tools.selection().ids().len(), 1);
}

// Duplicate

#[test]
fn alt_drag_duplicates_instead_of_moving() {
    let (doc, ids) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    tools.pointer_down(&doc, alt(at(50.0, 50.0)));
    tools.pointer_move(&doc, alt(at(250.0, 50.0)));
    assert!(tools.overrides(&doc).is_empty());
    assert_eq!(
        tools.previews(&doc)[0].bounds,
        Rect::new(200.0, 0.0, 300.0, 100.0)
    );
    assert_eq!(tools.cursor(), CursorHint::Copy);
    let bounds = created_bounds(tools.pointer_up(&doc, alt(at(250.0, 50.0))));
    assert_eq!(bounds, Rect::new(200.0, 0.0, 300.0, 100.0));
    assert_eq!(tools.selection().ids(), [ids[0]]);
}
