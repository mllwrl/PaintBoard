//! Toolbars and the properties panel, drawn with egui on top of the canvas.
//!
//! The UI only reads a [`Model`] of the app and reports what the user did as [`Action`]s; the
//! app carries them out, the same way as keyboard shortcuts.

use std::sync::Arc;

use egui::{
    Align2, Area, Button, Color32, Context, FontData, FontDefinitions, FontFamily, Frame, Id,
    Order, Painter, Pos2, Rect, Response, Sense, Shadow, Shape, Slider, Stroke, Vec2,
};
use pb_core::{Rgba, ShapeKind, Style, StyleChange, ZOrder};
use pb_geom::kurbo::{self, PathEl};
use pb_platform::Instant;
use pb_render::{FrameTarget, Gpu, GridStyle};
use pb_tools::ToolKind;
use winit::window::Window;

/// Something the user asked for through the UI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Tool(ToolKind),
    Undo,
    Redo,
    NewBoard,
    Open,
    Save,
    SaveAs,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    NextGrid,
    ToggleSnap,
    SetStyle(StyleChange),
    /// A slider drag starts: the style changes until [`Action::EndStyleDrag`] are one undo step.
    BeginStyleDrag,
    EndStyleDrag,
    Group,
    Ungroup,
    ToggleLock,
    Duplicate,
    Delete,
    Reorder(ZOrder),
}

/// What the UI shows.
pub struct Model<'a> {
    pub board: &'a str,
    pub tool: ToolKind,
    pub last_shape: ShapeKind,
    pub can_undo: bool,
    pub can_redo: bool,
    pub zoom: f64,
    pub grid: GridStyle,
    pub snap: bool,
    /// Hidden while a gesture is in progress.
    pub selection: Option<SelectionInfo>,
}

pub struct SelectionInfo {
    /// Selection bounds on screen, in logical pixels.
    pub screen: kurbo::Rect,
    /// Style of the first selected object.
    pub style: Style,
    pub has_group: bool,
    pub several: bool,
    pub locked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Popup {
    Fill,
    Stroke,
    Opacity,
}

/// Colors offered for fill and outline. The last entry is "none".
const PALETTE: [Rgba; 16] = [
    Rgba::rgb(0xFF, 0xFF, 0xFF),
    Rgba::rgb(0xE6, 0xE6, 0xE6),
    Rgba::rgb(0x80, 0x80, 0x80),
    Rgba::rgb(0x1F, 0x29, 0x37),
    Rgba::rgb(0xFF, 0xF1, 0x76),
    Rgba::rgb(0xFF, 0xB7, 0x4D),
    Rgba::rgb(0xF2, 0x55, 0x55),
    Rgba::rgb(0xF4, 0x8F, 0xB1),
    Rgba::rgb(0xB3, 0x9D, 0xDB),
    Rgba::rgb(0x5C, 0x6B, 0xF0),
    Rgba::rgb(0x81, 0xD4, 0xFA),
    Rgba::rgb(0x4D, 0xD0, 0xC4),
    Rgba::rgb(0x66, 0xBB, 0x6A),
    Rgba::rgb(0xC5, 0xE1, 0xA5),
    Rgba::rgb(0x8D, 0x6E, 0x63),
    Rgba(0),
];

const ACCENT: Color32 = Color32::from_rgb(0x3B, 0x6E, 0xF5);
const INK: Color32 = Color32::from_rgb(0x1F, 0x29, 0x37);
const BUTTON: f32 = 36.0;
const MARGIN: f32 = 12.0;
/// Gap between the selection and its toolbar; leaves room for the rotation knob.
const SELECTION_GAP: f32 = 44.0;

pub struct Ui {
    ctx: Context,
    state: egui_winit::State,
    renderer: Option<egui_wgpu::Renderer>,
    /// Texture changes not yet uploaded (kept when a frame could not be drawn).
    textures: egui::TexturesDelta,
    paint_jobs: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
    popup: Option<Popup>,
    repaint_at: Option<Instant>,
}

impl Ui {
    pub fn new(window: &Window) -> Self {
        let ctx = Context::default();
        ctx.set_fonts(fonts());
        ctx.set_visuals(egui::Visuals::light());
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        Self {
            ctx,
            state,
            renderer: None,
            textures: egui::TexturesDelta::default(),
            paint_jobs: Vec::new(),
            pixels_per_point: 1.0,
            popup: None,
            repaint_at: None,
        }
    }

