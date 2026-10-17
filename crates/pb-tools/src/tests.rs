use pb_geom::kurbo::Line;

use super::*;

fn at(x: f64, y: f64) -> Pointer {
    Pointer {
        world: Point::new(x, y),
        zoom: 1.0,
        visible: Rect::new(-1e6, -1e6, 1e6, 1e6),
        pressure: 1.0,
        shift: false,
        ctrl: false,
        alt: false,
    }
}

fn shape(kind: ObjectKind, bounds: Rect) -> ObjectData {
    ObjectData {
        kind,
        bounds,
        rotation: 0.0,
        style: Style::default(),
        tags: Default::default(),
        link: None,
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
            doc.apply(Command::Create(vec![
                shape(ShapeKind::Rectangle.into(), bounds).into(),
            ]))
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
        Some(Command::Create(items)) => match &items[..] {
            [Content::Object(data) | Content::Copied { data, .. }] => data.bounds,
            other => panic!("expected one object, got {other:?}"),
        },
        other => panic!("expected Create, got {other:?}"),
    }
}

// Shape tool

#[test]
fn drag_creates_rectangle_and_returns_to_select() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Shape(ShapeKind::Rectangle));
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
    tools.set_active(ToolKind::Shape(ShapeKind::Rectangle));
    let bounds = created_bounds(drag(&mut tools, &doc, at(0.0, 0.0), at(1.0, 1.0)));
    assert_eq!(bounds.center(), Point::ZERO);
    assert_eq!(bounds.width(), 100.0);
    // Diagram shapes come in their own proportions.
    tools.set_active(ToolKind::Shape(ShapeKind::Terminator));
    let bounds = created_bounds(drag(&mut tools, &doc, at(0.0, 0.0), at(1.0, 1.0)));
    assert_eq!(bounds.size(), ShapeKind::Terminator.default_size().into());
}

#[test]
fn click_slop_is_measured_on_screen() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Shape(ShapeKind::Rectangle));
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
    tools.set_active(ToolKind::Shape(ShapeKind::Rectangle));
    let bounds = created_bounds(drag(&mut tools, &doc, at(0.0, 0.0), shift(at(-40.0, 10.0))));
    assert_eq!(bounds, Rect::new(-40.0, 0.0, 0.0, 40.0));
}

#[test]
fn cancel_drops_shape_gesture() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Shape(ShapeKind::Rectangle));
    tools.pointer_down(&doc, at(0.0, 0.0));
    assert!(tools.cancel());
    assert!(tools.pointer_up(&doc, at(50.0, 50.0)).is_none());
    assert_eq!(tools.active(), ToolKind::Shape(ShapeKind::Rectangle));
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
    assert!(tools.delete_selected(&doc).is_none());
    click(&mut tools, &doc, at(50.0, 50.0));
    assert_eq!(
        tools.delete_selected(&doc),
        Some(Command::Delete(ids.clone()))
    );
    assert_eq!(
        tools.nudge_selected(&doc, Vec2::new(1.0, 0.0)),
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
    let mut data = shape(
        ShapeKind::Rectangle.into(),
        Rect::new(0.0, 0.0, 100.0, 20.0),
    );
    data.rotation = FRAC_PI_2;
    let id = doc
        .apply(Command::Create(vec![data.into()]))
        .unwrap()
        .created[0];
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
    let mut data = shape(
        ShapeKind::Rectangle.into(),
        Rect::new(0.0, 0.0, 100.0, 20.0),
    );
    data.rotation = FRAC_PI_2;
    doc.apply(Command::Create(vec![data.into()])).unwrap();
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

// Groups and locking

fn grouped(doc: &mut Document, ids: &[ObjectId]) -> ObjectId {
    doc.apply(Command::Group(ids.to_vec())).unwrap().created[0]
}

fn locked(doc: &mut Document, ids: &[ObjectId]) {
    doc.apply(Command::SetLocked {
        ids: ids.to_vec(),
        locked: true,
    })
    .unwrap();
}

#[test]
fn clicking_a_member_selects_and_drags_the_group() {
    let (mut doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 0.0, 300.0, 100.0),
    ]);
    let g = grouped(&mut doc, &ids);
    let mut tools = Tools::default();
    tools.pointer_down(&doc, at(250.0, 50.0));
    assert_eq!(tools.selection().ids(), [g]);
    tools.pointer_move(&doc, at(260.0, 50.0));
    assert_eq!(tools.overrides(&doc).len(), 2);
    let cmd = tools.pointer_up(&doc, at(260.0, 50.0));
    assert_eq!(
        cmd,
        Some(Command::Move {
            ids: vec![g],
            delta: Vec2::new(10.0, 0.0)
        })
    );
    // The selection frame wraps the whole group.
    assert_eq!(
        tools.selection().frame(&doc).unwrap().rect,
        Rect::new(0.0, 0.0, 300.0, 100.0)
    );
}

#[test]
fn group_resizes_as_a_box() {
    let (mut doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(100.0, 100.0, 200.0, 200.0),
    ]);
    grouped(&mut doc, &ids);
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(50.0, 50.0));
    let frames = frame_of_cmd(drag(&mut tools, &doc, at(200.0, 200.0), at(400.0, 400.0)));
    assert_eq!(frames[1].rect, Rect::new(200.0, 200.0, 400.0, 400.0));
}

