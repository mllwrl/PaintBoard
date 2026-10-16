use std::collections::HashMap;
use std::sync::Arc;

use pb_core::{Command, Document, Object, ShapeKind, ZOrder};
use pb_geom::Camera;
use pb_geom::kurbo::{Point, Size, Vec2};
use pb_platform::Instant;
use pb_render::vello::Scene;
use pb_render::{Gpu, GridStyle, Overlay, Theme, View, build_scene};
use pb_tools::{
    Align, Axis, Clipboard, CursorHint, Pointer, ToolKind, Tools, rotation_knob, selection_frame,
    unit_frame,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::navigation::Navigation;
use crate::session::Session;
use crate::ui::{Action, Model, SelectionInfo, Ui};

pub enum UserEvent {
    /// GPU initialization finished (asynchronous on the web).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    GpuReady(Gpu),
}

pub struct App {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    scene: Scene,
    theme: Theme,
    camera: Camera,
    grid: GridStyle,
    navigation: Navigation,
    document: Document,
    tools: Tools,
    clipboard: Clipboard,
    session: Session,
    ui: Option<Ui>,
    /// A left-button drag started on the canvas: its events go to the tools even when the
    /// pointer passes over the UI.
    canvas_drag: bool,
    /// A style slider is being dragged, and its changes form one undo step.
    style_drag: bool,
    /// Pressure of a pen touching the screen or tablet; `None` for a mouse.
    pen_pressure: Option<f64>,
    shown_title: String,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let (session, document) = Session::restore();
        Self {
            proxy,
            window: None,
            gpu: None,
            scene: Scene::new(),
            theme: Theme::light(),
            camera: Camera::default(),
            grid: GridStyle::default(),
            navigation: Navigation::default(),
            document,
            tools: Tools::default(),
            clipboard: Clipboard::default(),
            session,
            ui: None,
            canvas_drag: false,
            style_drag: false,
            pen_pressure: None,
            shown_title: String::new(),
        }
    }

    fn scale(&self) -> f64 {
        self.window.as_ref().map_or(1.0, |w| w.scale_factor())
    }

    fn viewport(&self) -> Size {
        self.window.as_ref().map_or(Size::ZERO, |w| {
            let size = w.inner_size().to_logical::<f64>(w.scale_factor());
            Size::new(size.width, size.height)
        })
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if self.gpu.is_none() {
            return;
        }
        let scale = window.scale_factor();
        let size = window.inner_size().to_logical::<f64>(scale);
        let overrides = self.tools.overrides(&self.document);
        let previews = self.tools.previews(&self.document);
        let overlay = overlay(&self.document, &self.tools, &overrides, self.camera.zoom());
        build_scene(
            &mut self.scene,
            &View {
                camera: &self.camera,
                viewport: Size::new(size.width, size.height),
                scale,
                grid: self.grid,
                theme: &self.theme,
                objects: self.document.objects(),
                overrides: &overrides,
                hidden: self.tools.hidden(),
                previews: &previews,
                overlay: &overlay,
            },
        );

        let board = self.session.name();
        let model = Model {
            board: &board,
            tool: self.tools.active(),
            last_shape: self.tools.last_shape(),
            ink: self
                .tools
                .ink_style(self.tools.active() == ToolKind::Marker),
            can_undo: self.document.can_undo(),
            can_redo: self.document.can_redo(),
            zoom: self.camera.zoom(),
            grid: self.grid,
            snap: self.tools.snap_to_grid(),
            selection: self.selection_info(),
        };
        let actions = match &mut self.ui {
            Some(ui) => ui.run(&window, &model),
            None => Vec::new(),
        };
        if let Some(gpu) = &mut self.gpu {
            let ui = &mut self.ui;
            gpu.render(&self.scene, self.theme.background, |target| {
                if let Some(ui) = ui {
                    ui.paint(target);
                }
            });
        }

        let title = format!("{board} — PaintBoard");
        if self.shown_title != title {
            window.set_title(&title);
            self.shown_title = title;
        }

        if !actions.is_empty() {
            for action in actions {
                self.perform(action);
            }
            self.request_redraw();
        }
    }

    /// What the selection toolbar needs; nothing while a gesture is in progress.
    fn selection_info(&self) -> Option<SelectionInfo> {
        if self.tools.is_busy() || self.tools.active() != ToolKind::Select {
            return None;
        }
        let selection = self.tools.selection();
        let ids = selection.ids();
        let bounds = selection.frame(&self.document)?.aabb();
        let first = ids.iter().flat_map(|&id| self.document.leaves(id)).next()?;
        let corner = |x, y| self.camera.world_to_screen(Point::new(x, y));
        Some(SelectionInfo {
            screen: pb_geom::kurbo::Rect::from_points(
                corner(bounds.x0, bounds.y0),
                corner(bounds.x1, bounds.y1),
            ),
            style: first.data.style,
            has_group: ids.iter().any(|&id| self.document.group(id).is_some()),
            has_fill: ids
                .iter()
                .flat_map(|&id| self.document.leaves(id))
                .any(|o| !o.data.kind.is_ink()),
            several: ids.len() > 1,
            locked: selection.has_locked(&self.document),
        })
    }

    /// Carries out a UI action.
    fn perform(&mut self, action: Action) {
        let viewport = self.viewport();
        match action {
            Action::Tool(tool) => self.tools.set_active(tool),
            Action::InkStyle { marker, style } => self.tools.set_ink_style(marker, style),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::NewBoard => self.new_board(),
            Action::Open => self.open_board(),
            Action::Save => self.session.save(&self.document),
            Action::SaveAs => {
                self.session.save_as(&self.document);
            }
            Action::ZoomIn => self.navigation.zoom_in(&mut self.camera, viewport),
            Action::ZoomOut => self.navigation.zoom_out(&mut self.camera, viewport),
            Action::ZoomReset => self.navigation.zoom_reset(&mut self.camera, viewport),
            Action::NextGrid => self.grid = self.grid.next(),
            Action::ToggleSnap => self.tools.set_snap_to_grid(!self.tools.snap_to_grid()),
            Action::SetStyle(change) => {
                let ids = self.tools.selection().ids();
                let ids: Vec<_> = ids
                    .iter()
                    .copied()
                    .filter(|&id| !self.document.is_locked(id))
                    .collect();
                if !ids.is_empty() {
                    self.apply(Command::SetStyle { ids, change });
                }
            }
            Action::BeginStyleDrag => {
                if !self.style_drag {
                    self.document.begin_undo_group();
                    self.style_drag = true;
                }
            }
            Action::EndStyleDrag => self.end_style_drag(),
            Action::Group => self.apply_with(Tools::group_selected),
            Action::Ungroup => self.apply_with(Tools::ungroup_selected),
            Action::ToggleLock => self.apply_with(Tools::toggle_lock_selected),
            Action::Duplicate => {
                self.apply_with(|tools, doc| pb_tools::duplicate(doc, tools.selection()));
            }
            Action::Delete => self.apply_with(Tools::delete_selected),
            Action::Reorder(to) => self.reorder(to),
        }
    }

    fn end_style_drag(&mut self) {
        if self.style_drag {
            self.document.end_undo_group();
            self.style_drag = false;
        }
    }

    fn open_board(&mut self) {
        self.end_style_drag();
        if let Some(document) = self.session.open(&self.document) {
            self.replace_document(document);
        }
    }

    fn new_board(&mut self) {
        self.end_style_drag();
        if let Some(document) = self.session.new_board(&self.document) {
            self.replace_document(document);
        }
    }

    /// The pointer for tools, if it is over the canvas.
    fn pointer(&self) -> Option<Pointer> {
        let cursor = self.navigation.cursor()?;
        Some(Pointer {
            world: self.camera.screen_to_world(cursor),
            zoom: self.camera.zoom(),
            visible: self.camera.visible_world_rect(self.viewport()),
            pressure: self.pen_pressure.unwrap_or(1.0),
            shift: self.navigation.modifiers().shift_key(),
            ctrl: self.navigation.modifiers().control_key(),
            alt: self.navigation.modifiers().alt_key(),
        })
    }

    fn apply(&mut self, command: Command) {
        match self.document.apply(command) {
            // New objects get selected, like in Miro, and so do the parts of an ungrouped group.
            // Ink is not: the pen stays active for the next stroke.
            Ok(applied) if !applied.created.is_empty() => {
                if !self.tools.active().draws_ink() {
                    self.tools.select_only(applied.created);
                }
            }
            Ok(applied) if !applied.ungrouped.is_empty() => {
                self.tools.select_only(applied.ungrouped)
            }
            Ok(_) => {}
            Err(err) => log::error!("command failed: {err}"),
        }
        self.tools.sync(&self.document);
    }

    fn undo(&mut self) {
        self.end_style_drag();
        self.tools.cancel();
        if let Err(err) = self.document.undo() {
            log::error!("undo failed: {err}");
        }
        self.tools.sync(&self.document);
    }

    fn redo(&mut self) {
        self.tools.cancel();
        if let Err(err) = self.document.redo() {
            log::error!("redo failed: {err}");
        }
        self.tools.sync(&self.document);
    }

    /// Returns true if the view changed.
    fn handle_left_button(&mut self, state: ElementState) -> bool {
        let Some(pointer) = self.pointer() else {
            return false;
        };
        match state {
            ElementState::Pressed => self.tools.pointer_down(&self.document, pointer),
            ElementState::Released => match self.tools.pointer_up(&self.document, pointer) {
                Some(command) => {
                    self.apply(command);
                    true
                }
                // The gesture may still have changed the selection.
                None => true,
            },
        }
    }

    fn cursor_icon(&self) -> CursorIcon {
        match self.navigation.cursor_icon() {
            CursorIcon::Default => match self.tools.cursor() {
                CursorHint::Default => CursorIcon::Default,
                CursorHint::Crosshair => CursorIcon::Crosshair,
                CursorHint::Move => CursorIcon::Move,
                CursorHint::Copy => CursorIcon::Copy,
                CursorHint::Rotate => CursorIcon::Grab,
                CursorHint::ResizeNs => CursorIcon::NsResize,
                CursorHint::ResizeEw => CursorIcon::EwResize,
                CursorHint::ResizeNwse => CursorIcon::NwseResize,
                CursorHint::ResizeNesw => CursorIcon::NeswResize,
            },
            icon => icon,
        }
    }

    /// Applies a command built from the current selection, if there is one.
    fn apply_with(&mut self, build: impl FnOnce(&Tools, &Document) -> Option<Command>) {
        if let Some(command) = build(&self.tools, &self.document) {
            self.apply(command);
        }
    }

    /// Alt shortcuts for aligning and distributing (Miro's layout).
    fn handle_arrange_key(&mut self, code: KeyCode, shift: bool) -> bool {
        let align =
            |to| move |tools: &Tools, doc: &Document| pb_tools::align(doc, tools.selection(), to);
        let distribute = |axis| {
            move |tools: &Tools, doc: &Document| pb_tools::distribute(doc, tools.selection(), axis)
        };
        match code {
            KeyCode::KeyH if shift => self.apply_with(distribute(Axis::Horizontal)),
            KeyCode::KeyV if shift => self.apply_with(distribute(Axis::Vertical)),
            KeyCode::KeyA => self.apply_with(align(Align::Left)),
            KeyCode::KeyD => self.apply_with(align(Align::Right)),
            KeyCode::KeyW => self.apply_with(align(Align::Top)),
            KeyCode::KeyS => self.apply_with(align(Align::Bottom)),
            KeyCode::KeyH => self.apply_with(align(Align::CenterX)),
            KeyCode::KeyV => self.apply_with(align(Align::CenterY)),
            _ => return false,
        }
        true
    }

    fn reorder(&mut self, to: ZOrder) {
        let ids = self.tools.selection().ids().to_vec();
        if !ids.is_empty() {
            self.apply(Command::Reorder { ids, to });
        }
    }

    fn nudge(&mut self, dx: f64, dy: f64) {
        // Shift moves in bigger steps, like in Miro.
        let step = if self.navigation.modifiers().shift_key() {
            10.0
        } else {
            1.0
        };
        if let Some(command) = self
            .tools
            .nudge_selected(&self.document, Vec2::new(dx, dy) * step)
        {
            self.apply(command);
        }
    }

    /// Shows another board (opened or new) from a clean slate.
    fn replace_document(&mut self, document: Document) {
        self.document = document;
        self.tools.cancel();
        self.tools.clear_selection();
        self.tools.sync(&self.document);
        let viewport = self.viewport();
        self.camera.look_at(Point::ZERO, 1.0, viewport);
    }

    fn on_gpu_ready(&mut self, gpu: Gpu) {
        if let Some(ui) = &mut self.ui {
            ui.attach_gpu(&gpu);
        }
        self.gpu = Some(gpu);
        self.request_redraw();
    }

    fn handle_key(&mut self, event: &KeyEvent) -> bool {
        let PhysicalKey::Code(code) = event.physical_key else {
            return false;
        };
        if code == KeyCode::Space {
            self.navigation
                .set_space(event.state == ElementState::Pressed);
            return false;
        }
        if event.state != ElementState::Pressed {
            return false;
        }
        let modifiers = self.navigation.modifiers();
        let ctrl = modifiers.control_key();
        if modifiers.alt_key() && !ctrl {
            return self.handle_arrange_key(code, modifiers.shift_key());
        }
        let viewport = self.viewport();
        // Physical keys, so shortcuts work the same on any keyboard layout.
        match code {
            KeyCode::KeyS if ctrl && modifiers.shift_key() => {
                self.session.save_as(&self.document);
            }
            KeyCode::KeyS if ctrl => self.session.save(&self.document),
            KeyCode::KeyO if ctrl => self.open_board(),
            KeyCode::KeyN if ctrl => self.new_board(),
            KeyCode::KeyZ if ctrl && modifiers.shift_key() => self.redo(),
            KeyCode::KeyZ if ctrl => self.undo(),
            KeyCode::KeyY if ctrl => self.redo(),
            KeyCode::KeyV if !ctrl => self.tools.set_active(ToolKind::Select),
            KeyCode::KeyR if !ctrl => self.tools.set_active(ToolKind::Shape(ShapeKind::Rectangle)),
            KeyCode::KeyO if !ctrl => self.tools.set_active(ToolKind::Shape(ShapeKind::Ellipse)),
            KeyCode::KeyS if !ctrl => self.tools.cycle_shape(),
            KeyCode::KeyP if !ctrl => self.tools.set_active(ToolKind::Pen),
            KeyCode::KeyM if !ctrl => self.tools.set_active(ToolKind::Marker),
            KeyCode::KeyE if !ctrl => self.tools.set_active(ToolKind::Eraser),
            KeyCode::KeyA if ctrl => self.tools.select_all(&self.document),
            KeyCode::KeyC if ctrl => {
                self.clipboard.copy(&self.document, self.tools.selection());
            }
            KeyCode::KeyX if ctrl => {
                if self.clipboard.copy(&self.document, self.tools.selection())
                    && let Some(command) = self.tools.delete_selected(&self.document)
                {
                    self.apply(command);
                }
            }
            KeyCode::KeyV if ctrl => {
                // Paste where the pointer is, like Miro; cascade when it is off the canvas.
                let at = self
                    .navigation
                    .hovered_cursor()
                    .map(|c| self.camera.screen_to_world(c));
                if let Some(command) = self.clipboard.paste(at) {
                    self.apply(command);
                }
            }
            KeyCode::KeyG if ctrl && modifiers.shift_key() => {
                self.apply_with(Tools::ungroup_selected);
            }
            KeyCode::KeyG if ctrl => self.apply_with(Tools::group_selected),
            KeyCode::KeyL if ctrl && modifiers.shift_key() => {
                self.apply_with(Tools::toggle_lock_selected);
            }
            KeyCode::KeyD if ctrl => {
                if let Some(command) = pb_tools::duplicate(&self.document, self.tools.selection()) {
                    self.apply(command);
                }
            }
            KeyCode::BracketRight if ctrl => self.reorder(if modifiers.shift_key() {
                ZOrder::Front
            } else {
                ZOrder::Forward
            }),
            KeyCode::BracketLeft if ctrl => self.reorder(if modifiers.shift_key() {
                ZOrder::Back
            } else {
                ZOrder::Backward
            }),
            KeyCode::Delete | KeyCode::Backspace => {
                if let Some(command) = self.tools.delete_selected(&self.document) {
                    self.apply(command);
                }
            }
            KeyCode::ArrowLeft => self.nudge(-1.0, 0.0),
            KeyCode::ArrowRight => self.nudge(1.0, 0.0),
            KeyCode::ArrowUp => self.nudge(0.0, -1.0),
            KeyCode::ArrowDown => self.nudge(0.0, 1.0),
            // Esc backs out one level: gesture, then tool, then selection.
            KeyCode::Escape => {
                if !self.tools.cancel() {
                    if self.tools.active() != ToolKind::Select {
                        self.tools.set_active(ToolKind::Select);
                    } else {
                        self.tools.clear_selection();
                    }
                }
            }
            KeyCode::Equal | KeyCode::NumpadAdd if ctrl => {
                self.navigation.zoom_in(&mut self.camera, viewport);
            }
            KeyCode::Minus | KeyCode::NumpadSubtract if ctrl => {
                self.navigation.zoom_out(&mut self.camera, viewport);
            }
            KeyCode::Digit0 | KeyCode::Numpad0 if ctrl => {
                self.navigation.zoom_reset(&mut self.camera, viewport);
            }
            KeyCode::Home => self.camera.look_at(Point::ZERO, 1.0, viewport),
            KeyCode::KeyG if modifiers.shift_key() && !ctrl && !event.repeat => {
                self.tools.set_snap_to_grid(!self.tools.snap_to_grid());
            }
            KeyCode::KeyG if !ctrl && !event.repeat => self.grid = self.grid.next(),
            _ => return false,
        }
        true
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("PaintBoard")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        #[cfg(target_arch = "wasm32")]
        let attributes = {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes.with_append(true)
        };
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create window"),
        );
        self.window = Some(window.clone());
        self.ui = Some(Ui::new(&window));
        self.camera.look_at(Point::ZERO, 1.0, self.viewport());

        let size = window.inner_size();
        let gpu = Gpu::new(window, size.width, size.height);

        #[cfg(not(target_arch = "wasm32"))]
        match pollster::block_on(gpu) {
            Ok(gpu) => self.on_gpu_ready(gpu),
            Err(err) => {
                log::error!("failed to initialize GPU: {err}");
                event_loop.exit();
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            let proxy = self.proxy.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match gpu.await {
                    Ok(gpu) => {
                        let _ = proxy.send_event(UserEvent::GpuReady(gpu));
                    }
                    Err(err) => log::error!("failed to initialize GPU (WebGPU required): {err}"),
                }
            });
        }
    }

    /// Runs after each batch of events: autosaves when due and wakes up for the next one.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.session.tick(&self.document, now);
        let ui_repaint = self.ui.as_ref().and_then(Ui::repaint_at);
        if ui_repaint.is_some_and(|at| at <= now) {
            self.request_redraw();
        }
        let wake = [self.session.deadline(), ui_repaint.filter(|&at| at > now)]
            .into_iter()
            .flatten()
            .min();
        event_loop.set_control_flow(match wake {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.session.flush(&self.document);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::GpuReady(gpu) => self.on_gpu_ready(gpu),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let scale = self.scale();
        let mut ui_consumed = false;
        if let (Some(ui), Some(window)) = (&mut self.ui, &self.window) {
            let response = ui.on_window_event(window, &event);
            if response.repaint {
                window.request_redraw();
            }
            ui_consumed = response.consumed;
        }
        let over_ui = self.ui.as_ref().is_some_and(Ui::wants_pointer) && !self.canvas_drag;
        let changed = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                false
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                false
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                true
            }
            WindowEvent::ScaleFactorChanged { .. } => true,
            WindowEvent::ModifiersChanged(modifiers) => {
                self.navigation.set_modifiers(modifiers.state());
                // Shift changes the shape being drawn without moving the mouse.
                self.pointer()
                    .is_some_and(|p| self.tools.pointer_move(&self.document, p))
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let typing = self.ui.as_ref().is_some_and(Ui::wants_keyboard);
                !typing && self.handle_key(&event)
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos = position.to_logical::<f64>(scale);
                let panned = self
                    .navigation
                    .cursor_moved(&mut self.camera, Point::new(pos.x, pos.y));
                let tool_changed = if over_ui {
                    self.tools.clear_hover()
                } else {
                    self.pointer()
                        .is_some_and(|p| self.tools.pointer_move(&self.document, p))
                };
                panned || tool_changed
            }
            WindowEvent::CursorLeft { .. } => {
                self.navigation.cursor_left();
                false
            }
            WindowEvent::MouseInput { state, button, .. } => {
                // Presses on the UI stay there; releases always reach the canvas so that a
                // drag started on it ends even over a toolbar.
                if state == ElementState::Pressed && (ui_consumed || over_ui) {
                    false
                } else {
                    let consumed = self.navigation.mouse_button(button, state);
                    let left = button == MouseButton::Left;
                    if left {
                        self.canvas_drag = state == ElementState::Pressed && !consumed;
                    }
                    !consumed && left && self.handle_left_button(state)
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if ui_consumed || over_ui {
                    false
                } else {
                    self.navigation.wheel(&mut self.camera, delta, scale);
                    true
                }
            }
            // Pens report pressure as touch force; the strokes themselves come in as mouse
            // events, which Windows also sends for pens.
            WindowEvent::Touch(touch) => {
                self.pen_pressure = match touch.phase {
                    TouchPhase::Ended | TouchPhase::Cancelled => None,
                    _ => touch.force.map(|f| f.normalized()),
                };
                false
            }
            WindowEvent::PinchGesture { delta, .. } => {
                self.navigation.pinch(&mut self.camera, delta);
                true
            }
            _ => false,
        };
        // Over the UI, egui sets the cursor.
        if let Some(window) = &self.window
            && !over_ui
        {
            window.set_cursor(self.cursor_icon());
        }
        if changed {
            self.request_redraw();
        }
    }
}

/// Selection feedback, following objects mid-gesture.
fn overlay(document: &Document, tools: &Tools, overrides: &[Object], zoom: f64) -> Overlay {
    let overrides: HashMap<_, _> = overrides.iter().map(|o| (o.id, &o.data)).collect();
    let frame = |o: &Object| overrides.get(&o.id).copied().unwrap_or(&o.data).frame();
    let ids = tools.selection().ids();
    let selection = selection_frame(document, ids, frame);
    let selected = match ids {
        [id] if document.group(*id).is_some() => {
            document.leaves(*id).into_iter().map(frame).collect()
        }
        [_] => Vec::new(),
        _ => ids
            .iter()
            .filter_map(|&id| unit_frame(document, id, frame))
            .collect(),
    };
    let show_handles = tools.shows_handles(document);
    Overlay {
        selected,
        selection,
        show_handles,
        locked: tools.selection().has_locked(document),
        rotate_knob: selection
            .filter(|_| show_handles)
            .map(|f| rotation_knob(f, zoom)),
        hover: tools.hover().and_then(|id| unit_frame(document, id, frame)),
        marquee: tools.marquee(),
        guides: tools.guides(document),
        eraser: tools.eraser(),
    }
}
