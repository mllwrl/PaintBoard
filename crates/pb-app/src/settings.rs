//! The app's own preferences, kept in its data folder between runs.

/// File with the chosen theme: `dark` or `light`.
const THEME_FILE: &str = "theme.txt";

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
