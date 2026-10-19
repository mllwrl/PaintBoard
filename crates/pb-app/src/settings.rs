//! The app's own preferences, kept in its data folder between runs.

use crate::i18n::Lang;

/// File with the chosen theme: `dark` or `light`.
const THEME_FILE: &str = "theme.txt";

/// File with the chosen UI language's code (see [`Lang::code`]).
const LANGUAGE_FILE: &str = "language.txt";

/// The UI language picked last time, `None` if never picked.
pub fn language() -> Option<Lang> {
    let dir = pb_platform::fs::app_data_dir().ok()?;
    let text = pb_platform::fs::read(&dir.join(LANGUAGE_FILE)).ok()?;
    Lang::from_code(String::from_utf8_lossy(&text).trim())
}

/// Remembers the UI language picked.
pub fn save_language(lang: Lang) {
    let saved = pb_platform::fs::app_data_dir().and_then(|dir| {
        pb_platform::fs::write_atomic(&dir.join(LANGUAGE_FILE), lang.code().as_bytes())
    });
    if let Err(err) = saved {
        log::warn!("language not saved: {err}");
    }
}

/// File with the UI's size (e.g. `1.25`) and whether high contrast is on (`contrast`).
const VIEW_FILE: &str = "view.txt";

/// The UI's size and high contrast as picked last time; normal size, no contrast if never.
pub fn view() -> (f32, bool) {
    let text = pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::read(&dir.join(VIEW_FILE)))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let scale = text
        .split_whitespace()
        .find_map(|w| w.parse::<f32>().ok())
        .filter(|s| s.is_finite())
        .unwrap_or(1.0);
    (scale, text.split_whitespace().any(|w| w == "contrast"))
}

/// Remembers the UI's size and high contrast.
pub fn save_view(scale: f32, high_contrast: bool) {
    let text = format!("{scale}{}", if high_contrast { " contrast" } else { "" });
    let saved = pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::write_atomic(&dir.join(VIEW_FILE), text.as_bytes()));
    if let Err(err) = saved {
        log::warn!("view settings not saved: {err}");
    }
}

/// File with the address of the server boards were last shared through.
const SERVER_FILE: &str = "server.txt";

/// The server boards were last shared through; empty if none yet.
pub fn server() -> String {
    pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::read(&dir.join(SERVER_FILE)))
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        .unwrap_or_default()
}

pub fn save_server(server: &str) {
    let saved = pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::write_atomic(&dir.join(SERVER_FILE), server.as_bytes()));
    if let Err(err) = saved {
        log::warn!("server address not saved: {err}");
    }
}

/// The theme picked last time: `Some(true)` for dark, `None` if never picked.
pub fn dark_theme() -> Option<bool> {
    let dir = pb_platform::fs::app_data_dir().ok()?;
    let text = pb_platform::fs::read(&dir.join(THEME_FILE)).ok()?;
    match String::from_utf8_lossy(&text).trim() {
        "dark" => Some(true),
        "light" => Some(false),
        _ => None,
    }
}

/// Remembers the theme picked.
pub fn save_dark_theme(dark: bool) {
    let saved = pb_platform::fs::app_data_dir().and_then(|dir| {
        let text = if dark { "dark" } else { "light" };
        pb_platform::fs::write_atomic(&dir.join(THEME_FILE), text.as_bytes())
    });
    if let Err(err) = saved {
        log::warn!("theme not saved: {err}");
    }
}
