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

/// Opens the file at `path` in the app the system uses for its kind; returns whether it
/// was.
pub fn open_file(path: &std::path::Path) -> bool {
    imp::open_url(&path.to_string_lossy())
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
    pub fn open_url(url: &str) -> bool {
        log::warn!("opening {url} is not supported in the browser yet");
        false
    }
}
