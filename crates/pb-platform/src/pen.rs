//! What a touch-screen pointer is: a pen (and which of its buttons are down) or a finger.
//!
//! On Windows the window gets pens and fingers as touches with the system's pointer id,
//! which the system can be asked about while the touch is handled. In the browser fingers
//! come as touches and pens as a mouse; a listener on the page notes each pointer event
//! (its kind and buttons) just before the window gets it. macOS also hands a tablet's pen
//! over as a mouse, with its pressure in the mouse event, which winit drops; a monitor of
//! the app's events notes it the same way. Elsewhere nothing is known.

/// The kind of thing touching or hovering over the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Pen,
    Touch,
    Mouse,
}

/// A pointer as the system reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenInfo {
    pub kind: PointerKind,
    /// It touches the screen (a pen's tip or eraser end).
    pub contact: bool,
    /// The pen is turned over (its eraser end toward the screen) or its eraser button is
    /// pressed.
    pub eraser: bool,
    /// The button on the pen's side (barrel) is pressed.
    pub barrel: bool,
    /// How hard the pen presses, 0 to 1.
    pub pressure: f64,
    /// Tilt in degrees, sideways and toward the user.
    pub tilt: (i32, i32),
}

/// What the system says about pointer `id` (a touch's id), if it knows it.
pub fn info(id: u64) -> Option<PenInfo> {
    let id = u32::try_from(id).ok()?;
    imp::info(id)
}

/// In the browser and on macOS, which hand pens over as a mouse: the pen (its pointer id and state)
/// behind the mouse event being handled, if it is one. `None` elsewhere.
pub fn current() -> Option<(u64, PenInfo)> {
    imp::current()
}

