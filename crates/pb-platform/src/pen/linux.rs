//! Pens on Linux and the BSDs.
//!
//! Under X11 a tablet's pen comes as a mouse, and winit drops its pressure; the raw
//! XInput 2 events, which a connection of our own to the same server gets too, still have
//! it. Under Wayland a compositor turns a pen into a pointer only for clients that do not
//! ask for tablets; this one asks, so the pen's events come from here ([`next_move`])
//! instead of winit.

use super::{PenInfo, PenMove, PointerKind};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};

/// The id the pen's touches get: neither system names a pointer the way Windows does.
const PEN_ID: u32 = 1;

enum Listener {
    X11(Box<x11::Listener>),
    Wayland(wayland::Listener),
}

thread_local! {
    static LISTENER: RefCell<Option<Listener>> = const { RefCell::new(None) };
    /// What the last pointer event said: a mouse's events (under X11) say `Mouse`.
    static LAST: Cell<Option<PenInfo>> = const { Cell::new(None) };
}

const MOUSE: PenInfo = PenInfo {
    kind: PointerKind::Mouse,
    contact: false,
    eraser: false,
    barrel: false,
    pressure: 1.0,
    tilt: (0, 0),
};

pub fn info(id: u32) -> Option<PenInfo> {
    if id != PEN_ID {
        return None;
    }
    take_x11_events();
    LAST.get()
}

pub fn current() -> Option<(u64, PenInfo)> {
    // Under Wayland the pen does not come as a mouse: every mouse event is a mouse's.
    if !take_x11_events() {
        return None;
    }
    LAST.get()
        .filter(|info| info.kind == PointerKind::Pen)
        .map(|info| (u64::from(PEN_ID), info))
}

pub fn listen() {}

pub fn attach(window: &(impl HasDisplayHandle + HasWindowHandle)) {
    let (Ok(display), Ok(handle)) = (window.display_handle(), window.window_handle()) else {
        return;
    };
    let listener = match (display.as_raw(), handle.as_raw()) {
        (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(handle)) => {
            wayland::Listener::new(display.display, handle.surface).map(Listener::Wayland)
        }
        (RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_), _) => {
            x11::Listener::new().map(|x11| Listener::X11(Box::new(x11)))
        }
        _ => return,
    };
    match listener {
        Ok(listener) => LISTENER.set(Some(listener)),
        Err(err) => log::warn!("pens will act as a mouse, without pressure: {err}"),
    }
}

pub fn next_move() -> Option<PenMove> {
    let next = LISTENER.with_borrow_mut(|listener| match listener {
        Some(Listener::Wayland(wayland)) => wayland.next(),
        _ => None,
    })?;
    LAST.set(Some(next.info));
    Some(PenMove {
        id: u64::from(PEN_ID),
        ..next
    })
}

/// Notes the raw X11 events that came; false when not under X11.
fn take_x11_events() -> bool {
    LISTENER.with_borrow_mut(|listener| {
        let Some(Listener::X11(x11)) = listener else {
            return false;
        };
        if let Err(err) = x11.take_events() {
            log::warn!("pens act as a mouse from now on: {err}");
            *listener = None;
            LAST.set(None);
            return false;
        }
        true
    })
}

mod x11 {
    use super::{LAST, MOUSE, PenInfo, PointerKind};
    use std::collections::HashMap;
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xinput::{self, ConnectionExt as _, DeviceClassData, Fp3232};
    use x11rb::protocol::xproto::ConnectionExt as _;
    use x11rb::rust_connection::RustConnection;

    type Error = Box<dyn std::error::Error>;

    /// Mouse buttons as X11 numbers them: a pen's tip and its side buttons.
    const TIP: u32 = 1;
    const SIDE: [u32; 2] = [2, 3];

    pub struct Listener {
        conn: RustConnection,
        /// The "Abs Pressure" valuator label.
        pressure_label: u32,
        devices: HashMap<u16, Device>,
    }

    /// A pointing device (a slave device in XInput's words).
    #[derive(Default)]
    struct Device {
        /// The pressure valuator's number and range, for a pen (or its eraser end).
        pressure: Option<(u16, f64, f64)>,
        /// The device is a pen's eraser end, as the Wacom driver makes it.
        eraser: bool,
        tip: bool,
        side: bool,
        last_pressure: f64,
    }

