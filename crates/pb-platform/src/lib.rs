//! Platform abstractions (filesystem, clipboard, time, file dialogs) with native and web
//! implementations. Everything else stays free of direct system calls, so the same code runs
//! on the desktop and in the browser.

pub mod dialog;
pub mod fs;
pub mod handwriting;

/// A monotonic clock that also works in the browser, where `std::time::Instant` panics.
pub use web_time::{Duration, Instant};