#[test]
fn group_and_ungroup_commands() {
    let (mut doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Rect::new(20.0, 0.0, 30.0, 10.0),
    ]);
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(5.0, 5.0));
    assert!(tools.group_selected(&doc).is_none());
    tools.select_all(&doc);
    let cmd = tools.group_selected(&doc).unwrap();
    let g = doc.apply(cmd).unwrap().created[0];
    tools.sync(&doc);
    // Members selected before grouping now stand for their group.
    assert_eq!(tools.selection().ids(), [g]);
    assert_eq!(
        tools.ungroup_selected(&doc),
        Some(Command::Ungroup(vec![g]))
    );
    tools.select_only([ids[0]]);
    assert!(tools.ungroup_selected(&doc).is_none());
}

#[test]
fn locked_objects_select_but_do_not_move() {
    let (mut doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 0.0, 300.0, 100.0),
    ]);
    locked(&mut doc, &[ids[0]]);
    let mut tools = Tools::default();
    assert!(drag(&mut tools, &doc, at(50.0, 50.0), at(80.0, 80.0)).is_none());
    assert_eq!(tools.selection().ids(), [ids[0]]);
    assert!(tools.overrides(&doc).is_empty());
    assert!(tools.delete_selected(&doc).is_none());
    assert!(tools.nudge_selected(&doc, Vec2::new(1.0, 0.0)).is_none());
    assert!(!tools.shows_handles(&doc));
    // No resize handle on a locked object.
    assert!(drag(&mut tools, &doc, at(100.0, 100.0), at(150.0, 150.0)).is_none());
    assert_eq!(
        tools.toggle_lock_selected(&doc),
        Some(Command::SetLocked {
            ids: vec![ids[0]],
            locked: false
        })
    );
    tools.select_all(&doc);
    assert_eq!(tools.selection().ids(), [ids[1]]);
}

#[test]
fn mixed_selection_moves_only_unlocked() {
    let (mut doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 0.0, 300.0, 100.0),
    ]);
    locked(&mut doc, &[ids[0]]);
    let mut tools = Tools::default();
    click(&mut tools, &doc, at(50.0, 50.0));
    click(&mut tools, &doc, shift(at(250.0, 50.0)));
    assert_eq!(tools.selection().ids().len(), 2);
    let cmd = drag(&mut tools, &doc, at(250.0, 50.0), at(260.0, 50.0));
    assert_eq!(
        cmd,
        Some(Command::Move {
            ids: vec![ids[1]],
            delta: Vec2::new(10.0, 0.0)
        })
    );
    assert_eq!(
        tools.toggle_lock_selected(&doc),
        Some(Command::SetLocked {
            ids: vec![ids[0], ids[1]],
            locked: true
        })
    );
}

// Snapping

fn ctrl(p: Pointer) -> Pointer {
    Pointer { ctrl: true, ..p }
}

fn moved_by(cmd: Option<Command>) -> Vec2 {
    match cmd {
        Some(Command::Move { delta, .. }) => delta,
        other => panic!("expected Move, got {other:?}"),
    }
}

#[test]
fn move_snaps_to_nearby_edges_with_guides() {
    let (doc, _) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 300.0, 260.0, 400.0),
    ]);
    let mut tools = Tools::default();
    tools.pointer_down(&doc, at(230.0, 350.0));
    // Left edge lands at 4, 4 px from the other object's left edge: snaps to 0.
    tools.pointer_move(&doc, at(34.0, 350.0));
    let guides = tools.guides(&doc);
    assert_eq!(
        guides,
        [Line::new(Point::new(0.0, 0.0), Point::new(0.0, 400.0))]
    );
    assert_eq!(
        moved_by(tools.pointer_up(&doc, at(34.0, 350.0))),
        Vec2::new(-200.0, 0.0)
    );
    assert!(tools.guides(&doc).is_empty());
}

#[test]
fn ctrl_turns_snapping_off() {
    let (doc, _) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 300.0, 260.0, 400.0),
    ]);
    let mut tools = Tools::default();
    let cmd = drag(&mut tools, &doc, at(230.0, 350.0), ctrl(at(34.0, 350.0)));
    assert_eq!(moved_by(cmd), Vec2::new(-196.0, 0.0));
}

#[test]
fn far_or_offscreen_objects_do_not_snap() {
    let (doc, _) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 300.0, 260.0, 400.0),
    ]);
    let mut tools = Tools::default();
    // 10 px away is out of reach.
    let cmd = drag(&mut tools, &doc, at(230.0, 350.0), at(40.0, 350.0));
    assert_eq!(moved_by(cmd), Vec2::new(-190.0, 0.0));
    // In reach, but the target is off screen.
    let offscreen = |x, y| Pointer {
        visible: Rect::new(150.0, 150.0, 1000.0, 1000.0),
        ..at(x, y)
    };
    let cmd = drag(
        &mut tools,
        &doc,
        offscreen(230.0, 350.0),
        offscreen(34.0, 350.0),
    );
    assert_eq!(moved_by(cmd), Vec2::new(-196.0, 0.0));
}

