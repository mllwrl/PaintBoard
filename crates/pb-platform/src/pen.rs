//! What a touch-screen pointer is: a pen (and which of its buttons are down) or a finger.
//! On Windows the window gets pens and fingers as touches with the system's pointer id,
//! which the system can be asked about while the touch is handled. Elsewhere nothing is
//! known, and fingers and pens are told apart by nothing.

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

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use super::{PenInfo, PointerKind};
    use windows::Win32::UI::Input::Pointer::{GetPointerPenInfo, GetPointerType, POINTER_PEN_INFO};
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
            eraser: false,
            barrel: false,
            pressure: 1.0,
            tilt: (0, 0),
        };
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
}

#[cfg(not(windows))]
mod imp {
    pub fn info(_id: u32) -> Option<super::PenInfo> {
        None
    }
}
