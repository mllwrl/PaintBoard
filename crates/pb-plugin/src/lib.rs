//! Plugins: JavaScript that reads and changes the board through the app.
//!
//! A plugin is a folder with a [`Manifest`] (`manifest.json`) and its script. The script
//! runs after [`PRELUDE`], which gives it the `paintboard` object; every call on it
//! becomes a message to the app ([`Message`]) and the app's answer settles the promise the
//! call returned. On the desktop the script runs in QuickJS ([`Engine`]), in the browser
//! in a Web Worker ([`Engine`] there too, answering later); either way it sees only what the app answers, and the app answers
//! only what the plugin's permissions allow.

mod manifest;
#[cfg(not(target_arch = "wasm32"))]
mod runtime;
#[cfg(target_arch = "wasm32")]
mod worker;

pub use manifest::{API_VERSION, Manifest, Permission};
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::Engine;
#[cfg(target_arch = "wasm32")]
pub use worker::Engine;

use serde_json::Value;

/// Defines `paintboard` for the plugin's script; run before it.
pub const PRELUDE: &str = include_str!("prelude.js");

/// A message from a plugin to the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A call on `paintboard`, to answer with [`answer`] under its `id`.
    Call {
        id: u64,
        name: String,
        args: Vec<Value>,
    },
    /// A line for the plugin's log.
    Log { level: String, text: String },
}

impl Message {
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if let Some(text) = value.get("log").and_then(Value::as_str) {
            let level = value
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or("info")
                .to_owned();
            return Ok(Self::Log {
                level,
                text: text.to_owned(),
            });
        }
        let id = value
            .get("id")
            .and_then(Value::as_u64)
            .ok_or("a call without an id")?;
        let name = value
            .get("call")
            .and_then(Value::as_str)
            .ok_or("a call without a name")?
            .to_owned();
        let args = match value.get("args") {
            Some(Value::Array(args)) => args.clone(),
            _ => Vec::new(),
        };
        Ok(Self::Call { id, name, args })
    }
}

/// The app's answer to call `id`: its result, or why it failed.
pub fn answer(id: u64, result: Result<Value, String>) -> String {
    match result {
        Ok(value) => serde_json::json!({ "id": id, "ok": value }),
        Err(error) => serde_json::json!({ "id": id, "error": error }),
    }
    .to_string()
}

/// Something that happened, for the plugin's `paintboard.on` handlers (and its commands
/// and panel: events `command` and `panel`).
pub fn event(name: &str, data: Value) -> String {
    serde_json::json!({ "event": name, "data": data }).to_string()
}

/// The names of the calls the prelude makes, like `board.create`.
pub fn call_names() -> Vec<&'static str> {
    PRELUDE
        .match_indices("call(\"")
        .filter_map(|(i, m)| PRELUDE[i + m.len()..].split('"').next())
        .filter(|name| name.contains('.'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The types authors get describe every call.
    #[test]
    fn the_typescript_types_match_the_prelude() {
        let types = include_str!("../../../packaging/plugins/paintboard.d.ts");
        let names = call_names();
        assert!(names.len() >= 15, "{names:?}");
        for name in names {
            let (space, function) = name.split_once('.').unwrap();
            let start = types
                .find(&format!("namespace {space} {{"))
                .unwrap_or_else(|| panic!("no namespace {space} in paintboard.d.ts"));
            let body = &types[start..];
            let body = &body[..body.find("\n  }").unwrap()];
            assert!(
                body.contains(&format!("function {function}(")),
                "{name} is not in paintboard.d.ts"
            );
        }
    }

    #[test]
    fn messages_are_read() {
        assert_eq!(
            Message::parse(r#"{"id":3,"call":"board.update","args":["1@2",{"x":5}]}"#).unwrap(),
            Message::Call {
                id: 3,
                name: "board.update".into(),
                args: vec!["1@2".into(), serde_json::json!({"x": 5})],
            }
        );
        assert_eq!(
            Message::parse(r#"{"log":"hi","level":"warn"}"#).unwrap(),
            Message::Log {
                level: "warn".into(),
                text: "hi".into()
            }
        );
        assert!(Message::parse(r#"{"call":"x"}"#).is_err());
        assert_eq!(answer(1, Err("no".into())), r#"{"error":"no","id":1}"#);
    }
}
