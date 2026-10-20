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

/// The links a board is shared by, as text: the one this copy joined with, and, when it
/// made the board, those to look and to comment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SharedLinks {
    pub link: String,
    pub view: Option<String>,
    pub comment: Option<String>,
}

fn shared_lines() -> Vec<(String, SharedLinks)> {
    let text = pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::read(&dir.join(SHARED_FILE)))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let board = parts.next()?.to_owned();
            let link = parts.next().filter(|l| !l.is_empty())?.to_owned();
            let mut next = || parts.next().filter(|l| !l.is_empty()).map(str::to_owned);
            let (view, comment) = (next(), next());
            Some((
                board,
                SharedLinks {
                    link,
                    view,
                    comment,
                },
            ))
        })
        .collect()
}

/// The links the board in file `board` (empty: the untitled board) is shared by.
pub fn shared_link(board: &str) -> Option<SharedLinks> {
    shared_lines()
        .into_iter()
        .find(|(b, _)| b == board)
        .map(|(_, links)| links)
}

/// The file of the board shared by `link` (any of its links), if one is remembered.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn board_with_link(link: &str) -> Option<String> {
    shared_lines()
        .into_iter()
        .find(|(_, l)| {
            l.link == link || l.view.as_deref() == Some(link) || l.comment.as_deref() == Some(link)
        })
        .map(|(board, _)| board)
}

/// The links of the board in file `from` are now those of the board in `to`.
pub fn move_shared_link(from: &str, to: &str) {
    if let Some(links) = shared_link(from) {
        save_shared_link(from, None);
        save_shared_link(to, Some(&links));
    }
}

/// Remembers (or with `None` forgets) the links the board in file `board` is shared by.
pub fn save_shared_link(board: &str, links: Option<&SharedLinks>) {
    let mut lines: Vec<_> = shared_lines()
        .into_iter()
        .filter(|(b, _)| b != board)
        .collect();
    if let Some(links) = links {
        lines.push((board.to_owned(), links.clone()));
    }
    let text: String = lines
        .iter()
        .map(|(b, l)| {
            format!(
                "{b}\t{}\t{}\t{}\n",
                l.link,
                l.view.as_deref().unwrap_or_default(),
                l.comment.as_deref().unwrap_or_default()
            )
        })
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

/// File with what is remembered about update checks (see [`crate::updates::Prefs`]).
const UPDATES_FILE: &str = "updates.txt";

pub fn update_prefs() -> crate::updates::Prefs {
    pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::read(&dir.join(UPDATES_FILE)))
        .map(|bytes| crate::updates::Prefs::parse(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

pub fn save_update_prefs(prefs: &crate::updates::Prefs) {
    let saved = pb_platform::fs::app_data_dir().and_then(|dir| {
        pb_platform::fs::write_atomic(&dir.join(UPDATES_FILE), prefs.to_text().as_bytes())
    });
    if let Err(err) = saved {
        log::warn!("update settings not saved: {err}");
    }
}

/// File with the accounts signed in, a line each: server, email, session.
const ACCOUNTS_FILE: &str = "accounts.txt";

fn account_lines() -> Vec<(String, String, String)> {
    let text = pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::read(&dir.join(ACCOUNTS_FILE)))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let server = parts.next()?.to_owned();
            let email = parts.next()?.to_owned();
            let session = parts.next().filter(|s| !s.is_empty())?.to_owned();
            Some((server, email, session))
        })
        .collect()
}

/// The account signed in on `server`: its email and session.
pub fn account(server: &str) -> Option<(String, String)> {
    account_lines()
        .into_iter()
        .find(|(s, ..)| s == server)
        .map(|(_, email, session)| (email, session))
}

/// Remembers (or with `None` forgets) the account signed in on `server`.
pub fn save_account(server: &str, account: Option<(&str, &str)>) {
    let mut lines: Vec<_> = account_lines()
        .into_iter()
        .filter(|(s, ..)| s != server)
        .collect();
    if let Some((email, session)) = account {
        lines.push((server.to_owned(), email.to_owned(), session.to_owned()));
    }
    let text: String = lines
        .iter()
        .map(|(s, e, k)| format!("{s}\t{e}\t{k}\n"))
        .collect();
    let saved = pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::write_atomic(&dir.join(ACCOUNTS_FILE), text.as_bytes()));
    if let Err(err) = saved {
        log::warn!("account not remembered: {err}");
    }
}

/// File with what is known about pens (see [`PenSettings::to_words`]).
const PEN_FILE: &str = "pen.txt";

/// Whether a pen was ever used on this machine, and whether fingers move the board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PenSettings {
    /// A pen has been seen.
    pub seen: bool,
    /// "Pen draws, fingers move the board", if picked; when not, it is on once a pen has
    /// been seen.
    pub fingers_move: Option<bool>,
}

impl PenSettings {
    /// Whether fingers move the board rather than act as the mouse.
    pub fn fingers_navigate(self) -> bool {
        self.fingers_move.unwrap_or(self.seen)
    }

    pub fn from_words(text: &str) -> Self {
        let mut pen = Self::default();
        for word in text.split_whitespace() {
            match word {
                "seen" => pen.seen = true,
                "fingers-move" => pen.fingers_move = Some(true),
                "fingers-draw" => pen.fingers_move = Some(false),
                _ => {}
            }
        }
        pen
    }

    pub fn to_words(self) -> String {
        let mut words = Vec::new();
        if self.seen {
            words.push("seen");
        }
        match self.fingers_move {
            Some(true) => words.push("fingers-move"),
            Some(false) => words.push("fingers-draw"),
            None => {}
        }
        words.join(" ")
    }
}

pub fn pen() -> PenSettings {
    pb_platform::fs::app_data_dir()
        .and_then(|dir| pb_platform::fs::read(&dir.join(PEN_FILE)))
        .map(|bytes| PenSettings::from_words(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

pub fn save_pen(pen: PenSettings) {
    let saved = pb_platform::fs::app_data_dir().and_then(|dir| {
        pb_platform::fs::write_atomic(&dir.join(PEN_FILE), pen.to_words().as_bytes())
    });
    if let Err(err) = saved {
        log::warn!("pen settings not saved: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::PenSettings;

    #[test]
    fn fingers_move_the_board_once_a_pen_is_seen_unless_picked() {
        let fresh = PenSettings::default();
        assert!(!fresh.fingers_navigate());
        let seen = PenSettings {
            seen: true,
            ..fresh
        };
        assert!(seen.fingers_navigate());
        let picked = PenSettings {
            fingers_move: Some(false),
            ..seen
        };
        assert!(!picked.fingers_navigate());
        for pen in [fresh, seen, picked] {
            assert_eq!(PenSettings::from_words(&pen.to_words()), pen);
        }
    }
}