/// Starts noting pointer events, in the browser and on macOS (after the event loop is
/// made); does nothing elsewhere.
pub fn listen() {
    imp::listen();
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use super::{PenInfo, PointerKind};
    use windows::Win32::UI::Input::Pointer::{
        GetPointerInfo, GetPointerPenInfo, GetPointerType, POINTER_FLAG_INCONTACT, POINTER_INFO,
        POINTER_PEN_INFO,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, POINTER_INPUT_TYPE, PT_MOUSE, PT_PEN,
        PT_TOUCH,
    };

    pub fn info(id: u32) -> Option<PenInfo> {
        let mut kind = POINTER_INPUT_TYPE::default();
        // SAFETY: `kind` is a valid place for the answer for the whole call.
        unsafe { GetPointerType(id, &mut kind) }.ok()?;
        let kind = match kind {
            PT_PEN => PointerKind::Pen,
            PT_TOUCH => PointerKind::Touch,
            PT_MOUSE => PointerKind::Mouse,
            _ => return None,
        };
        let mut info = PenInfo {
            kind,
            contact: false,
            eraser: false,
            barrel: false,
            pressure: 1.0,
            tilt: (0, 0),
        };
        let mut pointer = POINTER_INFO::default();
        // SAFETY: `pointer` is a valid, writable POINTER_INFO for the whole call.
        if unsafe { GetPointerInfo(id, &mut pointer) }.is_ok() {
            info.contact = pointer.pointerFlags.0 & POINTER_FLAG_INCONTACT.0 != 0;
        }
        if kind == PointerKind::Pen {
            let mut pen = POINTER_PEN_INFO::default();
            // SAFETY: `pen` is a valid, writable POINTER_PEN_INFO for the whole call.
            if unsafe { GetPointerPenInfo(id, &mut pen) }.is_ok() {
                let flags = pen.penFlags;
                info.eraser = flags & (PEN_FLAG_INVERTED | PEN_FLAG_ERASER) != 0;
                info.barrel = flags & PEN_FLAG_BARREL != 0;
                info.pressure = f64::from(pen.pressure.min(1024)) / 1024.0;
                info.tilt = (pen.tiltX, pen.tiltY);
            }
        }
        Some(info)
    }

    pub fn current() -> Option<(u64, PenInfo)> {
        None
    }

    pub fn listen() {}
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{PenInfo, PointerKind};
    use std::cell::Cell;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    /// Buttons as Pointer Events number them.
    const TIP: u16 = 1;
    const BARREL: u16 = 2;
    const ERASER: u16 = 32;

    thread_local! {
        /// The last pointer event: its pointer id and what it said.
        static LAST: Cell<Option<(u32, PenInfo)>> = const { Cell::new(None) };
        static LISTENING: Cell<bool> = const { Cell::new(false) };
    }

    pub fn info(id: u32) -> Option<PenInfo> {
        LAST.get()
            .filter(|(last, _)| *last == id)
            .map(|(_, info)| info)
    }

    pub fn current() -> Option<(u64, PenInfo)> {
        LAST.get()
            .filter(|(_, info)| info.kind == PointerKind::Pen)
            .map(|(id, info)| (u64::from(id), info))
    }

    pub fn listen() {
        if LISTENING.replace(true) {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        let note = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(note);
        // Caught on the way down to the canvas, so noted before the window handles it.
        for event in [
            "pointerdown",
            "pointermove",
            "pointerup",
            "pointercancel",
            "pointerover",
        ] {
            let added = window.add_event_listener_with_callback_and_bool(
                event,
                note.as_ref().unchecked_ref(),
                true,
            );
            if added.is_err() {
                log::warn!("pens will act as a mouse: no {event} listener");
            }
        }
        // Lives as long as the page.
        note.forget();
    }

    fn note(event: web_sys::PointerEvent) {
        let kind = match event.pointer_type().as_str() {
            "pen" => PointerKind::Pen,
            "touch" => PointerKind::Touch,
            _ => PointerKind::Mouse,
        };
        let buttons = event.buttons();
        let pressure = event.pressure();
        // Some browsers report a pen touching with its side button held as the side button
        // alone; a pen hovering presses with 0.
        let contact = buttons & (TIP | ERASER) != 0 || buttons & BARREL != 0 && pressure > 0.0;
        let info = PenInfo {
            kind,
            contact,
            eraser: buttons & ERASER != 0,
            barrel: buttons & BARREL != 0,
            pressure: f64::from(pressure),
            tilt: (event.tilt_x(), event.tilt_y()),
        };
        if let Ok(id) = u32::try_from(event.pointer_id()) {
            LAST.set(Some((id, info)));
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use super::{PenInfo, PointerKind};
    use block2::RcBlock;
    use objc2_app_kit::{
        NSEvent, NSEventButtonMask, NSEventMask, NSEventSubtype, NSEventType, NSPointingDeviceType,
    };
    use std::cell::Cell;
    use std::ptr::NonNull;

    /// The id the pen's touches get: macOS names no pointer in mouse events.
    const PEN_ID: u32 = 1;

    thread_local! {
        /// The last mouse or tablet event: what it said (a mouse's events say `Mouse`).
        static LAST: Cell<Option<PenInfo>> = const { Cell::new(None) };
        /// The pen's eraser end is the one near the tablet, as its last proximity event said.
        static ERASER: Cell<bool> = const { Cell::new(false) };
        static LISTENING: Cell<bool> = const { Cell::new(false) };
    }

    pub fn info(id: u32) -> Option<PenInfo> {
        LAST.get().filter(|_| id == PEN_ID)
    }

    pub fn current() -> Option<(u64, PenInfo)> {
        LAST.get()
            .filter(|info| info.kind == PointerKind::Pen)
            .map(|info| (u64::from(PEN_ID), info))
    }

    pub fn listen() {
        if LISTENING.replace(true) {
            return;
        }
        // The right and other buttons stay a mouse's: a pen's side button clicks the right
        // one by default, which opens the context menu as it is.
        let mask = NSEventMask::LeftMouseDown
            | NSEventMask::LeftMouseUp
            | NSEventMask::LeftMouseDragged
            | NSEventMask::MouseMoved
            | NSEventMask::RightMouseDown
            | NSEventMask::OtherMouseDown
            | NSEventMask::TabletPoint
            | NSEventMask::TabletProximity;
        let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit hands over a live event for the length of the call.
            note(unsafe { event.as_ref() });
            event.as_ptr()
        });
        // SAFETY: the handler gives back the event it gets, unchanged, as AppKit wants.
        // The monitor sees events as they are taken from the queue, before the window
        // (and winit) gets them.
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
        match monitor {
            // Lives as long as the app.
            Some(monitor) => std::mem::forget(monitor),
            None => log::warn!("pens will act as a mouse: no event monitor"),
        }
    }

    fn note(event: &NSEvent) {
        let kind = event.r#type();
        let subtype = event.subtype();
        let proximity =
            kind == NSEventType::TabletProximity || subtype == NSEventSubtype::TabletProximity;
        if proximity {
            ERASER.set(
                event.isEnteringProximity()
                    && event.pointingDeviceType() == NSPointingDeviceType::Eraser,
            );
            if kind == NSEventType::TabletProximity {
                return;
            }
        }
        let tablet = kind == NSEventType::TabletPoint || subtype == NSEventSubtype::TabletPoint;
        let pen_button =
            kind == NSEventType::RightMouseDown || kind == NSEventType::OtherMouseDown;
        if !tablet || pen_button {
            LAST.set(Some(PenInfo {
                kind: PointerKind::Mouse,
                contact: false,
                eraser: false,
                barrel: false,
                pressure: 1.0,
                tilt: (0, 0),
            }));
            return;
        }
        let contact = match kind {
            NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged => true,
            NSEventType::LeftMouseUp | NSEventType::MouseMoved => false,
            // A tablet point between mouse events: touching as the last one said.
            _ => LAST.get().is_some_and(|last| last.contact),
        };
        let buttons = event.buttonMask();
        let tilt = event.tilt();
        // Tilt comes as -1 to 1 each way: a pen lying flat is 1.
        let degrees = |t: f64| (t.clamp(-1.0, 1.0) * 90.0).round() as i32;
        LAST.set(Some(PenInfo {
            kind: PointerKind::Pen,
            contact,
            eraser: ERASER.get(),
            barrel: buttons
                .intersects(NSEventButtonMask::PenLowerSide | NSEventButtonMask::PenUpperSide),
            pressure: f64::from(event.pressure()).clamp(0.0, 1.0),
            tilt: (degrees(tilt.x), degrees(tilt.y)),
        }));
    }
}

#[cfg(not(any(windows, target_os = "macos", target_arch = "wasm32")))]
mod imp {
    pub fn info(_id: u32) -> Option<super::PenInfo> {
        None
    }

    pub fn current() -> Option<(u64, super::PenInfo)> {
        None
    }

    pub fn listen() {}
}