#[test]
fn grid_snapping_rounds_the_top_left_corner() {
    let (doc, _) = doc_with(&[Rect::new(3.0, 7.0, 53.0, 57.0)]);
    let mut tools = Tools::default();
    tools.set_snap_to_grid(true);
    // At zoom 1 the visible grid is 20 world units.
    let cmd = drag(&mut tools, &doc, at(20.0, 20.0), at(45.0, 31.0));
    // Corner (3, 7) + (25, 11) = (28, 18) goes to (20, 20).
    assert_eq!(moved_by(cmd), Vec2::new(17.0, 13.0));
}

#[test]
fn resize_snaps_dragged_edge() {
    let (doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(0.0, 200.0, 153.0, 300.0),
    ]);
    let mut tools = Tools::default();
    tools.select_only([ids[0]]);
    tools.pointer_down(&doc, at(100.0, 50.0));
    tools.pointer_move(&doc, at(149.0, 50.0));
    assert_eq!(
        tools.guides(&doc),
        [Line::new(Point::new(153.0, 0.0), Point::new(153.0, 300.0))]
    );
    let frames = frame_of_cmd(tools.pointer_up(&doc, at(149.0, 50.0)));
    assert_eq!(frames[0].rect, Rect::new(0.0, 0.0, 153.0, 100.0));
}

#[test]
fn image_corners_keep_aspect_unless_shift() {
    let mut doc = Document::new();
    let asset = doc.add_asset(b"img".to_vec());
    let mut data = shape(
        ShapeKind::Rectangle.into(),
        Rect::new(0.0, 0.0, 200.0, 100.0),
    );
    data.kind = ObjectKind::Image(pb_core::Image::new(asset));
    let id = doc
        .apply(Command::Create(vec![data.into()]))
        .unwrap()
        .created[0];
    let mut tools = Tools::default();
    tools.select_only([id]);
    let kept = frame_of_cmd(drag(&mut tools, &doc, at(200.0, 100.0), at(400.0, 120.0)));
    let r = kept[0].rect;
    assert!((r.width() / r.height() - 2.0).abs() < 1e-9, "{r:?}");
    let free = frame_of_cmd(drag(
        &mut tools,
        &doc,
        shift(at(200.0, 100.0)),
        shift(at(400.0, 120.0)),
    ));
    assert_eq!(free[0].rect, Rect::new(0.0, 0.0, 400.0, 120.0));
}

#[test]
fn shape_corners_snap() {
    let (doc, _) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Shape(ShapeKind::Rectangle));
    let bounds = created_bounds(drag(&mut tools, &doc, at(103.0, -2.0), at(197.0, 52.0)));
    assert_eq!(bounds, Rect::new(100.0, 0.0, 197.0, 50.0));
}

#[test]
fn alt_drag_copies_snap_to_the_original() {
    let (doc, _) = doc_with(&[Rect::new(0.0, 0.0, 100.0, 100.0)]);
    let mut tools = Tools::default();
    tools.pointer_down(&doc, alt(at(50.0, 50.0)));
    let bounds = created_bounds(tools.pointer_up(&doc, alt(at(153.0, 52.0))));
    // Left edge 103 snaps to the original's right edge, top 2 to its top.
    assert_eq!(bounds, Rect::new(100.0, 0.0, 200.0, 100.0));
}

// Shape kinds

#[test]
fn shape_tool_draws_the_chosen_kind_and_cycles() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.cycle_shape();
    assert_eq!(tools.active(), ToolKind::Shape(ShapeKind::Rectangle));
    tools.cycle_shape();
    assert_eq!(tools.active(), ToolKind::Shape(ShapeKind::RoundedRectangle));
    tools.set_active(ToolKind::Shape(ShapeKind::Star));
    let cmd = drag(&mut tools, &doc, at(0.0, 0.0), at(50.0, 50.0));
    match cmd {
        Some(Command::Create(items)) => {
            assert!(matches!(&items[..], [Content::Object(d)] if d.kind == ShapeKind::Star.into()));
        }
        other => panic!("expected Create, got {other:?}"),
    }
    // Back to Select after drawing; the shape tool remembers the star.
    tools.cycle_shape();
    assert_eq!(tools.active(), ToolKind::Shape(ShapeKind::Star));
}

// Ink

fn ink_of(cmd: Option<Command>) -> ObjectData {
    match cmd {
        Some(Command::Create(items)) => match &items[..] {
            [Content::Object(data)] if data.kind.is_ink() => data.clone(),
            other => panic!("expected one stroke, got {other:?}"),
        },
        other => panic!("expected Create, got {other:?}"),
    }
}

fn draw(tools: &mut Tools, doc: &Document, points: &[Pointer]) -> Option<Command> {
    tools.pointer_down(doc, points[0]);
    for &p in &points[1..] {
        tools.pointer_move(doc, p);
    }
    tools.pointer_up(doc, *points.last().unwrap())
}

#[test]
fn pen_draws_a_simplified_stroke_and_stays_active() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Pen);
    let line: Vec<_> = (0..=20).map(|i| at(i as f64 * 5.0, 10.0)).collect();
    tools.pointer_down(&doc, line[0]);
    tools.pointer_move(&doc, line[1]);
    // While drawing, the stroke shows as a preview.
    assert!(tools.previews(&doc)[0].kind.is_ink());
    for &p in &line[2..] {
        tools.pointer_move(&doc, p);
    }
    let data = ink_of(tools.pointer_up(&doc, line[20]));
    let ObjectKind::Ink(ink) = &data.kind else {
        unreachable!()
    };
    // A straight line needs only its two ends.
    assert_eq!(ink.points().len(), 2);
    assert_eq!(data.bounds, Rect::new(0.0, 10.0, 100.0, 10.0));
    assert_eq!(data.style.stroke_width, InkStyle::PEN.width);
    assert_eq!(tools.active(), ToolKind::Pen);
}

