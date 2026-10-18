//! Files on the local disk.
//!
//! TODO(web): back these with OPFS or IndexedDB; for now the web build has no files.

use std::io;
use std::path::{Path, PathBuf};

/// Name of the app's own folder inside the platform data directory.
const APP_DIR: &str = "PaintBoard";

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    imp::read(path)
}

/// Replaces the file in one step, so a crash mid-write never leaves a truncated board: the
/// data goes to a temporary file next to it, which is then renamed over the original.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    imp::write_atomic(path, bytes)
}

/// Directory for the app's own files (autosave, settings), created if missing.
pub fn app_data_dir() -> io::Result<PathBuf> {
    imp::app_data_dir()
}

/// Directory for short-lived files (e.g. a PDF to print), created if missing.
pub fn temp_dir() -> io::Result<PathBuf> {
    imp::temp_dir()
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::ffi::OsString;
    use std::fs;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    pub fn read(path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut tmp: OsString = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let result = (|| {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            // Replaces an existing file on every platform, including Windows.
            fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    pub fn app_data_dir() -> io::Result<PathBuf> {
        let env = |name| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let base = if cfg!(windows) {
            env("APPDATA")
        } else if cfg!(target_os = "macos") {
            env("HOME").map(|home| home.join("Library/Application Support"))
        } else {
            env("XDG_DATA_HOME").or_else(|| env("HOME").map(|home| home.join(".local/share")))
        };
        let dir = base
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user data directory"))?
            .join(super::APP_DIR);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn temp_dir() -> io::Result<PathBuf> {
        let dir = std::env::temp_dir().join(super::APP_DIR);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "no file storage on the web yet")
    }

    pub fn read(_path: &Path) -> io::Result<Vec<u8>> {
        Err(unsupported())
    }

    pub fn write_atomic(_path: &Path, _bytes: &[u8]) -> io::Result<()> {
        Err(unsupported())
    }

    pub fn app_data_dir() -> io::Result<PathBuf> {
        let _ = super::APP_DIR;
        Err(unsupported())
    }

    pub fn temp_dir() -> io::Result<PathBuf> {
        Err(unsupported())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("pb-platform-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("board.pboard");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(read(&path).unwrap(), b"second");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["board.pboard"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
