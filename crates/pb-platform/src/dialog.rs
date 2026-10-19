//! File and message dialogs: the system's on the desktop, which block until the user
//! answers, and the browser's on the web.

use std::path::{Path, PathBuf};

/// Texts of the browser's dialogs that the app does not pass in, in the UI's language.
#[derive(Clone, Debug)]
pub struct WebTexts {
    /// Ends the question before closing an unsaved board: what OK does.
    pub ok_saves: String,
    /// Asked when that was cancelled: whether to close the board without saving.
    pub discard: String,
    /// Asks for the name of a board kept in the browser.
    pub board_name: String,
}

impl Default for WebTexts {
    fn default() -> Self {
        Self {
            ok_saves: "OK: save it.".into(),
            discard: "Close the board without saving it?".into(),
            board_name: "Board name".into(),
        }
    }
}

thread_local! {
    static WEB_TEXTS: std::cell::RefCell<WebTexts> = std::cell::RefCell::default();
}

/// Sets the texts of the browser's dialogs (call when the UI's language changes).
pub fn set_web_texts(texts: WebTexts) {
    WEB_TEXTS.set(texts);
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn web_texts() -> WebTexts {
    WEB_TEXTS.with_borrow(Clone::clone)
}

/// A file type offered in open and save dialogs.
pub struct FileType<'a> {
    pub name: &'a str,
    /// Without the dot.
    pub extension: &'a str,
}

/// Asks for any number of files with one of `extensions` (without dots), offered under
/// `name`. Empty if cancelled.
pub fn pick_open_many(name: &str, extensions: &[&str]) -> Vec<PathBuf> {
    imp::pick_open_many(name, extensions)
}

/// Asks where to save, suggesting `suggested` (a file name or a full path).
pub fn pick_save(file_type: &FileType, suggested: &Path) -> Option<PathBuf> {
    imp::pick_save(file_type, suggested)
}

/// Asks for files with one of `extensions` (without dots), several with `many`, and hands
/// them to `done`: right away on the desktop; on the web once the browser has read them,
/// as its file picker does not block. `done` is not called if nothing was picked.
pub fn pick_files(
    name: &str,
    extensions: &[&str],
    many: bool,
    done: impl FnOnce(Vec<PathBuf>) + 'static,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let paths = if many {
            imp::pick_open_many(name, extensions)
        } else {
            imp::pick_one(name, extensions).into_iter().collect()
        };
        if !paths.is_empty() {
            done(paths);
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        imp::pick_files(extensions, many, Box::new(done));
    }
}

/// Calls `dropped` with files dragged onto the page and where (in physical pixels from
/// the window's top left). In the browser only: winit reports drops on the desktop.
#[cfg(target_arch = "wasm32")]
pub fn on_files_dropped(dropped: impl Fn(Vec<PathBuf>, (f64, f64)) + 'static) {
    imp::on_files_dropped(Box::new(dropped));
}

pub fn show_error(title: &str, message: &str) {
    log::error!("{title}: {message}");
    imp::show_error(title, message);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Cancel,
}

/// A question with Yes, No and Cancel buttons.
pub fn ask(title: &str, message: &str) -> Answer {
    imp::ask(title, message)
}