#[test]
fn ink_width_follows_zoom_and_pressure_is_kept() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Pen);
    let pen = |x, y, pressure| Pointer {
        zoom: 2.0,
        pressure,
        ..at(x, y)
    };
    let data = ink_of(draw(
        &mut tools,
        &doc,
        &[
            pen(0.0, 0.0, 0.2),
            pen(10.0, 10.0, 0.9),
            pen(20.0, 0.0, 0.5),
        ],
    ));
    assert_eq!(data.style.stroke_width, InkStyle::PEN.width / 2.0);
    let ObjectKind::Ink(ink) = &data.kind else {
        unreachable!()
    };
    assert!(ink.has_pressure());
}

#[test]
fn marker_is_wide_and_see_through() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Marker);
    let data = ink_of(draw(&mut tools, &doc, &[at(0.0, 0.0), at(50.0, 0.0)]));
    assert_eq!(data.style.stroke_width, InkStyle::MARKER.width);
    assert!(data.style.opacity < 1.0);
}

#[test]
fn click_with_pen_makes_a_dot() {
    let doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Pen);
    let data = ink_of(click(&mut tools, &doc, at(5.0, 5.0)));
    assert_eq!(data.bounds, Rect::new(5.0, 5.0, 5.0, 5.0));
}

#[test]
fn ink_is_hit_along_its_line() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Pen);
    let cmd = draw(&mut tools, &doc, &[at(0.0, 0.0), at(100.0, 100.0)]).unwrap();
    let id = doc.apply(cmd).unwrap().created[0];
    tools.set_active(ToolKind::Select);
    // Inside the bounds but far from the diagonal: nothing.
    click(&mut tools, &doc, at(90.0, 10.0));
    assert!(tools.selection().is_empty());
    click(&mut tools, &doc, at(51.0, 50.0));
    assert_eq!(tools.selection().ids(), [id]);
}

#[test]
fn eraser_removes_touched_strokes_only() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Pen);
    let a = draw(&mut tools, &doc, &[at(0.0, 0.0), at(100.0, 0.0)]).unwrap();
    let b = draw(&mut tools, &doc, &[at(0.0, 50.0), at(100.0, 50.0)]).unwrap();
    let a = doc.apply(a).unwrap().created[0];
    doc.apply(b).unwrap();
    let rect = doc
        .apply(Command::Create(vec![
            shape(
                ShapeKind::Rectangle.into(),
                Rect::new(40.0, -20.0, 60.0, 20.0),
            )
            .into(),
        ]))
        .unwrap()
        .created[0];
    tools.set_active(ToolKind::Eraser);
    // Sweep across the first stroke (and the rectangle), fast enough to skip pixels.
    tools.pointer_down(&doc, at(50.0, -30.0));
    tools.pointer_move(&doc, at(50.0, 30.0));
    assert_eq!(tools.hidden(), [a]);
    assert_eq!(
        tools.pointer_up(&doc, at(50.0, 30.0)),
        Some(Command::Delete(vec![a]))
    );
    assert!(doc.get(rect).is_some());
    assert!(tools.eraser().is_some());
}

// Connectors

fn the_connector(doc: &Document) -> (ObjectId, Connector) {
    doc.objects()
        .iter()
        .find_map(|o| match &o.data.kind {
            ObjectKind::Connector(c) => Some((o.id, c.clone())),
            _ => None,
        })
        .expect("a connector")
}

fn apply(doc: &mut Document, cmd: Option<Command>) -> pb_core::Applied {
    doc.apply(cmd.expect("a command")).unwrap()
}

#[test]
fn connector_tool_connects_two_objects() {
    let (mut doc, ids) = doc_with(&[
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(300.0, 0.0, 400.0, 100.0),
    ]);
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Connector);
    tools.pointer_down(&doc, at(50.0, 50.0));
    // Near the second box's left connection point (300, 50): it snaps there.
    tools.pointer_move(&doc, at(305.0, 45.0));
    let (points, active) = tools.connection_hints(&doc);
    assert_eq!(points.len(), 4);
    assert!(active.is_some_and(|p| (p - Point::new(300.0, 50.0)).hypot() < 1e-6));
    assert_eq!(tools.previews(&doc).len(), 1);
    let cmd = tools.pointer_up(&doc, at(305.0, 45.0));
    apply(&mut doc, cmd);
    assert_eq!(tools.active(), ToolKind::Select);
    let (id, c) = the_connector(&doc);
    assert_eq!(c.start.object(), Some(ids[0]));
    assert!(matches!(
        c.start,
        End::Attached {
            anchor: pb_core::Anchor::Auto,
            ..
        }
    ));
    assert!(matches!(
        c.end,
        End::Attached { object, anchor: pb_core::Anchor::Fixed(_), .. } if object == ids[1]
    ));
    let (s, e) = doc.connector_ends(id).unwrap();
    assert!((s.point - Point::new(100.0, 50.0)).hypot() < 1e-6);
    assert!((e.point - Point::new(300.0, 50.0)).hypot() < 1e-6);
}

