//! The system clipboard: text, pictures and files copied in a file manager. In the
//! browser it is read only through [`read_then`].

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

/// Gets the clipboard ready to read, then calls `done`: right away on the desktop; in the
/// browser once it has been read (the browser may ask the user first).
pub fn read_then(done: impl FnOnce() + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    done();
    #[cfg(target_arch = "wasm32")]
    imp::read_then(Box::new(done));
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
    //! The browser lets pages read the clipboard only asynchronously (and asks the user
    //! the first time), so [`read_then`](super::read_then) reads it into memory and the
    //! other functions answer from there. Pictures become files, as dropped ones do.

    use std::cell::RefCell;
    use std::path::PathBuf;

    use js_sys::{Array, Uint8Array};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Blob, Clipboard, ClipboardItem};

    use super::ClipboardImage;

    /// What the clipboard held when last read (or was last set to).
    #[derive(Default)]
    struct Contents {
        text: Option<String>,
        files: Vec<PathBuf>,
    }

    thread_local! {
        static CONTENTS: RefCell<Contents> = RefCell::default();
    }

    fn clipboard() -> Option<Clipboard> {
        Some(web_sys::window()?.navigator().clipboard())
    }

    pub fn text() -> Option<String> {
        CONTENTS.with_borrow(|c| c.text.clone())
    }

    pub fn set_text(text: &str) {
        CONTENTS.set(Contents {
            text: Some(text.to_owned()),
            files: Vec::new(),
        });
        if let Some(clipboard) = clipboard() {
            let written = clipboard.write_text(text);
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(err) = JsFuture::from(written).await {
                    log::warn!("could not copy to the clipboard: {err:?}");
                }
            });
        }
    }

    pub fn image() -> Option<ClipboardImage> {
        None
    }

    pub fn files() -> Vec<PathBuf> {
        CONTENTS.with_borrow(|c| c.files.clone())
    }

    pub fn clear() {
        set_text("");
    }

    pub fn read_then(done: Box<dyn FnOnce()>) {
        let Some(clipboard) = clipboard() else {
            done();
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            let contents = match read(&clipboard).await {
                Ok(contents) => contents,
                Err(err) => {
                    // Refused, or a browser without `read`: text may still be allowed.
                    log::info!("clipboard not read whole ({err:?}), trying its text");
                    let text = JsFuture::from(clipboard.read_text()).await.ok();
                    Contents {
                        text: text.and_then(|t| t.as_string()),
                        files: Vec::new(),
                    }
                }
            };
            CONTENTS.set(contents);
            done();
        });
    }

    async fn read(clipboard: &Clipboard) -> Result<Contents, JsValue> {
        let items: Array = JsFuture::from(clipboard.read()).await?.unchecked_into();
        let mut contents = Contents::default();
        for (i, item) in items.iter().enumerate() {
            let item: ClipboardItem = item.unchecked_into();
            for kind in item.types().iter().filter_map(|t| t.as_string()) {
                let Some(extension) = kind.strip_prefix("image/") else {
                    if kind == "text/plain" && contents.text.is_none() {
                        let blob: Blob =
                            JsFuture::from(item.get_type(&kind)).await?.unchecked_into();
                        contents.text = JsFuture::from(blob.text()).await?.as_string();
                    }
                    continue;
                };
                let extension = match extension {
                    "svg+xml" => "svg",
                    "jpeg" => "jpg",
                    other => other,
                };
                let blob: Blob = JsFuture::from(item.get_type(&kind)).await?.unchecked_into();
                let buffer = JsFuture::from(blob.array_buffer()).await?;
                let bytes = Uint8Array::new(&buffer).to_vec();
                let path = crate::fs::uploads_dir().join(format!("pasted-{i}.{extension}"));
                if crate::fs::write_atomic(&path, &bytes).is_ok() {
                    contents.files.push(path);
                }
            }
        }
        Ok(contents)
    }
}
