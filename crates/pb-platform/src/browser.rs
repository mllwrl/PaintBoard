//! Opening web pages, mail addresses and files in the system's default app.

/// Opens `url` in the default browser (or mail app for `mailto:`). Only `http`, `https`
/// and `mailto` addresses are opened; returns whether it was.
pub fn open_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if !["http://", "https://", "mailto:"]
        .iter()
        .any(|s| lower.starts_with(s))
    {
        log::warn!("not opening {url}: unsupported address");
        return false;
    }
    imp::open_url(url)
}

/// The page's own address, in the browser.
#[cfg(target_arch = "wasm32")]
pub fn page_address() -> Option<String> {
    web_sys::window()?.location().href().ok()
}

/// Opens the file at `path` in the app the system uses for its kind; returns whether it
/// was.
/// In the browser, the file (kept in memory) opens in a new tab: a PDF in its viewer,
/// from which it can be printed.
pub fn open_file(path: &std::path::Path) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        imp::open_url(&path.to_string_lossy())
    }
    #[cfg(target_arch = "wasm32")]
    {
        imp::open_file(path)
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};

    pub fn open_url(url: &str) -> bool {
        // SAFETY: both strings are valid, NUL-terminated and outlive the call.
        #[allow(unsafe_code)]
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(url),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        // Values above 32 mean success.
        result.0 as isize > 32
    }
}

#[cfg(all(not(windows), not(target_arch = "wasm32")))]
mod imp {
    pub fn open_url(url: &str) -> bool {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(opener).arg(url).spawn().is_ok()
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::path::Path;

    use web_sys::{Blob, BlobPropertyBag, Url};

    pub fn open_url(url: &str) -> bool {
        web_sys::window()
            .and_then(|w| w.open_with_url_and_target(url, "_blank").ok())
            .flatten()
            .is_some()
    }

    pub fn open_file(path: &Path) -> bool {
        let Ok(bytes) = crate::fs::read(path) else {
            return false;
        };
        let kind = match path.extension().and_then(|e| e.to_str()) {
            Some("pdf") => "application/pdf",
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("svg") => "image/svg+xml",
            _ => "application/octet-stream",
        };
        let options = BlobPropertyBag::new();
        options.set_type(kind);
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
        let Ok(blob) = Blob::new_with_u8_array_sequence_and_options(&parts, &options) else {
            return false;
        };
        // The tab keeps the URL for as long as it is open, so it is never revoked.
        Url::create_object_url_with_blob(&blob).is_ok_and(|url| open_url(&url))
    }
}
