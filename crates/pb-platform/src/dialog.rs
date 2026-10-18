//! Native file and message dialogs. They block until the user answers.
//!
//! TODO(web): use the browser's file picker and downloads; for now dialogs return nothing.

use std::path::{Path, PathBuf};

/// A file type offered in open and save dialogs.
pub struct FileType<'a> {
    pub name: &'a str,
    /// Without the dot.
    pub extension: &'a str,
}

pub fn pick_open(file_type: &FileType) -> Option<PathBuf> {
    imp::pick_open(file_type)
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

    pub fn pick_open(file_type: &FileType) -> Option<PathBuf> {
        FileDialog::new()
            .add_filter(file_type.name, &[file_type.extension])
            .pick_file()
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
    use std::path::{Path, PathBuf};

    use super::{Answer, FileType};

    pub fn pick_open(_file_type: &FileType) -> Option<PathBuf> {
        None
    }

    pub fn pick_open_many(_name: &str, _extensions: &[&str]) -> Vec<PathBuf> {
        Vec::new()
    }

    pub fn ask(_title: &str, _message: &str) -> Answer {
        Answer::No
    }

    /// No dialogs on the web yet: go ahead (what is asked about can be undone).
    pub fn confirm(_title: &str, _message: &str) -> bool {
        true
    }

    pub fn pick_save(_file_type: &FileType, _suggested: &Path) -> Option<PathBuf> {
        None
    }

    pub fn show_error(_title: &str, _message: &str) {}
}