#[test]
fn connector_click_draws_nothing_and_free_drag_draws_a_line() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Connector);
    assert!(click(&mut tools, &doc, at(10.0, 10.0)).is_none());
    tools.set_active(ToolKind::Connector);
    let cmd = drag(&mut tools, &doc, at(10.0, 10.0), at(200.0, 10.0));
    apply(&mut doc, cmd);
    let (_, c) = the_connector(&doc);
    assert_eq!(c.start, End::Free(Point::new(10.0, 10.0)));
    assert_eq!(c.end, End::Free(Point::new(200.0, 10.0)));
}

#[test]
fn connectors_are_picked_along_their_line_and_ends_can_be_reattached() {
    let (mut doc, ids) = doc_with(&[Rect::new(300.0, 0.0, 400.0, 100.0)]);
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Connector);
    let cmd = drag(&mut tools, &doc, at(0.0, 300.0), at(200.0, 300.0));
    let id = apply(&mut doc, cmd).created[0];
    tools.sync(&doc);
    // Beside the line nothing is hit; on it the connector is.
    assert_eq!(tools.object_at(&doc, at(100.0, 330.0)), None);
    assert_eq!(tools.object_at(&doc, at(100.0, 301.0)), Some(id));
    click(&mut tools, &doc, at(100.0, 300.0));
    assert_eq!(tools.selection().ids(), [id]);
    assert!(!tools.shows_handles(&doc));
    let handles = tools.connector_handles(&doc).unwrap();
    assert!((handles[1] - Point::new(200.0, 300.0)).hypot() < 1e-6);
    // Drag the end onto the box.
    let cmd = drag(&mut tools, &doc, at(200.0, 300.0), at(350.0, 50.0));
    apply(&mut doc, cmd);
    let (_, c) = the_connector(&doc);
    assert_eq!(c.end.object(), Some(ids[0]));
    assert_eq!(c.start, End::Free(Point::new(0.0, 300.0)));
}

#[test]
fn marquee_selects_connectors_by_their_line() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Connector);
    let cmd = drag(&mut tools, &doc, at(0.0, 0.0), at(400.0, 400.0));
    let id = apply(&mut doc, cmd).created[0];
    tools.sync(&doc);
    // A box inside the connector's bounds but away from the diagonal.
    drag(&mut tools, &doc, at(300.0, 20.0), at(380.0, 80.0));
    assert!(tools.selection().is_empty());
    drag(&mut tools, &doc, at(190.0, 190.0), at(210.0, 210.0));
    assert_eq!(tools.selection().ids(), [id]);
}

#[test]
fn labels_are_hit_and_slide_along_the_line() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Connector);
    let cmd = drag(&mut tools, &doc, at(0.0, 0.0), at(400.0, 0.0));
    let id = apply(&mut doc, cmd).created[0];
    let label = Label {
        text: "label".into(),
        at: 0.25,
        size: pb_geom::kurbo::Size::new(60.0, 40.0),
    };
    doc.apply(Command::SetLabel { id, label }).unwrap();
    tools.sync(&doc);
    let middle = doc.connector_label(id).unwrap().center();
    // The label's box is part of the connector, away from the line too.
    let below = at(middle.x, middle.y + 15.0);
    assert_eq!(tools.object_at(&doc, below), Some(id));
    click(&mut tools, &doc, below);
    assert_eq!(tools.selection().ids(), [id]);
    // A click on the label of the selected connector leaves it; a drag slides it.
    assert!(click(&mut tools, &doc, below).is_none());
    let cmd = drag(&mut tools, &doc, below, at(middle.x + 100.0, 200.0));
    let Some(Command::SetLabel { label, .. }) = &cmd else {
        panic!("expected SetLabel, got {cmd:?}");
    };
    assert!(label.at > 0.45 && label.at < 0.55, "{}", label.at);
    assert_eq!(&*label.text, "label");
}

// Frames

/// A board with a frame at (0, 0)-(400, 300) and a rectangle inside it, applied as made.
fn framed() -> (Document, ObjectId, ObjectId) {
    let mut tools = Tools::default();
    let mut doc = Document::new();
    tools.set_active(ToolKind::Frame);
    let cmd = drag(&mut tools, &doc, at(0.0, 0.0), at(400.0, 300.0)).unwrap();
    let frame = doc.apply(cmd).unwrap().created[0];
    let inside = doc
        .apply(Command::Create(vec![
            shape(
                ShapeKind::Rectangle.into(),
                Rect::new(50.0, 50.0, 100.0, 100.0),
            )
            .into(),
        ]))
        .unwrap()
        .created[0];
    (doc, frame, inside)
}

#[test]
fn frame_tool_draws_or_places_named_frames() {
    let (doc, frame, _) = framed();
    let data = &doc.get(frame).unwrap().data;
    assert_eq!(data.bounds, Rect::new(0.0, 0.0, 400.0, 300.0));
    assert_eq!(data.kind.text(), Some("Frame 1"));
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Frame);
    let p = Pointer {
        zoom: 2.0,
        ..at(1000.0, 1000.0)
    };
    let Some(Command::Create(items)) = click(&mut tools, &doc, p) else {
        panic!("expected Create");
    };
    let [Content::Object(data)] = &items[..] else {
        panic!("expected one frame");
    };
    // 16:9 at the zoom it was placed at, and the next name.
    assert_eq!(data.bounds.size(), pb_geom::kurbo::Size::new(320.0, 180.0));
    assert_eq!(data.kind.text(), Some("Frame 2"));
    assert_eq!(tools.active(), ToolKind::Select);
}

