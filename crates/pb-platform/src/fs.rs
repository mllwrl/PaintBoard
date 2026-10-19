//! Files on the local disk, or in the browser's own storage on the web.

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

pub fn exists(path: &Path) -> bool {
    imp::exists(path)
}

pub fn remove(path: &Path) -> io::Result<()> {
    imp::remove(path)
}

/// Loads the files kept in the browser; call once before anything reads a file.
#[cfg(target_arch = "wasm32")]
pub async fn init() {
    imp::init().await;
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

    pub fn exists(path: &Path) -> bool {
        path.exists()
    }

    pub fn remove(path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    pub fn temp_dir() -> io::Result<PathBuf> {
        let dir = std::env::temp_dir().join(super::APP_DIR);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    //! The browser has no file system the app can reach synchronously, so files live in
    //! memory: [`init`](super::init) fills it at start from the origin private file system
    //! (OPFS), and writes under the app data directory go back there in the background.
    //! Other paths (temporary files, downloads) are kept in memory only.

    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, HashMap};
    use std::io;
    use std::path::{Path, PathBuf};

    use js_sys::{AsyncIterator, Promise, Reflect, Uint8Array};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        File, FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetDirectoryOptions,
        FileSystemGetFileOptions, FileSystemWritableFileStream,
    };

    thread_local! {
        static FILES: RefCell<HashMap<PathBuf, Vec<u8>>> = RefCell::default();
        /// Changes not yet written to OPFS, by path relative to the app data directory:
        /// new contents, or `None` to remove. Only the latest change of a file is kept.
        static PENDING: RefCell<BTreeMap<String, Option<Vec<u8>>>> = RefCell::default();
        static FLUSHING: Cell<bool> = const { Cell::new(false) };
    }

    fn data_dir() -> PathBuf {
        Path::new("/").join(super::APP_DIR)
    }

    fn not_found(path: &Path) -> io::Error {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} not found", path.display()),
        )
    }

    fn js_error(err: JsValue) -> io::Error {
        io::Error::other(format!("{err:?}"))
    }

    pub fn read(path: &Path) -> io::Result<Vec<u8>> {
        FILES.with_borrow(|files| files.get(path).cloned().ok_or_else(|| not_found(path)))
    }

    pub fn exists(path: &Path) -> bool {
        FILES.with_borrow(|files| files.contains_key(path))
    }

    pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
        FILES.with_borrow_mut(|files| files.insert(path.to_owned(), bytes.to_vec()));
        persist(path, Some(bytes.to_vec()));
        Ok(())
    }

    pub fn remove(path: &Path) -> io::Result<()> {
        FILES
            .with_borrow_mut(|files| files.remove(path))
            .ok_or_else(|| not_found(path))?;
        persist(path, None);
        Ok(())
    }

    pub fn app_data_dir() -> io::Result<PathBuf> {
        Ok(data_dir())
    }

    pub fn temp_dir() -> io::Result<PathBuf> {
        Ok(Path::new("/tmp").join(super::APP_DIR))
    }

    /// Queues the change for OPFS if `path` is under the app data directory.
    fn persist(path: &Path, bytes: Option<Vec<u8>>) {
        let Ok(relative) = path.strip_prefix(data_dir()) else {
            return;
        };
        let key = relative.to_string_lossy().replace('\\', "/");
        PENDING.with_borrow_mut(|pending| pending.insert(key, bytes));
        if !FLUSHING.replace(true) {
            wasm_bindgen_futures::spawn_local(flush());
        }
    }

    /// Writes the queued changes one at a time, so they reach OPFS in order.
    async fn flush() {
        while let Some((key, bytes)) = PENDING.with_borrow_mut(|p| p.pop_first()) {
            if let Err(err) = store(&key, bytes.as_deref()).await {
                log::error!("could not keep {key} in the browser: {err}");
            }
        }
        FLUSHING.set(false);
    }

    async fn root() -> io::Result<FileSystemDirectoryHandle> {
        let window = web_sys::window().ok_or_else(|| io::Error::other("no window"))?;
        let storage = window.navigator().storage();
        let root: FileSystemDirectoryHandle = JsFuture::from(storage.get_directory())
            .await
            .map_err(js_error)?
            .unchecked_into();
        subdir(&root, super::APP_DIR).await
    }

    async fn subdir(
        dir: &FileSystemDirectoryHandle,
        name: &str,
    ) -> io::Result<FileSystemDirectoryHandle> {
        let options = FileSystemGetDirectoryOptions::new();
        options.set_create(true);
        let handle = dir.get_directory_handle_with_options(name, &options);
        Ok(JsFuture::from(handle)
            .await
            .map_err(js_error)?
            .unchecked_into())
    }

    async fn store(key: &str, bytes: Option<&[u8]>) -> io::Result<()> {
        let mut dir = root().await?;
        let mut parts: Vec<&str> = key.split('/').filter(|p| !p.is_empty()).collect();
        let name = parts
            .pop()
            .ok_or_else(|| io::Error::other("empty file name"))?;
        for part in parts {
            dir = subdir(&dir, part).await?;
        }
        let Some(bytes) = bytes else {
            return JsFuture::from(dir.remove_entry(name))
                .await
                .map(drop)
                .map_err(js_error);
        };
        let options = FileSystemGetFileOptions::new();
        options.set_create(true);
        let file: FileSystemFileHandle =
            JsFuture::from(dir.get_file_handle_with_options(name, &options))
                .await
                .map_err(js_error)?
                .unchecked_into();
        // The new contents replace the file only on close, like an atomic rename.
        let stream: FileSystemWritableFileStream = JsFuture::from(file.create_writable())
            .await
            .map_err(js_error)?
            .unchecked_into();
        JsFuture::from(stream.write_with_u8_array(bytes).map_err(js_error)?)
            .await
            .map_err(js_error)?;
        JsFuture::from(stream.close()).await.map_err(js_error)?;
        Ok(())
    }

    pub async fn init() {
        match root().await {
            Ok(dir) => {
                if let Err(err) = load_dir(dir, data_dir()).await {
                    log::error!("could not read the files kept in the browser: {err}");
                }
            }
            Err(err) => log::warn!("no file storage in this browser: {err}"),
        }
    }

    /// Reads every file under `dir` into memory as `path/...`.
    async fn load_dir(dir: FileSystemDirectoryHandle, path: PathBuf) -> io::Result<()> {
        let mut stack = vec![(dir, path)];
        while let Some((dir, path)) = stack.pop() {
            let entries: AsyncIterator = dir.values();
            loop {
                let next: Promise = entries.next().map_err(js_error)?;
                let step = JsFuture::from(next).await.map_err(js_error)?;
                let done = Reflect::get(&step, &"done".into()).map_err(js_error)?;
                if done.as_bool() == Some(true) {
                    break;
                }
                let handle = Reflect::get(&step, &"value".into()).map_err(js_error)?;
                if let Some(sub) = handle.dyn_ref::<FileSystemDirectoryHandle>() {
                    stack.push((sub.clone(), path.join(sub.name())));
                } else if let Some(file) = handle.dyn_ref::<FileSystemFileHandle>() {
                    let blob: File = JsFuture::from(file.get_file())
                        .await
                        .map_err(js_error)?
                        .unchecked_into();
                    let buffer = JsFuture::from(blob.array_buffer())
                        .await
                        .map_err(js_error)?;
                    let bytes = Uint8Array::new(&buffer).to_vec();
                    FILES.with_borrow_mut(|files| files.insert(path.join(file.name()), bytes));
                }
            }
        }
        Ok(())
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