    pub fn attach_gpu(&mut self, gpu: &Gpu) {
        let max_side = gpu.device().limits().max_texture_dimension_2d as usize;
        self.state.set_max_texture_side(max_side);
        self.renderer = Some(egui_wgpu::Renderer::new(
            gpu.device(),
            gpu.surface_format(),
            egui_wgpu::RendererOptions::default(),
        ));
    }

    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &winit::event::WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    /// The pointer is over the UI or dragging one of its widgets.
    pub fn wants_pointer(&self) -> bool {
        self.ctx.is_pointer_over_egui() || self.ctx.egui_is_using_pointer()
    }

    /// A text field has focus, so keys are not shortcuts.
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// When the UI wants to be drawn again (animations, tooltips).
    pub fn repaint_at(&self) -> Option<Instant> {
        self.repaint_at
    }

    /// Lays out the UI for this frame and returns what the user did.
    pub fn run(&mut self, window: &Window, model: &Model) -> Vec<Action> {
        let input = self.state.take_egui_input(window);
        let mut actions = Vec::new();
        let popup = &mut self.popup;
        let output = self.ctx.run_pass(input, |ctx| {
            board_menu(ctx, model, &mut actions);
            toolbar(ctx, model, &mut actions);
            view_controls(ctx, model, &mut actions);
            if let Some(selection) = &model.selection {
                selection_toolbar(ctx, selection, popup, &mut actions);
            } else {
                *popup = None;
            }
        });
        self.state
            .handle_platform_output(window, output.platform_output);
        self.textures.append(output.textures_delta);
        self.pixels_per_point = output.pixels_per_point;
        self.paint_jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.repaint_at = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .and_then(|v| Instant::now().checked_add(v.repaint_delay));
        actions
    }

    /// Draws the UI laid out by the last [`Ui::run`] into the frame.
    pub fn paint(&mut self, target: FrameTarget<'_>) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let mut textures = std::mem::take(&mut self.textures);
        for (id, deltas) in &textures.set {
            for delta in deltas {
                renderer.update_texture(target.device, target.queue, *id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: target.size,
            pixels_per_point: self.pixels_per_point,
        };
        // Callback command buffers would only come from custom paint callbacks; there are none.
        let _ = renderer.update_buffers(
            target.device,
            target.queue,
            target.encoder,
            &self.paint_jobs,
            &screen,
        );
        let pass = target
            .encoder
            .begin_render_pass(&egui_wgpu::wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(egui_wgpu::wgpu::RenderPassColorAttachment {
                    view: target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: egui_wgpu::wgpu::Operations {
                        load: egui_wgpu::wgpu::LoadOp::Load,
                        store: egui_wgpu::wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        renderer.render(&mut pass.forget_lifetime(), &self.paint_jobs, &screen);
        for id in &textures.free {
            renderer.free_texture(id);
        }
        // Applied above; egui checks that deltas are not dropped by accident.
        textures.clear();
    }
}

/// Inter (SIL Open Font License) instead of egui's built-in fonts, whose licenses we have
/// not cleared.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    fonts.font_data.insert(
        "inter".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.insert(family, vec!["inter".to_owned()]);
    }
    fonts
}

/// White rounded panel with a soft shadow, like Miro's floating toolbars.
fn panel() -> Frame {
    Frame::new()
        .fill(Color32::WHITE)
        .corner_radius(10)
        .inner_margin(6)
        .stroke(Stroke::new(1.0, Color32::from_gray(225)))
        .shadow(Shadow {
            offset: [0, 2],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(30),
        })
}

fn board_menu(ctx: &Context, model: &Model, actions: &mut Vec<Action>) {
    Area::new(Id::unique("board"))
        .fixed_pos(Pos2::new(MARGIN, MARGIN))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.strong(model.board);
                    ui.separator();
                    ui.menu_button("File", |ui| {
                        let items = [
                            ("New board", "Ctrl+N", Action::NewBoard),
                            ("Open…", "Ctrl+O", Action::Open),
                            ("Save", "Ctrl+S", Action::Save),
                            ("Save as…", "Ctrl+Shift+S", Action::SaveAs),
                        ];
                        for (label, shortcut, action) in items {
                            if ui.add(Button::new(label).shortcut_text(shortcut)).clicked() {
                                actions.push(action);
                                ui.close();
                            }
                        }
                    });
                });
            });
        });
}

