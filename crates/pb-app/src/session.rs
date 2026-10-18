//! The open board's file: where it lives, restoring it on start and saving it as you go.
//!
//! Like Miro, there is no explicit "unsaved" state: a board with a file is written back to
//! it shortly after every edit, and an untitled board is kept in a recovery file in the app
//! data directory until it is saved under a name.

use std::io;
use std::path::{Path, PathBuf};

use crate::i18n::{tr, trf};

use pb_core::Document;
use pb_platform::dialog::{self, Answer, FileType};
use pb_platform::{Instant, fs};
use pb_storage::{Autosave, EXTENSION, decode, encode};

const FILE_TYPE: FileType = FileType {
    name: "PaintBoard board",
    extension: EXTENSION,
};
/// Where the untitled board is kept, in the app data directory.
const RECOVERY_FILE: &str = "untitled.pboard";
/// Remembers the board to reopen on the next start, in the app data directory.
const LAST_BOARD_FILE: &str = "last-board.txt";
const UNTITLED: &str = "Untitled";

pub struct Session {
    /// The board's file; `None` for an untitled board.
    path: Option<PathBuf>,
    /// App data directory; `None` where there is none (the web, for now), which turns off
    /// keeping untitled boards.
    data_dir: Option<PathBuf>,
    autosave: Autosave,
}

impl Session {
    /// Opens `file` (named on the command line, e.g. by a double click on it), or else
    /// the board of the last run.
    pub fn start(file: Option<PathBuf>) -> (Self, Document) {
        let (mut session, doc) = Self::restore();
        let Some(path) = file.filter(|f| Some(f) != session.path.as_ref()) else {
            return (session, doc);
        };
        match load(&path) {
            Ok(opened) => {
                if session.release(&doc) {
                    session.switch_to(Some(path), &opened);
                    return (session, opened);
                }
            }
            Err(err) => {
                let title = trf("Could not open {}", &[&path.display()]);
                dialog::show_error(&title, &err.to_string());
            }
        }
        (session, doc)
    }

    /// Reopens the board of the last run: its file, or else the untitled board.
    pub fn restore() -> (Self, Document) {
        let data_dir = fs::app_data_dir()
            .inspect_err(|err| log::warn!("no app data directory, autosave is off: {err}"))
            .ok();
        let mut session = Self {
            path: None,
            data_dir,
            autosave: Autosave::new(0),
        };
        let last = session
            .data_file(LAST_BOARD_FILE)
            .and_then(|f| fs::read(&f).ok())
            .map(|bytes| PathBuf::from(String::from_utf8_lossy(&bytes).trim()));
        if let Some(path) = last {
            match load(&path) {
                Ok(doc) => {
                    session.path = Some(path);
                    session.autosave = Autosave::new(doc.revision());
                    return (session, doc);
                }
                Err(err) => log::warn!("could not reopen {}: {err}", path.display()),
            }
        }
        let doc = match session.data_file(RECOVERY_FILE) {
            Some(recovery) if recovery.exists() => load(&recovery).unwrap_or_else(|err| {
                log::warn!("could not restore the untitled board: {err}");
                Document::new()
            }),
            _ => Document::new(),
        };
        session.autosave = Autosave::new(doc.revision());
        (session, doc)
    }

    /// File name for the window title.
    pub fn name(&self) -> String {
        self.path.as_deref().and_then(Path::file_stem).map_or_else(
            || crate::i18n::tr(UNTITLED).to_owned(),
            |s| s.to_string_lossy().into_owned(),
        )
    }

    /// When the next autosave is due.
    pub fn deadline(&self) -> Option<Instant> {
        self.autosave.deadline()
    }

    /// Call after anything that may have changed the document; writes it once due.
    pub fn tick(&mut self, doc: &Document, now: Instant) {
        self.autosave.observe(doc.revision(), now);
        if self.autosave.is_due(now)
            && let Err(err) = self.write(doc)
        {
            log::error!("autosave failed: {err}");
            self.autosave.failed(now);
        }
    }

    /// Writes pending changes right away (before quitting or switching boards).
    pub fn flush(&mut self, doc: &Document) {
        self.autosave.observe(doc.revision(), Instant::now());
        if self.autosave.is_dirty()
            && let Err(err) = self.write(doc)
        {
            dialog::show_error(tr("Could not save the board"), &err.to_string());
        }
    }