#[test]
fn frames_are_picked_by_their_title_and_carry_their_contents() {
    let (doc, frame, inside) = framed();
    let mut tools = Tools::default();
    // The body is see-through: a press there selects nothing and starts a marquee.
    assert_eq!(tools.object_at(&doc, at(300.0, 200.0)), None);
    assert_eq!(tools.object_at(&doc, at(75.0, 75.0)), Some(inside));
    let title = at(20.0, -8.0);
    assert_eq!(tools.object_at(&doc, title), Some(frame));
    let Some(Command::Move { ids, delta }) = drag(&mut tools, &doc, title, at(70.0, 42.0)) else {
        panic!("expected Move");
    };
    assert_eq!(delta, Vec2::new(50.0, 50.0));
    assert!(ids.contains(&frame) && ids.contains(&inside));
    // Frames do not rotate.
    assert!(!tools.can_rotate(&doc));
    // Copies take the contents along.
    let Some(Command::Create(items)) = clipboard::duplicate(&doc, tools.selection()) else {
        panic!("expected Create");
    };
    assert_eq!(items.len(), 2);
}

#[test]
fn marquee_in_a_frame_selects_its_contents_only() {
    let (doc, frame, inside) = framed();
    let mut tools = Tools::default();
    drag(&mut tools, &doc, at(300.0, 250.0), at(40.0, 40.0));
    assert_eq!(tools.selection().ids(), &[inside]);
    drag(&mut tools, &doc, at(-10.0, -10.0), at(410.0, 310.0));
    assert!(tools.selection().contains(frame) && tools.selection().contains(inside));
}

// Tables

#[test]
fn table_tool_places_a_grid_and_lines_come_and_go() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Table);
    let cmd = click(&mut tools, &doc, at(0.0, 0.0)).unwrap();
    let id = doc.apply(cmd).unwrap().created[0];
    let data = doc.get(id).unwrap().data.clone();
    let ObjectKind::Table(table) = &data.kind else {
        panic!("not a table");
    };
    assert_eq!((table.rows.len(), table.cols.len()), (3, 3));
    assert_eq!(data.bounds.size(), pb_geom::kurbo::Size::new(360.0, 132.0));
    tools.select_only([id]);
    let cmd = tools.add_table_line(&doc, true, true).unwrap();
    doc.apply(cmd).unwrap();
    let data = doc.get(id).unwrap().data.clone();
    let ObjectKind::Table(table) = &data.kind else {
        panic!("not a table");
    };
    assert_eq!(table.rows.len(), 4);
    assert!((data.bounds.height() - 176.0).abs() < 1e-9);
    // Rows keep their size.
    let rect = table.cell_rect(data.bounds, 0, 0).unwrap();
    assert!((rect.height() - 44.0).abs() < 1e-9);
    let cmd = tools.remove_table_lines(&doc, false).unwrap();
    doc.apply(cmd).unwrap();
    let data = doc.get(id).unwrap().data.clone();
    let ObjectKind::Table(table) = &data.kind else {
        panic!("not a table");
    };
    assert_eq!(table.cols.len(), 2);
    assert!((data.bounds.width() - 240.0).abs() < 1e-9);
}

#[test]
fn table_cells_are_selected_and_lines_go_in_between() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Table);
    let cmd = click(&mut tools, &doc, at(0.0, 0.0)).unwrap();
    let id = doc.apply(cmd).unwrap().created[0];
    let table = |doc: &Document| match &doc.get(id).unwrap().data.kind {
        ObjectKind::Table(t) => t.clone(),
        _ => panic!("not a table"),
    };
    let bounds = doc.get(id).unwrap().data.bounds;
    let center = |r: usize, c: usize| {
        let rect = table(&doc).cell_rect(bounds, r, c).unwrap();
        at(rect.center().x, rect.center().y)
    };
    // With the table selected, a click picks a cell, and a drag then picks a block.
    tools.select_only([id]);
    assert_eq!(tools.table_cells(&doc), None);
    click(&mut tools, &doc, center(1, 1));
    assert_eq!(tools.table_cells(&doc), Some((id, CellRange::cell((1, 1)))));
    assert!(drag(&mut tools, &doc, center(0, 1), center(1, 2)).is_none());
    let block = CellRange::new((0, 1), (1, 2));
    assert_eq!(tools.table_cells(&doc), Some((id, block)));
    let mut shift = center(2, 2);
    shift.shift = true;
    click(&mut tools, &doc, shift);
    assert_eq!(
        tools.table_cells(&doc),
        Some((id, CellRange::new((0, 1), (2, 2))))
    );
    click(&mut tools, &doc, center(1, 1));

    // A row above the cell: the cell moves down with its row.
    let before = table(&doc).rows[1].id.clone();
    let cmd = tools.add_table_line(&doc, true, false).unwrap();
    doc.apply(cmd).unwrap();
    tools.sync(&doc);
    let t = table(&doc);
    assert_eq!(t.rows.len(), 4);
    assert_eq!(t.rows[2].id, before);
    assert_eq!(tools.table_cells(&doc), Some((id, CellRange::cell((2, 1)))));
    // A column to the right.
    let cmd = tools.add_table_line(&doc, false, true).unwrap();
    doc.apply(cmd).unwrap();
    let t = table(&doc);
    assert_eq!(t.cols.len(), 4);
    assert_eq!(tools.table_cells(&doc), Some((id, CellRange::cell((2, 1)))));
    // Deleting the cell's column removes that column only.
    let gone = t.cols[1].id.clone();
    let cmd = tools.remove_table_lines(&doc, false).unwrap();
    doc.apply(cmd).unwrap();
    tools.sync(&doc);
    let t = table(&doc);
    assert_eq!(t.cols.len(), 3);
    assert!(t.cols.iter().all(|c| c.id != gone));
    assert_eq!(tools.table_cells(&doc), None);
    assert_eq!(tools.selection().ids(), &[id]);
}