fn toolbar(ctx: &Context, model: &Model, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    let shapes_button = Area::new(Id::unique("tools"))
        .fixed_pos(Pos2::new(MARGIN, screen.center().y))
        .pivot(Align2::LEFT_CENTER)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel()
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                    let select = model.tool == ToolKind::Select;
                    if icon_button(ui, select, "Select (V)", paint_cursor).clicked() {
                        actions.push(Action::Tool(ToolKind::Select));
                    }
                    let shape_active = matches!(model.tool, ToolKind::Shape(_));
                    let shape = match model.tool {
                        ToolKind::Shape(shape) => shape,
                        ToolKind::Select => model.last_shape,
                    };
                    let shapes = icon_button(ui, shape_active, "Shapes (S, R, O)", |p, r, c| {
                        paint_shape(p, r, c, shape)
                    });
                    if shapes.clicked() && !shape_active {
                        actions.push(Action::Tool(ToolKind::Shape(shape)));
                    }
                    ui.separator();
                    let undo = ui.add_enabled_ui(model.can_undo, |ui| {
                        icon_button(ui, false, "Undo (Ctrl+Z)", |p, r, c| {
                            paint_turn(p, r, c, false)
                        })
                    });
                    if undo.inner.clicked() {
                        actions.push(Action::Undo);
                    }
                    let redo = ui.add_enabled_ui(model.can_redo, |ui| {
                        icon_button(ui, false, "Redo (Ctrl+Y)", |p, r, c| {
                            paint_turn(p, r, c, true)
                        })
                    });
                    if redo.inner.clicked() {
                        actions.push(Action::Redo);
                    }
                    shapes.rect
                })
                .inner
        })
        .inner;

    // Like Miro, the shape tool shows its shapes next to the toolbar.
    if let ToolKind::Shape(active) = model.tool {
        Area::new(Id::unique("shapes"))
            .fixed_pos(Pos2::new(
                shapes_button.right() + 18.0,
                shapes_button.top() - 8.0,
            ))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                panel().show(ui, |ui| {
                    egui::Grid::new("shape grid")
                        .spacing(Vec2::splat(2.0))
                        .show(ui, |ui| {
                            for (i, kind) in ShapeKind::ALL.into_iter().enumerate() {
                                let button =
                                    icon_button(ui, kind == active, kind.name(), |p, r, c| {
                                        paint_shape(p, r, c, kind)
                                    });
                                if button.clicked() {
                                    actions.push(Action::Tool(ToolKind::Shape(kind)));
                                }
                                if i % 3 == 2 {
                                    ui.end_row();
                                }
                            }
                        });
                });
            });
    }
}

fn view_controls(ctx: &Context, model: &Model, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    Area::new(Id::unique("view"))
        .fixed_pos(screen.max - Vec2::splat(MARGIN))
        .pivot(Align2::RIGHT_BOTTOM)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .button(model.grid.name())
                        .on_hover_text("Grid style (G)")
                        .clicked()
                    {
                        actions.push(Action::NextGrid);
                    }
                    if ui
                        .selectable_label(model.snap, "Snap to grid")
                        .on_hover_text("Shift+G")
                        .clicked()
                    {
                        actions.push(Action::ToggleSnap);
                    }
                    ui.separator();
                    if ui.button("−").on_hover_text("Zoom out (Ctrl+−)").clicked() {
                        actions.push(Action::ZoomOut);
                    }
                    let zoom = format!("{:.0}%", model.zoom * 100.0);
                    if ui
                        .button(zoom)
                        .on_hover_text("Reset zoom (Ctrl+0)")
                        .clicked()
                    {
                        actions.push(Action::ZoomReset);
                    }
                    if ui.button("+").on_hover_text("Zoom in (Ctrl+=)").clicked() {
                        actions.push(Action::ZoomIn);
                    }
                });
            });
        });
}

