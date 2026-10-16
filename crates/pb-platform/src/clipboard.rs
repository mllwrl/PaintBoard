//! The system clipboard, for text.
//!
//! TODO(web): the async browser clipboard API.

/// The clipboard's text, if it holds any.
pub fn text() -> Option<String> {
    imp::text()
}

pub fn set_text(text: &str) {
    imp::set_text(text);
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
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
}

#[cfg(target_arch = "wasm32")]
mod imp {
    pub fn text() -> Option<String> {
        None
    }

    pub fn set_text(_text: &str) {}
}