#[test]
fn deleting_with_cells_selected_empties_them() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Table);
    let cmd = click(&mut tools, &doc, at(0.0, 0.0)).unwrap();
    let id = doc.apply(cmd).unwrap().created[0];
    let ObjectKind::Table(t) = doc.get(id).unwrap().data.kind.clone() else {
        panic!("not a table");
    };
    for (r, c) in [(0, 0), (0, 1), (2, 2)] {
        doc.apply(Command::SetCell {
            id,
            row: t.rows[r].id.clone(),
            col: t.cols[c].id.clone(),
            cell: pb_core::Cell {
                text: "x".into(),
                spans: Default::default(),
            },
        })
        .unwrap();
    }
    tools.select_cell(id, (0, 0));
    assert_eq!(tools.clear_cells_text(&doc).len(), 1);
    tools.select_only([id]);
    assert!(tools.clear_cells_text(&doc).is_empty());
}

#[test]
fn selected_cells_merge_and_split() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Table);
    let cmd = click(&mut tools, &doc, at(0.0, 0.0)).unwrap();
    let id = doc.apply(cmd).unwrap().created[0];
    let merged = |doc: &Document| match &doc.get(id).unwrap().data.kind {
        ObjectKind::Table(t) => t.merged(),
        _ => panic!("not a table"),
    };
    tools.select_cell(id, (0, 0));
    assert!(!tools.can_merge_cells(&doc, true));
    assert!(tools.merge_cells(&doc, true).is_none());
    tools.select_cell(id, (0, 0));
    tools.cells.as_mut().unwrap().end = (1, 1);
    let cmd = tools.merge_cells(&doc, true).unwrap();
    doc.apply(cmd).unwrap();
    tools.sync(&doc);
    let block = CellRange::new((0, 0), (1, 1));
    assert_eq!(merged(&doc), vec![block]);
    assert!(!tools.can_merge_cells(&doc, true));
    // Picking a cell of the block picks all of it.
    tools.select_cell(id, (1, 1));
    assert_eq!(tools.table_cells(&doc), Some((id, block)));
    // A row below the block stays out of it.
    let cmd = tools.add_table_line(&doc, true, true).unwrap();
    doc.apply(cmd).unwrap();
    assert_eq!(merged(&doc), vec![block]);
    // Splitting from any part of it.
    tools.select_cell(id, (1, 0));
    let cmd = tools.merge_cells(&doc, false).unwrap();
    doc.apply(cmd).unwrap();
    assert!(merged(&doc).is_empty());
    // Removing the bottom row shrinks a block that reaches it.
    tools.select_cell(id, (2, 0));
    tools.cells.as_mut().unwrap().end = (3, 2);
    doc.apply(tools.merge_cells(&doc, true).unwrap()).unwrap();
    tools.select_only([id]);
    let cmd = tools.remove_table_lines(&doc, true).unwrap();
    doc.apply(cmd).unwrap();
    assert_eq!(merged(&doc), vec![CellRange::new((2, 0), (2, 2))]);
}

// Kanban boards

