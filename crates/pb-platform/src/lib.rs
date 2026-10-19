//! Platform abstractions (filesystem, clipboard, time, file dialogs, sound) with native and web
//! implementations. Everything else stays free of direct system calls, so the same code runs
//! on the desktop and in the browser.

pub mod audio;
pub mod browser;
pub mod clipboard;
pub mod dialog;
pub mod fs;
pub mod handwriting;
pub mod net;
pub mod web;

/// A monotonic clock that also works in the browser, where `std::time::Instant` panics.
pub use web_time::{Duration, Instant};

/// The person using the app, as the system knows them: who reactions and comments are
/// by. "Me" when the system does not say.
pub fn user_name() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        for var in ["USERNAME", "USER"] {
            if let Ok(name) = std::env::var(var)
                && !name.trim().is_empty()
            {
                return name.trim().to_owned();
            }
        }
    }
    "Me".to_owned()
}

/// The system's (or the browser's) language and region, like `ru-RU`, if it says.
pub fn locale() -> Option<String> {
    sys_locale::get_locale()
}

/// Milliseconds since the Unix epoch, also in the browser.
pub fn now_ms() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