/// Properties and actions for the selection, floating above it.
fn selection_toolbar(
    ctx: &Context,
    selection: &SelectionInfo,
    popup: &mut Option<Popup>,
    actions: &mut Vec<Action>,
) {
    let r = selection.screen;
    let center = r.center().x as f32;
    // Above the selection, or below it when there is no room at the top.
    let (anchor, pivot) = if (r.y0 as f32) - SELECTION_GAP > 90.0 {
        (
            Pos2::new(center, r.y0 as f32 - SELECTION_GAP),
            Align2::CENTER_BOTTOM,
        )
    } else {
        (
            Pos2::new(center, r.y1 as f32 + SELECTION_GAP / 2.0),
            Align2::CENTER_TOP,
        )
    };
    let bar = Area::new(Id::unique("selection"))
        .fixed_pos(anchor)
        .pivot(pivot)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| {
                ui.horizontal(|ui| {
                    if selection.locked {
                        if ui.button("Unlock").on_hover_text("Ctrl+Shift+L").clicked() {
                            actions.push(Action::ToggleLock);
                        }
                        return;
                    }
                    let style = selection.style;
                    let fill = swatch_button(ui, style.fill, false, "Fill color");
                    toggle(popup, Popup::Fill, fill.clicked());
                    let stroke = swatch_button(ui, style.stroke, true, "Outline");
                    toggle(popup, Popup::Stroke, stroke.clicked());
                    let opacity = format!("{:.0}%", style.opacity * 100.0);
                    let opacity = ui.button(opacity).on_hover_text("Opacity");
                    toggle(popup, Popup::Opacity, opacity.clicked());
                    ui.separator();
                    if selection.several && ui.button("Group").on_hover_text("Ctrl+G").clicked() {
                        actions.push(Action::Group);
                    }
                    if selection.has_group
                        && ui.button("Ungroup").on_hover_text("Ctrl+Shift+G").clicked()
                    {
                        actions.push(Action::Ungroup);
                    }
                    if ui.button("Lock").on_hover_text("Ctrl+Shift+L").clicked() {
                        actions.push(Action::ToggleLock);
                    }
                    ui.separator();
                    if ui.button("Front").on_hover_text("Ctrl+Shift+]").clicked() {
                        actions.push(Action::Reorder(ZOrder::Front));
                    }
                    if ui.button("Back").on_hover_text("Ctrl+Shift+[").clicked() {
                        actions.push(Action::Reorder(ZOrder::Back));
                    }
                    ui.separator();
                    if ui.button("Duplicate").on_hover_text("Ctrl+D").clicked() {
                        actions.push(Action::Duplicate);
                    }
                    if ui.button("Delete").on_hover_text("Delete").clicked() {
                        actions.push(Action::Delete);
                    }
                });
            });
        })
        .response;

    if selection.locked {
        *popup = None;
    }
    // A press anywhere else (the canvas, other toolbars) closes the popup.
    let pressed_outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .pointer_interact_pos()
            .is_some_and(|p| !bar.rect.contains(p));
    let Some(open) = *popup else {
        return;
    };
    let popup_area = Area::new(Id::unique("selection popup"))
        .fixed_pos(Pos2::new(bar.rect.left(), bar.rect.bottom() + 6.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            panel().show(ui, |ui| match open {
                Popup::Fill => {
                    if let Some(color) = palette(ui, selection.style.fill) {
                        actions.push(Action::SetStyle(StyleChange {
                            fill: Some(color),
                            ..StyleChange::default()
                        }));
                    }
                }
                Popup::Stroke => {
                    if let Some(color) = palette(ui, selection.style.stroke) {
                        actions.push(Action::SetStyle(StyleChange {
                            stroke: Some(color),
                            ..StyleChange::default()
                        }));
                    }
                    ui.add_space(4.0);
                    let width = selection.style.stroke_width;
                    style_slider(ui, width, 0.0..=20.0, " px", actions, |v| StyleChange {
                        stroke_width: Some(v),
                        ..StyleChange::default()
                    });
                }
                Popup::Opacity => {
                    let percent = selection.style.opacity * 100.0;
                    style_slider(ui, percent, 0.0..=100.0, "%", actions, |v| StyleChange {
                        opacity: Some(v / 100.0),
                        ..StyleChange::default()
                    });
                }
            });
        })
        .response;
    if pressed_outside
        && ctx
            .pointer_interact_pos()
            .is_some_and(|p| !popup_area.rect.contains(p))
    {
        *popup = None;
    }
}

fn toggle(popup: &mut Option<Popup>, which: Popup, clicked: bool) {
    if clicked {
        *popup = if *popup == Some(which) {
            None
        } else {
            Some(which)
        };
    }
}

/// A slider whose drag is a single undo step.
fn style_slider(
    ui: &mut egui::Ui,
    mut value: f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    actions: &mut Vec<Action>,
    change: impl Fn(f64) -> StyleChange,
) {
    let response = ui.add(Slider::new(&mut value, range).suffix(suffix));
    if response.drag_started() {
        actions.push(Action::BeginStyleDrag);
    }
    if response.changed() {
        actions.push(Action::SetStyle(change(value)));
    }
    if response.drag_stopped() {
        actions.push(Action::EndStyleDrag);
    }
}

