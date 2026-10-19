//! The app's own preferences, kept in its data folder between runs.

use crate::i18n::Lang;
use crate::ui::Panels;

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

/// File with the panels put away (see [`Panels::to_words`]).
const PANELS_FILE: &str = "panels.txt";

/// The panels as left last time; all shown if never changed.
pub fn panels() -> Panels {
    pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::read(&dir.join(PANELS_FILE)))
        .map(|bytes| Panels::from_words(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

/// Remembers which panels are put away.
pub fn save_panels(panels: Panels) {
    let saved = pb_platform::fs::app_data_dir().and_then(|dir| {
        pb_platform::fs::write_atomic(&dir.join(PANELS_FILE), panels.to_words().as_bytes())
    });
    if let Err(err) = saved {
        log::warn!("panels not saved: {err}");
    }
}

/// File with the name the user goes by on boards, if they gave one.
const NAME_FILE: &str = "name.txt";

/// The name the user gave, if any (else the system's is used).
pub fn name() -> Option<String> {
    let dir = pb_platform::fs::app_data_dir().ok()?;
    let text = pb_platform::fs::read(&dir.join(NAME_FILE)).ok()?;
    let name = String::from_utf8_lossy(&text).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

pub fn save_name(name: &str) {
    let saved = pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::write_atomic(&dir.join(NAME_FILE), name.as_bytes()));
    if let Err(err) = saved {
        log::warn!("name not saved: {err}");
    }
}

/// File with the links of shared boards, a line each: the board's file (empty for the
/// untitled board), its link, and its link to look (empty if unknown), tab-separated.
/// Kept here rather than in the board, which everyone with a view link gets too.
const SHARED_FILE: &str = "shared-boards.txt";

fn shared_lines() -> Vec<(String, String, String)> {
    let text = pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::read(&dir.join(SHARED_FILE)))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let board = parts.next()?.to_owned();
            let link = parts.next().filter(|l| !l.is_empty())?.to_owned();
            let view = parts.next().unwrap_or_default().to_owned();
            Some((board, link, view))
        })
        .collect()
}

/// The links the board in file `board` (empty: the untitled board) is shared by: to
/// edit or look as joined, and to look, if known.
pub fn shared_link(board: &str) -> Option<(String, Option<String>)> {
    shared_lines()
        .into_iter()
        .find(|(b, ..)| b == board)
        .map(|(_, link, view)| (link, (!view.is_empty()).then_some(view)))
}

/// Remembers (or with `None` forgets) the links the board in file `board` is shared by.
pub fn save_shared_link(board: &str, links: Option<(&str, Option<&str>)>) {
    let mut lines: Vec<_> = shared_lines()
        .into_iter()
        .filter(|(b, ..)| b != board)
        .collect();
    if let Some((link, view)) = links {
        lines.push((
            board.to_owned(),
            link.to_owned(),
            view.unwrap_or_default().to_owned(),
        ));
    }
    let text: String = lines
        .iter()
        .map(|(b, l, v)| format!("{b}\t{l}\t{v}\n"))
        .collect();
    let saved = pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::write_atomic(&dir.join(SHARED_FILE), text.as_bytes()));
    if let Err(err) = saved {
        log::warn!("shared board not remembered: {err}");
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