    /// Ctrl+S: an untitled board asks for a name, others are written right away.
    pub fn save(&mut self, doc: &Document) {
        if self.path.is_some() {
            self.flush(doc);
        } else {
            self.save_as(doc);
        }
    }

    /// Ctrl+Shift+S. Returns false if the user cancelled or the write failed.
    pub fn save_as(&mut self, doc: &Document) -> bool {
        let suggested = self
            .path
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("{}.{EXTENSION}", crate::i18n::tr(UNTITLED))));
        let Some(path) = dialog::pick_save(&FILE_TYPE, &suggested) else {
            return false;
        };
        if let Err(err) = fs::write_atomic(&path, &encode(doc)) {
            dialog::show_error(tr("Could not save the board"), &err.to_string());
            return false;
        }
        if self.path.is_none()
            && let Some(recovery) = self.data_file(RECOVERY_FILE)
        {
            // The board lives in its own file now.
            let _ = std::fs::remove_file(recovery);
        }
        self.switch_to(Some(path), doc);
        true
    }

    /// Ctrl+O. Returns the opened board, or `None` if nothing was opened.
    pub fn open(&mut self, doc: &Document) -> Option<Document> {
        let path = dialog::pick_open(&FILE_TYPE)?;
        if Some(&path) == self.path.as_ref() {
            return None;
        }
        let opened = match load(&path) {
            Ok(opened) => opened,
            Err(err) => {
                let title = trf("Could not open {}", &[&path.display()]);
                dialog::show_error(&title, &err.to_string());
                return None;
            }
        };
        if !self.release(doc) {
            return None;
        }
        self.switch_to(Some(path), &opened);
        Some(opened)
    }

    /// Ctrl+N. Returns the new board, or `None` if the user cancelled.
    pub fn new_board(&mut self, doc: &Document) -> Option<Document> {
        if !self.release(doc) {
            return None;
        }
        let new = Document::new();
        self.switch_to(None, &new);
        Some(new)
    }

    /// Makes sure the current board is kept before another replaces it. An untitled board
    /// only lives in the recovery file, which the next one reuses, so offer to save it.
    /// Returns false if the user cancelled.
    fn release(&mut self, doc: &Document) -> bool {
        if self.path.is_some() || doc.objects().is_empty() {
            self.flush(doc);
            return true;
        }
        let answer = dialog::ask(
            tr("Save the board?"),
            tr("This board has no file yet. Save it before closing it?"),
        );
        match answer {
            Answer::Yes => self.save_as(doc),
            Answer::No => true,
            Answer::Cancel => false,
        }
    }

    fn switch_to(&mut self, path: Option<PathBuf>, doc: &Document) {
        self.path = path;
        self.autosave = Autosave::new(doc.revision());
        if let Some(last) = self.data_file(LAST_BOARD_FILE) {
            let result = match &self.path {
                Some(path) => fs::write_atomic(&last, path.to_string_lossy().as_bytes()),
                None => match std::fs::remove_file(&last) {
                    Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                    other => other,
                },
            };
            if let Err(err) = result {
                log::warn!("could not remember the open board: {err}");
            }
        }
        if self.path.is_none() {
            // Start the recovery file over for the new untitled board.
            if let Err(err) = self.write(doc) {
                log::warn!("could not write the untitled board: {err}");
            }
        }
    }

    /// Writes the board to its file, or an untitled one to the recovery file.
    fn write(&mut self, doc: &Document) -> io::Result<()> {
        let target = match &self.path {
            Some(path) => path.clone(),
            None => match self.data_file(RECOVERY_FILE) {
                Some(recovery) => recovery,
                // Nowhere to keep it.
                None => {
                    self.autosave.saved(doc.revision());
                    return Ok(());
                }
            },
        };
        fs::write_atomic(&target, &encode(doc))?;
        self.autosave.saved(doc.revision());
        Ok(())
    }

    fn data_file(&self, name: &str) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|dir| dir.join(name))
    }
}

fn load(path: &Path) -> Result<Document, Box<dyn std::error::Error>> {
    Ok(decode(&fs::read(path)?)?)
}
