//! `manifest.json`: what a plugin is and what it may do.
//!
//! ```json
//! {
//!   "id": "example.number-stickies",
//!   "name": "Number stickies",
//!   "version": "1.0.0",
//!   "author": "Ann",
//!   "description": "Puts 1, 2, 3… before the text of the selected sticky notes.",
//!   "api": 1,
//!   "permissions": ["board:read", "board:write"],
//!   "main": "main.js"
//! }
//! ```

use serde::Deserialize;

/// The plugin API this app speaks. Plugins asking for a newer one are refused; older ones
/// keep working.
pub const API_VERSION: u32 = 1;

/// Something a plugin may be allowed to do. Without any, it can only show its own UI.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Permission {
    /// Read the board: objects, selection, view, tags.
    BoardRead,
    /// Change the board.
    BoardWrite,
    /// Fetch pages from these hosts (and their subdomains).
    Network(Vec<String>),
    /// Put text on the clipboard.
    Clipboard,
}

impl Permission {
    fn parse(text: &str) -> Result<Self, String> {
        match text {
            "board:read" => Ok(Self::BoardRead),
            "board:write" => Ok(Self::BoardWrite),
            "clipboard" => Ok(Self::Clipboard),
            _ => {
                let hosts = text
                    .strip_prefix("network:")
                    .ok_or_else(|| format!("unknown permission \"{text}\""))?;
                let hosts: Vec<String> = hosts
                    .split(',')
                    .map(|h| h.trim().to_ascii_lowercase())
                    .filter(|h| !h.is_empty())
                    .collect();
                let fine = !hosts.is_empty()
                    && hosts.iter().all(|h| {
                        h.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
                            && h.contains('.')
                    });
                if fine {
                    Ok(Self::Network(hosts))
                } else {
                    Err(format!("\"{text}\" should name hosts: network:example.com"))
                }
            }
        }
    }
}

/// A plugin's `manifest.json`, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Unique, like `author.plugin-name`: letters, digits, `.`, `-`, `_`.
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub api: u32,
    pub permissions: Vec<Permission>,
    /// The script to run, inside the plugin's folder.
    pub main: String,
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    description: String,
    api: u32,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default = "default_main")]
    main: String,
}

fn default_main() -> String {
    "main.js".into()
}

impl Manifest {
    pub fn parse(json: &str) -> Result<Self, String> {
        let raw: Raw = serde_json::from_str(json).map_err(|e| format!("manifest.json: {e}"))?;
        let plain = |s: &str| {
            !s.is_empty()
                && s.len() <= 100
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        };
        if !plain(&raw.id) {
            return Err(
                "manifest.json: \"id\" may have only letters, digits, '.', '-', '_'".into(),
            );
        }
        if raw.name.trim().is_empty() {
            return Err("manifest.json: \"name\" is empty".into());
        }
        if raw.api == 0 || raw.api > API_VERSION {
            return Err(format!(
                "the plugin needs API {} but this PaintBoard has API {API_VERSION}: update PaintBoard",
                raw.api
            ));
        }
        // The script lies in the plugin's folder: no way out of it.
        if !plain(&raw.main) || raw.main.starts_with('.') || !raw.main.ends_with(".js") {
            return Err("manifest.json: \"main\" should be a .js file next to it".into());
        }
        let mut permissions = raw
            .permissions
            .iter()
            .map(|p| Permission::parse(p))
            .collect::<Result<Vec<_>, _>>()?;
        permissions.sort();
        permissions.dedup();
        Ok(Self {
            id: raw.id,
            name: raw.name.trim().to_owned(),
            version: raw.version,
            author: raw.author,
            description: raw.description,
            api: raw.api,
            permissions,
            main: raw.main,
        })
    }

    pub fn allows(&self, permission: &Permission) -> bool {
        self.permissions.contains(permission)
    }

    /// Whether the plugin may fetch from `host`.
    pub fn may_fetch(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.permissions.iter().any(|p| match p {
            Permission::Network(hosts) => hosts
                .iter()
                .any(|h| host == *h || host.ends_with(&format!(".{h}"))),
            _ => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_are_checked() {
        let m = Manifest::parse(
            r#"{"id":"ann.numbers","name":" Numbers ","version":"1.0","api":1,
                "permissions":["board:write","board:read","network:api.example.com, Data.org"]}"#,
        )
        .unwrap();
        assert_eq!(m.name, "Numbers");
        assert_eq!(m.main, "main.js");
        assert!(m.allows(&Permission::BoardRead) && !m.allows(&Permission::Clipboard));
        assert!(m.may_fetch("api.example.com") && m.may_fetch("x.data.org"));
        assert!(!m.may_fetch("example.com") && !m.may_fetch("evildata.org"));
        for bad in [
            r#"{"id":"../x","name":"n","version":"1","api":1}"#,
            r#"{"id":"x","name":"","version":"1","api":1}"#,
            r#"{"id":"x","name":"n","version":"1","api":99}"#,
            r#"{"id":"x","name":"n","version":"1","api":1,"main":"../a.js"}"#,
            r#"{"id":"x","name":"n","version":"1","api":1,"permissions":["files"]}"#,
            r#"{"id":"x","name":"n","version":"1","api":1,"permissions":["network:"]}"#,
            "not json",
        ] {
            assert!(Manifest::parse(bad).is_err(), "{bad}");
        }
    }
}
