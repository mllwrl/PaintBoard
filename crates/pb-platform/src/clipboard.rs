//! The system clipboard: text, pictures and files copied in a file manager.
//!
//! TODO(web): the async browser clipboard API.

use std::path::PathBuf;

/// A picture on the clipboard: straight RGBA pixels, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The clipboard's text, if it holds any.
pub fn text() -> Option<String> {
    imp::text()
}

pub fn set_text(text: &str) {
    imp::set_text(text);
}

/// The clipboard's picture, if it holds one (a screenshot, an image copied in a browser).
pub fn image() -> Option<ClipboardImage> {
    imp::image()
}

/// Files copied in the file manager.
pub fn files() -> Vec<PathBuf> {
    imp::files()
}

/// Empties the clipboard, so content copied inside the app is not shadowed by older system
/// clipboard content when pasting.
pub fn clear() {
    imp::clear();
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::path::PathBuf;

    use super::ClipboardImage;

    pub fn text() -> Option<String> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .inspect_err(|err| log::debug!("no clipboard text: {err}"))
            .ok()
    }

    pub fn set_text(text: &str) {
        if let Err(err) = arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            log::warn!("could not copy to the clipboard: {err}");
        }
    }

    pub fn image() -> Option<ClipboardImage> {
        let image = arboard::Clipboard::new()
            .and_then(|mut c| c.get_image())
            .inspect_err(|err| log::debug!("no clipboard image: {err}"))
            .ok()?;
        Some(ClipboardImage {
            width: u32::try_from(image.width).ok()?,
            height: u32::try_from(image.height).ok()?,
            rgba: image.bytes.into_owned(),
        })
    }

    pub fn files() -> Vec<PathBuf> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.get().file_list())
            .inspect_err(|err| log::debug!("no clipboard files: {err}"))
            .unwrap_or_default()
    }

    pub fn clear() {
        if let Err(err) = arboard::Clipboard::new().and_then(|mut c| c.clear()) {
            log::warn!("could not clear the clipboard: {err}");
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::path::PathBuf;

    use super::ClipboardImage;

    pub fn text() -> Option<String> {
        None
    }

    pub fn set_text(_text: &str) {}

    pub fn image() -> Option<ClipboardImage> {
        None
    }

    pub fn files() -> Vec<PathBuf> {
        Vec::new()
    }

    pub fn clear() {}
}