/// A yes-or-no question, e.g. before deleting a lot; true for yes.
pub fn confirm(title: &str, message: &str) -> bool {
    imp::confirm(title, message)
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::path::{Path, PathBuf};

    use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

    use super::{Answer, FileType};

    pub fn pick_one(name: &str, extensions: &[&str]) -> Option<PathBuf> {
        FileDialog::new().add_filter(name, extensions).pick_file()
    }

    pub fn pick_open_many(name: &str, extensions: &[&str]) -> Vec<PathBuf> {
        FileDialog::new()
            .add_filter(name, extensions)
            .pick_files()
            .unwrap_or_default()
    }

    pub fn pick_save(file_type: &FileType, suggested: &Path) -> Option<PathBuf> {
        let mut dialog = FileDialog::new().add_filter(file_type.name, &[file_type.extension]);
        if let Some(dir) = suggested.parent().filter(|d| !d.as_os_str().is_empty()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(name) = suggested.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
        let path = dialog.save_file()?;
        // Some platforms do not add the extension of the chosen filter.
        Some(match path.extension() {
            Some(_) => path,
            None => path.with_extension(file_type.extension),
        })
    }

    pub fn show_error(title: &str, message: &str) {
        MessageDialog::new()
            .set_level(MessageLevel::Error)
            .set_title(title)
            .set_description(message)
            .show();
    }

    pub fn ask(title: &str, message: &str) -> Answer {
        let result = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title(title)
            .set_description(message)
            .set_buttons(MessageButtons::YesNoCancel)
            .show();
        match result {
            MessageDialogResult::Yes => Answer::Yes,
            MessageDialogResult::No => Answer::No,
            _ => Answer::Cancel,
        }
    }

    pub fn confirm(title: &str, message: &str) -> bool {
        let result = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title(title)
            .set_description(message)
            .set_buttons(MessageButtons::YesNo)
            .show();
        result == MessageDialogResult::Yes
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    //! The browser's own dialogs. Boards are saved in the browser's storage under a name;
    //! other files (exports) are downloaded. Files are picked with [`super::pick_files`],
    //! as the browser's file picker does not block.

    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use js_sys::Uint8Array;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{DragEvent, FileList, HtmlInputElement};

    use super::{Answer, FileType};

    fn window() -> Option<web_sys::Window> {
        web_sys::window()
    }

    pub fn pick_open_many(_name: &str, _extensions: &[&str]) -> Vec<PathBuf> {
        Vec::new()
    }

    fn joined(parts: &[&str]) -> String {
        parts.join(PARAGRAPH)
    }

    /// Two line breaks, between a dialog's title and its message.
    const PARAGRAPH: &str = "\n\n";

    pub fn ask(title: &str, message: &str) -> Answer {
        let Some(window) = window() else {
            return Answer::Cancel;
        };
        let texts = super::web_texts();
        // The browser has no Yes/No/Cancel: OK saves, and else a second question tells
        // closing without saving from changing one's mind.
        let save = joined(&[title, message, &texts.ok_saves]);
        if window.confirm_with_message(&save).unwrap_or(false) {
            return Answer::Yes;
        }
        if window.confirm_with_message(&texts.discard).unwrap_or(false) {
            Answer::No
        } else {
            Answer::Cancel
        }
    }

    pub fn confirm(title: &str, message: &str) -> bool {
        window()
            .and_then(|w| w.confirm_with_message(&joined(&[title, message])).ok())
            .unwrap_or(false)
    }

    /// A board is kept in the browser under the name asked for; anything else goes to the
    /// downloads (see `fs::write_atomic`).
    pub fn pick_save(file_type: &FileType, suggested: &Path) -> Option<PathBuf> {
        if file_type.extension != "pboard" {
            let name = suggested.file_name()?;
            return Some(crate::fs::downloads_dir().join(name));
        }
        let stem = suggested
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned());
        let name = window()?
            .prompt_with_message_and_default(
                &super::web_texts().board_name,
                &stem.unwrap_or_default(),
            )
            .ok()??;
        // Characters file systems refuse, should the board be downloaded later.
        let name: String = name
            .trim()
            .chars()
            .map(|c| if r#"/\:*?"<>|"#.contains(c) { '_' } else { c })
            .collect();
        if name.is_empty() {
            return None;
        }
        let dir = crate::fs::boards_dir().ok()?;
        Some(dir.join(format!("{name}.{}", file_type.extension)))
    }

    pub fn show_error(title: &str, message: &str) {
        if let Some(window) = window() {
            let _ = window.alert_with_message(&joined(&[title, message]));
        }
    }

    pub fn pick_files(extensions: &[&str], many: bool, done: Box<dyn FnOnce(Vec<PathBuf>)>) {
        let Some(input) = window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("input").ok())
            .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
        else {
            return;
        };
        input.set_type("file");
        input.set_multiple(many);
        let accept: Vec<String> = extensions.iter().map(|e| format!(".{e}")).collect();
        input.set_accept(&accept.join(","));
        let done = Rc::new(RefCell::new(Some(done)));
        let picked = input.clone();
        let on_change = Closure::<dyn FnMut()>::new(move || {
            let Some(done) = done.borrow_mut().take() else {
                return;
            };
            let files = picked.files();
            wasm_bindgen_futures::spawn_local(async move {
                let paths = read_files(files).await;
                if !paths.is_empty() {
                    done(paths);
                }
            });
        });
        input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
        // The input is never added to the page; the closure lives as long as the page.
        on_change.forget();
        input.click();
    }

    /// Reads `files` into memory (the uploads directory); returns where they went.
    async fn read_files(files: Option<FileList>) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let count = files.as_ref().map_or(0, FileList::length);
        for i in 0..count {
            let Some(file) = files.as_ref().and_then(|f| f.get(i)) else {
                continue;
            };
            match JsFuture::from(file.array_buffer()).await {
                Ok(buffer) => {
                    let path = crate::fs::uploads_dir().join(file.name());
                    let bytes = Uint8Array::new(&buffer).to_vec();
                    match crate::fs::write_atomic(&path, &bytes) {
                        Ok(()) => paths.push(path),
                        Err(err) => log::error!("{}: {err}", file.name()),
                    }
                }
                Err(err) => log::error!("{}: {err:?}", file.name()),
            }
        }
        paths
    }

    pub fn on_files_dropped(dropped: Box<dyn Fn(Vec<PathBuf>, (f64, f64))>) {
        let Some(window) = window() else {
            return;
        };
        // Without this the browser opens a dragged file itself instead of dropping it.
        let over = Closure::<dyn FnMut(DragEvent)>::new(|event: DragEvent| {
            event.prevent_default();
        });
        let dropped = Rc::new(dropped);
        let scale = window.device_pixel_ratio();
        let on_drop = Closure::<dyn FnMut(DragEvent)>::new(move |event: DragEvent| {
            event.prevent_default();
            let at = (
                f64::from(event.client_x()) * scale,
                f64::from(event.client_y()) * scale,
            );
            let files = event.data_transfer().and_then(|d| d.files());
            let dropped = dropped.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let paths = read_files(files).await;
                if !paths.is_empty() {
                    dropped(paths, at);
                }
            });
        });
        let target: &web_sys::EventTarget = window.as_ref();
        let _ = target.add_event_listener_with_callback("dragover", over.as_ref().unchecked_ref());
        let _ = target.add_event_listener_with_callback("drop", on_drop.as_ref().unchecked_ref());
        // Listening as long as the page is open.
        over.forget();
        on_drop.forget();
    }
}