#[test]
fn kanban_cards_are_dragged_selected_and_deleted() {
    let mut doc = Document::new();
    let mut tools = Tools::default();
    tools.set_active(ToolKind::Kanban);
    let cmd = click(&mut tools, &doc, at(0.0, 0.0)).unwrap();
    let id = doc.apply(cmd).unwrap().created[0];
    let board = |doc: &Document| match &doc.get(id).unwrap().data {
        data @ ObjectData {
            kind: ObjectKind::Kanban(k),
            ..
        } => (k.clone(), data.bounds),
        _ => panic!("not a kanban"),
    };
    let (k, bounds) = board(&doc);
    assert_eq!(k.columns.len(), 3);
    assert_eq!(bounds.width(), 720.0);
    assert_eq!(bounds.height(), k.fitted_height(720.0));
    for (n, text) in ["a", "b"].into_iter().enumerate() {
        let card = pb_core::Card {
            id: doc.new_line_id().into(),
            column: k.columns[0].id.clone(),
            order: n as f64,
            text: text.into(),
            spans: Default::default(),
        };
        let (k, b) = board(&doc);
        let bounds = k.with_card(card.clone()).fitted(b);
        doc.apply(Command::SetCard { id, card, bounds }).unwrap();
    }
    tools.select_only([id]);
    let (k, bounds) = board(&doc);
    let a = k.card_index(&k.cards[0].id).unwrap();
    let middle = k.card_rect(bounds, a).unwrap().center();
    // A click selects the card.
    click(&mut tools, &doc, at(middle.x, middle.y));
    assert_eq!(tools.selected_card(&doc), Some((id, a)));
    // A drag moves it to the top of the third column.
    let target = k.column_rect(bounds, 2).unwrap().center().x;
    let Some(cmd @ Command::SetCard { .. }) = drag(
        &mut tools,
        &doc,
        at(middle.x, middle.y),
        at(target, middle.y),
    ) else {
        panic!("expected SetCard");
    };
    doc.apply(cmd).unwrap();
    tools.sync(&doc);
    let (k, _) = board(&doc);
    assert_eq!(k.cards_in(0).len(), 1);
    assert_eq!(k.cards_in(2).len(), 1);
    assert_eq!(&*k.cards[k.cards_in(2)[0]].text, "a");
    // It stays selected, and Delete removes it.
    let (_, i) = tools.selected_card(&doc).unwrap();
    assert_eq!(&*k.cards[i].text, "a");
    let cmd = tools.remove_selected_card(&doc).unwrap();
    doc.apply(cmd).unwrap();
    tools.sync(&doc);
    assert_eq!(board(&doc).0.cards.len(), 1);
    assert_eq!(tools.selected_card(&doc), None);
    // Dragging the board itself, away from cards, still moves it.
    let title = board(&doc).0.title_rect(board(&doc).1, 1).unwrap().center();
    let Some(Command::Move { .. }) = drag(
        &mut tools,
        &doc,
        at(title.x, title.y),
        at(title.x + 50.0, title.y),
    ) else {
        panic!("expected Move");
    };
    // Columns come and go, and the board's width with them.
    doc.apply(tools.add_kanban_column(&doc).unwrap()).unwrap();
    let (k, b) = board(&doc);
    assert_eq!((k.columns.len(), b.width()), (4, 960.0));
    doc.apply(tools.remove_kanban_column(&doc).unwrap())
        .unwrap();
    assert_eq!(board(&doc).1.width(), 720.0);
}

// Mind maps

#[test]
fn mind_map_nodes_are_selected_toggled_and_removed() {
    let mut doc = Document::new();
    let size = pb_geom::kurbo::Size::new(60.0, 20.0);
    let map = pb_core::MindMap::new(doc.new_line_id(), "Root", size);
    let root = map.nodes[0].id.clone();
    let child = |id: &str, parent: &str, order: f64| pb_core::Node {
        id: id.into(),
        parent: Some(parent.into()),
        order,
        text: id.into(),
        spans: Default::default(),
        text_size: size,
        collapsed: false,
        left: false,
    };
    let map = map
        .with_node(child("a", &root, 0.0))
        .with_node(child("a1", "a", 0.0))
        .with_node(child("b", &root, 1.0));
    let bounds = Rect::from_origin_size((0.0, 0.0), map.layout().bounds.size());
    let data = ObjectData {
        kind: ObjectKind::MindMap(map.clone()),
        bounds,
        rotation: 0.0,
        style: pb_core::MindMap::style(),
        tags: Default::default(),
        link: None,
    };
    let id = doc
        .apply(Command::Create(vec![data.into()]))
        .unwrap()
        .created[0];
    let current = |doc: &Document| match &doc.get(id).unwrap().data {
        d @ ObjectData {
            kind: ObjectKind::MindMap(m),
            ..
        } => (m.clone(), d.bounds),
        _ => panic!("not a mind map"),
    };
    let mut tools = Tools::default();
    tools.select_only([id]);
    // A click on a node selects it.
    let a = map.index("a").unwrap();
    let a_rect = map.node_rect(bounds, a).unwrap();
    click(&mut tools, &doc, at(a_rect.center().x, a_rect.center().y));
    let stored_a = current(&doc).0.index("a").unwrap();
    assert_eq!(tools.selected_node(&doc), Some((id, stored_a)));
    assert_eq!(tools.node_state(&doc), Some((true, false)));
    // A click on its toggle collapses it; the root stays put.
    let layout = map.layout();
    let toggle = map.toggle_at(layout.find(a).unwrap()).unwrap();
    let toggle = map
        .to_world(&layout, bounds, Rect::from_center_size(toggle, (0.0, 0.0)))
        .center();
    let root_before = map.node_rect(bounds, map.root().unwrap()).unwrap();
    let cmd = click(&mut tools, &doc, at(toggle.x, toggle.y)).unwrap();
    doc.apply(cmd).unwrap();
    tools.sync(&doc);
    let (m, b) = current(&doc);
    assert!(m.nodes[m.index("a").unwrap()].collapsed);
    let root_after = m.node_rect(b, m.root().unwrap()).unwrap();
    assert!((root_before.center() - root_after.center()).hypot() < 1e-6);
    // Deleting the node takes its subtree and selects the parent.
    let cmd = tools.remove_selected_node(&doc).unwrap();
    doc.apply(cmd).unwrap();
    tools.sync(&doc);
    let (m, _) = current(&doc);
    assert_eq!(m.nodes.len(), 2);
    assert_eq!(tools.selected_node(&doc), Some((id, m.root().unwrap())));
    // Deleting the central topic deletes the map.
    let cmd = tools.remove_selected_node(&doc).unwrap();
    assert!(matches!(cmd, Command::Delete(ref ids) if ids == &[id]));
}