/// Grid of color swatches; returns the picked color.
fn palette(ui: &mut egui::Ui, current: Rgba) -> Option<Rgba> {
    let mut picked = None;
    egui::Grid::new("palette")
        .spacing(Vec2::splat(4.0))
        .show(ui, |ui| {
            for (i, color) in PALETTE.into_iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                paint_swatch(ui.painter(), rect, color);
                if color == current {
                    ui.painter().rect_stroke(
                        rect.expand(2.0),
                        6,
                        Stroke::new(2.0, ACCENT),
                        egui::StrokeKind::Outside,
                    );
                }
                let name = if color.to_bytes()[3] == 0 { "None" } else { "" };
                let response = if name.is_empty() {
                    response
                } else {
                    response.on_hover_text(name)
                };
                if response.clicked() {
                    picked = Some(color);
                }
                if i % 4 == 3 {
                    ui.end_row();
                }
            }
        });
    picked
}

fn swatch_button(ui: &mut egui::Ui, color: Rgba, ring: bool, tooltip: &str) -> Response {
    icon_button(ui, false, tooltip, |painter, rect, _| {
        if ring {
            let stroke = Stroke::new(4.0, to_color32(color));
            painter.circle_stroke(rect.center(), rect.width() / 2.0 - 2.0, stroke);
            painter.circle_stroke(
                rect.center(),
                rect.width() / 2.0,
                Stroke::new(1.0, Color32::from_gray(200)),
            );
        } else {
            paint_swatch(painter, rect, color);
        }
    })
}

fn paint_swatch(painter: &Painter, rect: Rect, color: Rgba) {
    if color.to_bytes()[3] == 0 {
        painter.rect_filled(rect, 4, Color32::WHITE);
        let slash = [rect.left_bottom(), rect.right_top()];
        painter.line_segment(slash, Stroke::new(1.5, Color32::from_rgb(0xF2, 0x55, 0x55)));
    } else {
        painter.rect_filled(rect, 4, to_color32(color));
    }
    painter.rect_stroke(
        rect,
        4,
        Stroke::new(1.0, Color32::from_gray(200)),
        egui::StrokeKind::Inside,
    );
}

/// A square toolbar button with a painted icon.
fn icon_button(
    ui: &mut egui::Ui,
    selected: bool,
    tooltip: &str,
    paint: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
    let painter = ui.painter();
    let enabled = ui.is_enabled();
    if selected {
        painter.rect_filled(rect, 8, ACCENT.gamma_multiply(0.15));
    } else if response.hovered() && enabled {
        painter.rect_filled(rect, 8, Color32::from_gray(238));
    }
    let color = if selected {
        ACCENT
    } else if enabled {
        INK
    } else {
        Color32::from_gray(190)
    };
    paint(painter, rect.shrink(9.0), color);
    response.on_hover_text(tooltip)
}

fn paint_cursor(painter: &Painter, rect: Rect, color: Color32) {
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * rect.width();
    let points = vec![
        at(0.15, 0.0),
        at(0.85, 0.62),
        at(0.5, 0.65),
        at(0.3, 1.0),
        at(0.15, 0.0),
    ];
    painter.add(Shape::line(points, Stroke::new(1.6, color)));
}

fn paint_shape(painter: &Painter, rect: Rect, color: Color32, kind: ShapeKind) {
    let bounds = kurbo::Rect::new(
        rect.left().into(),
        rect.top().into(),
        rect.right().into(),
        rect.bottom().into(),
    );
    let mut points = Vec::new();
    kurbo::flatten(kind.outline(bounds), 0.2, |el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => points.push(Pos2::new(p.x as f32, p.y as f32)),
        _ => {}
    });
    painter.add(Shape::closed_line(points, Stroke::new(1.6, color)));
}

/// A curved arrow for undo (counter-clockwise) and redo.
fn paint_turn(painter: &Painter, rect: Rect, color: Color32, redo: bool) {
    let c = rect.center() + Vec2::new(0.0, rect.height() * 0.15);
    let r = rect.width() * 0.4;
    let mirror = if redo { -1.0 } else { 1.0 };
    let at = |angle: f32| c + Vec2::new(mirror * r * angle.cos(), r * angle.sin());
    let arc: Vec<_> = (0..=16)
        .map(|i| at(std::f32::consts::PI * (1.0 + i as f32 / 16.0)))
        .collect();
    let start = arc[0];
    painter.add(Shape::line(arc, Stroke::new(1.6, color)));
    let head = [
        start + Vec2::new(-mirror * 4.0, -2.0),
        start + Vec2::new(0.0, 4.5),
        start + Vec2::new(mirror * 4.0, -2.0),
    ];
    painter.add(Shape::line(head.to_vec(), Stroke::new(1.6, color)));
}

fn to_color32(color: Rgba) -> Color32 {
    let [r, g, b, a] = color.to_bytes();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}