    impl Listener {
        pub fn new() -> Result<Self, Error> {
            let (conn, screen) = x11rb::connect(None)?;
            conn.xinput_xi_query_version(2, 2)?.reply()?;
            let pressure_label = conn.intern_atom(false, b"Abs Pressure")?.reply()?.atom;
            let root = conn.setup().roots[screen].root;
            let raw = xinput::XIEventMask::RAW_MOTION
                | xinput::XIEventMask::RAW_BUTTON_PRESS
                | xinput::XIEventMask::RAW_BUTTON_RELEASE;
            // Raw events come to the root window whichever window has the pointer; they are
            // only looked at while the window handles a mouse event of its own.
            conn.xinput_xi_select_events(
                root,
                &[
                    xinput::EventMask {
                        deviceid: xinput::Device::ALL_MASTER.into(),
                        mask: vec![raw],
                    },
                    xinput::EventMask {
                        deviceid: xinput::Device::ALL.into(),
                        mask: vec![xinput::XIEventMask::HIERARCHY],
                    },
                ],
            )?
            .check()?;
            let mut listener = Self {
                conn,
                pressure_label,
                devices: HashMap::new(),
            };
            listener.query_devices()?;
            Ok(listener)
        }

        fn query_devices(&mut self) -> Result<(), Error> {
            let reply = self
                .conn
                .xinput_xi_query_device(xinput::Device::ALL)?
                .reply()?;
            self.devices.clear();
            for info in reply.infos {
                let name = String::from_utf8_lossy(&info.name).to_lowercase();
                let pressure = info.classes.iter().find_map(|class| match &class.data {
                    DeviceClassData::Valuator(v) if v.label == self.pressure_label => {
                        Some((v.number, fixed(v.min), fixed(v.max)))
                    }
                    _ => None,
                });
                let device = Device {
                    pressure,
                    eraser: name.contains("eraser"),
                    ..Device::default()
                };
                self.devices.insert(info.deviceid, device);
            }
            Ok(())
        }

        /// Notes the raw events that came so far.
        pub fn take_events(&mut self) -> Result<(), Error> {
            while let Some(event) = self.conn.poll_for_event()? {
                let (event, press) = match event {
                    Event::XinputRawMotion(event) => (event, None),
                    Event::XinputRawButtonPress(event) => (event, Some(true)),
                    Event::XinputRawButtonRelease(event) => (event, Some(false)),
                    Event::XinputHierarchy(_) => {
                        self.query_devices()?;
                        continue;
                    }
                    _ => continue,
                };
                if !self.devices.contains_key(&event.sourceid) {
                    self.query_devices()?;
                }
                let Some(device) = self.devices.get_mut(&event.sourceid) else {
                    continue;
                };
                let Some((number, min, max)) = device.pressure else {
                    LAST.set(Some(MOUSE));
                    continue;
                };
                if let Some(down) = press {
                    if event.detail == TIP {
                        device.tip = down;
                    } else if SIDE.contains(&event.detail) {
                        device.side = down;
                        // The side buttons click the middle and right mouse buttons, which
                        // work as they are (the right one opens the context menu).
                        LAST.set(Some(MOUSE));
                        continue;
                    }
                }
                if let Some(value) = valuator(&event.valuator_mask, &event.axisvalues, number) {
                    device.last_pressure = if max > min {
                        ((value - min) / (max - min)).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                }
                LAST.set(Some(PenInfo {
                    kind: PointerKind::Pen,
                    contact: device.tip,
                    eraser: device.eraser,
                    barrel: device.side,
                    pressure: device.last_pressure,
                    tilt: (0, 0),
                }));
            }
            Ok(())
        }
    }

    /// Valuator `number`'s value in an event that lists the values of those in `mask`.
    fn valuator(mask: &[u32], values: &[Fp3232], number: u16) -> Option<f64> {
        let number = usize::from(number);
        let bit = |n: usize| {
            mask.get(n / 32)
                .is_some_and(|word| word >> (n % 32) & 1 != 0)
        };
        if !bit(number) {
            return None;
        }
        let index = (0..number).filter(|&n| bit(n)).count();
        values.get(index).map(|&v| fixed(v))
    }

    fn fixed(v: Fp3232) -> f64 {
        f64::from(v.integral) + f64::from(v.frac) / 4_294_967_296.0
    }
}

#[allow(unsafe_code)]
mod wayland {
    use super::{PenInfo, PenMove, PointerKind};
    use std::collections::{HashMap, VecDeque};
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_registry, wl_seat};
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop,
        event_created_child,
    };
    use wayland_protocols::wp::cursor_shape::v1::client::{
        wp_cursor_shape_device_v1::{self, WpCursorShapeDeviceV1},
        wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
    };
    use wayland_protocols::wp::tablet::zv2::client::{
        zwp_tablet_manager_v2::ZwpTabletManagerV2,
        zwp_tablet_pad_group_v2::{self, ZwpTabletPadGroupV2},
        zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2,
        zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2,
        zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
        zwp_tablet_seat_v2::{self, ZwpTabletSeatV2},
        zwp_tablet_tool_v2::{self, ZwpTabletToolV2},
        zwp_tablet_v2::ZwpTabletV2,
    };

    /// Buttons on a pen's side, as Linux numbers them (BTN_STYLUS, BTN_STYLUS2, BTN_STYLUS3).
    const SIDE: [u32; 3] = [0x14b, 0x14c, 0x149];

    pub struct Listener {
        conn: Connection,
        queue: EventQueue<State>,
        state: State,
    }

    struct State {
        /// The window's surface: the pen over another one (the title bar winit draws) is
        /// left alone.
        surface: *mut c_void,
        cursor: Option<WpCursorShapeManagerV1>,
        tools: HashMap<ObjectId, Tool>,
        moves: VecDeque<PenMove>,
    }

    #[derive(Default)]
    struct Tool {
        eraser: bool,
        cursor: Option<WpCursorShapeDeviceV1>,
        /// Over the window, from proximity in to the frame after proximity out.
        near: bool,
        leaving: bool,
        down: bool,
        pos: (f64, f64),
        pressure: f64,
        tilt: (i32, i32),
        side: Vec<u32>,
        /// Something changed since the last frame.
        changed: bool,
    }

    impl Listener {
        pub fn new(
            display: NonNull<c_void>,
            surface: NonNull<c_void>,
        ) -> Result<Self, Box<dyn std::error::Error>> {
            // SAFETY: winit's display stays open as long as the window, which outlives
            // the app's use of pens; this connection never closes it.
            let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
            let conn = Connection::from_backend(backend);
            let (globals, queue) = registry_queue_init::<State>(&conn)?;
            let qh = queue.handle();
            let manager: ZwpTabletManagerV2 = globals.bind(&qh, 1..=1, ())?;
            // Without it the compositor picks the pen's pointer over the window.
            let cursor = globals.bind(&qh, 1..=1, ()).ok();
            for global in globals.contents().clone_list() {
                if global.interface == wl_seat::WlSeat::interface().name {
                    let seat: wl_seat::WlSeat = globals.registry().bind(global.name, 1, &qh, ());
                    manager.get_tablet_seat(&seat, &qh, ());
                }
            }
            conn.flush()?;
            Ok(Self {
                conn,
                queue,
                state: State {
                    surface: surface.as_ptr(),
                    cursor,
                    tools: HashMap::new(),
                    moves: VecDeque::new(),
                },
            })
        }

        /// The next move of a pen over the window, in logical pixels.
        pub fn next(&mut self) -> Option<PenMove> {
            if self.state.moves.is_empty() {
                // winit reads the shared socket; the events it read for this queue wait here.
                if let Err(err) = self.queue.dispatch_pending(&mut self.state) {
                    log::warn!("tablet events: {err}");
                }
                // Cursor shapes asked for while handling them.
                let _ = self.conn.flush();
            }
            self.state.moves.pop_front()
        }
    }

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    delegate_noop!(State: ignore wl_seat::WlSeat);
    delegate_noop!(State: ZwpTabletManagerV2);
    delegate_noop!(State: ignore ZwpTabletV2);
    delegate_noop!(State: ignore ZwpTabletPadRingV2);
    delegate_noop!(State: ignore ZwpTabletPadStripV2);
    delegate_noop!(State: WpCursorShapeManagerV1);
    delegate_noop!(State: WpCursorShapeDeviceV1);

    impl Dispatch<ZwpTabletSeatV2, ()> for State {
        fn event(
            _: &mut Self,
            _: &ZwpTabletSeatV2,
            _: zwp_tablet_seat_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }

        event_created_child!(State, ZwpTabletSeatV2, [
            zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (ZwpTabletV2, ()),
            zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (ZwpTabletToolV2, ()),
            zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (ZwpTabletPadV2, ()),
        ]);
    }

    impl Dispatch<ZwpTabletPadV2, ()> for State {
        fn event(
            _: &mut Self,
            _: &ZwpTabletPadV2,
            _: zwp_tablet_pad_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }

        event_created_child!(State, ZwpTabletPadV2, [
            zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (ZwpTabletPadGroupV2, ()),
        ]);
    }

    impl Dispatch<ZwpTabletPadGroupV2, ()> for State {
        fn event(
            _: &mut Self,
            _: &ZwpTabletPadGroupV2,
            _: zwp_tablet_pad_group_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }

        event_created_child!(State, ZwpTabletPadGroupV2, [
            zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (ZwpTabletPadRingV2, ()),
            zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (ZwpTabletPadStripV2, ()),
        ]);
    }

    impl Dispatch<ZwpTabletToolV2, ()> for State {
        fn event(
            state: &mut Self,
            proxy: &ZwpTabletToolV2,
            event: zwp_tablet_tool_v2::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            use zwp_tablet_tool_v2::Event;
            if let Event::Removed = event {
                state.tools.remove(&proxy.id());
                proxy.destroy();
                return;
            }
            let tool = state.tools.entry(proxy.id()).or_default();
            match event {
                Event::Type { tool_type } => {
                    tool.eraser = tool_type == WEnum::Value(zwp_tablet_tool_v2::Type::Eraser);
                }
                Event::ProximityIn {
                    serial, surface, ..
                } => {
                    tool.near = surface.id().as_ptr().cast() == state.surface;
                    tool.leaving = false;
                    tool.changed = true;
                    if tool.near
                        && let Some(cursor) = &state.cursor
                    {
                        let device = tool
                            .cursor
                            .get_or_insert_with(|| cursor.get_tablet_tool_v2(proxy, qh, ()));
                        device.set_shape(serial, wp_cursor_shape_device_v1::Shape::Default);
                    }
                }
                // Sent once more, as not touching, at the frame.
                Event::ProximityOut => {
                    tool.down = false;
                    tool.side.clear();
                    tool.leaving = true;
                    tool.changed = true;
                }
                Event::Down { .. } => {
                    tool.down = true;
                    tool.changed = true;
                }
                Event::Up => {
                    tool.down = false;
                    tool.changed = true;
                }
                Event::Motion { x, y } => {
                    tool.pos = (x, y);
                    tool.changed = true;
                }
                Event::Pressure { pressure } => {
                    tool.pressure = f64::from(pressure) / 65535.0;
                    tool.changed = true;
                }
                Event::Tilt { tilt_x, tilt_y } => {
                    tool.tilt = (tilt_x.round() as i32, tilt_y.round() as i32);
                }
                Event::Button {
                    button,
                    state: pressed,
                    ..
                } => {
                    tool.side.retain(|&b| b != button);
                    if SIDE.contains(&button)
                        && pressed == WEnum::Value(zwp_tablet_tool_v2::ButtonState::Pressed)
                    {
                        tool.side.push(button);
                    }
                    tool.changed = true;
                }
                Event::Frame { .. } => {
                    if tool.changed && tool.near {
                        state.moves.push_back(PenMove {
                            id: 0,
                            pos: tool.pos,
                            info: PenInfo {
                                kind: PointerKind::Pen,
                                contact: tool.down,
                                eraser: tool.eraser,
                                barrel: !tool.side.is_empty(),
                                pressure: tool.pressure,
                                tilt: tool.tilt,
                            },
                        });
                    }
                    if tool.leaving {
                        tool.near = false;
                        tool.leaving = false;
                    }
                    tool.changed = false;
                }
                _ => {}
            }
        }
    }
}
