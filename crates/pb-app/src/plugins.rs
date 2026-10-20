//! Plugins in the app: installing them, running them, answering their calls on the board
//! within their permissions, and their commands, panels and log.
//!
//! Installed plugins live in `<app data>/plugins/<id>/`; `plugins.txt` says which are on
//! (and where plugins loaded for development are); each plugin's `paintboard.storage` is
//! `<app data>/plugin-data/<id>.json`. The board's side of the calls is `plugin_api`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use pb_core::{Command, Document, ObjectId};
use pb_geom::kurbo::Rect;
use pb_plugin::{Engine, Manifest, Message, Permission};
use serde_json::{Value, json};

/// Lines kept in a plugin's log.
const LOG_LINES: usize = 200;

/// How much a plugin may keep with `paintboard.storage`.
const STORAGE_LIMIT: usize = 1 << 20;

/// Calls answered for a plugin before the app stops to draw: a plugin calling in a loop
/// cannot freeze the app.
const CALLS_PER_TURN: usize = 2000;

/// A command a plugin added to the Plugins menu.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginCommand {
    pub id: String,
    pub title: String,
    /// Also in the object menu, for the selection.
    pub selection: bool,
}

pub struct Plugin {
    pub manifest: Manifest,
    /// Its files: in the app's plugins folder, or where it is developed.
    pub dir: PathBuf,
    /// Loaded from its own folder and reloaded when its files change.
    pub dev: bool,
    pub enabled: bool,
    engine: Option<Engine>,
    pub log: VecDeque<(String, String)>,
    pub commands: Vec<PluginCommand>,
    /// The panel it shows (its description as given), if any.
    pub panel: Option<Value>,
    /// Why it is not running.
    pub error: Option<String>,
    /// When its files last changed, for development reloads.
    changed: Option<std::time::SystemTime>,
}

impl Plugin {
    pub fn running(&self) -> bool {
        self.engine.is_some()
    }

    fn note(&mut self, level: &str, text: impl Into<String>) {
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back((level.to_owned(), text.into()));
    }
}

/// What the app does for plugins; the app implements it.
pub trait Host {
    fn document(&self) -> &Document;
    /// Applies commands as one step; returns the ids of what they created.
    fn apply(&mut self, commands: Vec<Command>) -> Result<Vec<ObjectId>, String>;
    fn selection(&self) -> Vec<ObjectId>;
    fn select(&mut self, ids: Vec<ObjectId>);
    /// The part of the board in view, and the zoom.
    fn view(&self) -> (Rect, f64);
    fn zoom_to(&mut self, ids: &[ObjectId]);
    /// The board, with a way to measure text on it: width and height of `text` at font
    /// `size`, wrapped at `wrap` if given.
    fn measured(&mut self) -> (&Document, Box<crate::plugin_api::Measure<'_>>);
    fn copy_text(&mut self, text: &str);
    /// Shows a short message from plugin `name`.
    fn notify(&mut self, name: &str, text: &str);
    /// Fetches for plugin `plugin`; the answer comes back to [`Plugins::fetched`] with
    /// the call's `id`.
    fn fetch(&mut self, plugin: &str, id: u64, url: &str, options: &Value);
    /// Commands that follow merge into one step for Ctrl+Z, until `group(false)`.
    fn group(&mut self, on: bool);
    /// Wakes the app to take in what plugins posted (in the browser, later).
    fn waker(&self) -> Box<dyn Fn()>;
}

#[derive(Default)]
pub struct Plugins {
    pub list: Vec<Plugin>,
    /// The selection and board revision plugins last heard of.
    told_selection: Vec<ObjectId>,
    told_revision: u64,
    /// Where plugins' `paintboard.storage` is kept; in memory (`kept`) without one.
    data_dir: Option<PathBuf>,
    kept: std::collections::HashMap<String, serde_json::Map<String, Value>>,
}

fn plugins_dir() -> Option<PathBuf> {
    pb_platform::fs::app_data_dir()
        .ok()
        .map(|d| d.join("plugins"))
}

fn data_dir() -> Option<PathBuf> {
    pb_platform::fs::app_data_dir()
        .ok()
        .map(|d| d.join("plugin-data"))
}

const LIST_FILE: &str = "plugins.txt";

fn read_text(path: &Path) -> Result<String, String> {
    pb_platform::fs::read(path)
        .map_err(|e| format!("{}: {e}", path.display()))
        .and_then(|b| String::from_utf8(b).map_err(|_| format!("{}: not UTF-8", path.display())))
}

/// Reads the plugin in `dir`.
fn load(dir: &Path, dev: bool) -> Result<Plugin, String> {
    let manifest = Manifest::parse(&read_text(&dir.join("manifest.json"))?)?;
    Ok(Plugin {
        manifest,
        dir: dir.to_owned(),
        dev,
        enabled: true,
        engine: None,
        log: VecDeque::new(),
        commands: Vec::new(),
        panel: None,
        error: None,
        changed: modified(dir),
    })
}

/// When the plugin's manifest or scripts last changed.
fn modified(dir: &Path) -> Option<std::time::SystemTime> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter_map(|e| e.metadata().ok()?.modified().ok())
            .max()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = dir;
        None
    }
}

/// Plain text for a permission, to show before installing.
pub fn describe(permission: &Permission) -> String {
    use crate::i18n::{tr, trf};
    match permission {
        Permission::BoardRead => tr("read the board").to_owned(),
        Permission::BoardWrite => tr("change the board").to_owned(),
        Permission::Clipboard => tr("put text on the clipboard").to_owned(),
        Permission::Network(hosts) => trf("fetch from {}", &[&hosts.join(", ")]),
    }
}

fn required(call: &str) -> Option<Permission> {
    match call {
        "board.create" | "board.update" | "board.remove" => Some(Permission::BoardWrite),
        c if c.starts_with("board.") => Some(Permission::BoardRead),
        "clipboard.writeText" => Some(Permission::Clipboard),
        _ => None,
    }
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    Some(host.split(':').next().unwrap_or(host)).filter(|h| !h.is_empty())
}

enum Reply {
    Now(Result<Value, String>),
    /// Answered later (network).
    Later,
}

impl Plugins {
    /// The installed plugins, started if on.
    pub fn load_all(host: &mut dyn Host) -> Self {
        let mut plugins = Self {
            data_dir: data_dir(),
            ..Self::default()
        };
        let list = pb_platform::fs::app_data_dir()
            .ok()
            .and_then(|d| read_text(&d.join(LIST_FILE)).ok())
            .unwrap_or_default();
        // A line per plugin: `plugin` (installed here, by id) or `dev` (by folder), its
        // place, and whether it is on.
        let mut found = Vec::new();
        for line in list.lines() {
            let mut parts = line.split('\t');
            let (Some(kind), Some(place)) = (parts.next(), parts.next()) else {
                continue;
            };
            let on = parts.next() != Some("off");
            match kind {
                "plugin" => {
                    if let Some(dir) = plugins_dir() {
                        found.push((dir.join(place), false, on));
                    }
                }
                "dev" => found.push((PathBuf::from(place), true, on)),
                _ => {}
            }
        }
        for (dir, dev, on) in found {
            match load(&dir, dev) {
                Ok(mut plugin) => {
                    if plugins
                        .list
                        .iter()
                        .any(|p| p.manifest.id == plugin.manifest.id)
                    {
                        continue;
                    }
                    plugin.enabled = on;
                    plugins.list.push(plugin);
                }
                Err(err) => log::warn!("plugin in {} not loaded: {err}", dir.display()),
            }
        }
        plugins
            .list
            .sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        for i in 0..plugins.list.len() {
            if plugins.list[i].enabled {
                plugins.start(host, i);
            }
        }
        plugins
    }

    fn save_list(&self) {
        let text: String = self
            .list
            .iter()
            .map(|p| {
                let on = if p.enabled { "on" } else { "off" };
                if p.dev {
                    format!("dev\t{}\t{on}\n", p.dir.display())
                } else {
                    format!("plugin\t{}\t{on}\n", p.manifest.id)
                }
            })
            .collect();
        let saved = pb_platform::fs::app_data_dir()
            .and_then(|d| pb_platform::fs::write_atomic(&d.join(LIST_FILE), text.as_bytes()));
        if let Err(err) = saved {
            log::warn!("plugin list not saved: {err}");
        }
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.list.iter().position(|p| p.manifest.id == id)
    }

    /// Runs plugin `i`'s script.
    fn start(&mut self, host: &mut dyn Host, i: usize) {
        let plugin = &mut self.list[i];
        plugin.commands.clear();
        plugin.panel = None;
        plugin.error = None;
        let main = plugin.dir.join(&plugin.manifest.main);
        let started = read_text(&main)
            .and_then(|source| Engine::start(&plugin.manifest.main, host.waker(), &source));
        match started {
            Ok(engine) => {
                plugin.engine = Some(engine);
                plugin.note("info", crate::i18n::tr("Started"));
                // What it does as it starts is one step for Ctrl+Z too.
                host.group(true);
                self.pump(host, i);
                host.group(false);
            }
            Err(err) => {
                plugin.note("error", err.clone());
                plugin.error = Some(err);
            }
        }
    }

    fn stop(&mut self, i: usize) {
        let plugin = &mut self.list[i];
        plugin.engine = None;
        plugin.commands.clear();
        plugin.panel = None;
    }

    /// Installs (or replaces) a plugin from its files: `manifest.json` and its script
    /// among them. Asks about its permissions first; returns its name if installed.
    pub fn install(
        &mut self,
        host: &mut dyn Host,
        files: &[(String, Vec<u8>)],
    ) -> Result<Option<String>, String> {
        let manifest_text = files
            .iter()
            .find(|(name, _)| name == "manifest.json")
            .ok_or_else(|| {
                crate::i18n::tr("Pick the plugin's manifest.json and its scripts.").to_owned()
            })?;
        let manifest = Manifest::parse(&String::from_utf8_lossy(&manifest_text.1))?;
        if !files.iter().any(|(name, _)| *name == manifest.main) {
            return Err(crate::i18n::trf(
                "The plugin's script {} is missing.",
                &[&manifest.main],
            ));
        }
        if !confirm_install(&manifest) {
            return Ok(None);
        }
        let dir = plugins_dir()
            .ok_or("no folder for plugins")?
            .join(&manifest.id);
        if let Some(i) = self.index(&manifest.id) {
            self.stop(i);
            self.list.remove(i);
        }
        for (name, bytes) in files {
            // Only plain names: the files stay in the plugin's folder.
            if pb_plugin::package::plain_name(name) {
                pb_platform::fs::write_atomic(&dir.join(name), bytes).map_err(|e| e.to_string())?;
            }
        }
        let name = manifest.name.clone();
        let plugin = load(&dir, false)?;
        self.list.push(plugin);
        self.list
            .sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        self.save_list();
        let i = self.index(&manifest.id).expect("just added");
        self.start(host, i);
        Ok(Some(name))
    }

    /// Runs the plugin in `dir` from there, reloading it when its files change.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_dev(&mut self, host: &mut dyn Host, dir: &Path) -> Result<String, String> {
        let plugin = load(dir, true)?;
        if let Some(i) = self.index(&plugin.manifest.id) {
            self.stop(i);
            self.list.remove(i);
        }
        let (id, name) = (plugin.manifest.id.clone(), plugin.manifest.name.clone());
        self.list.push(plugin);
        self.list
            .sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        self.save_list();
        let i = self.index(&id).expect("just added");
        self.start(host, i);
        Ok(name)
    }

    pub fn set_enabled(&mut self, host: &mut dyn Host, id: &str, on: bool) {
        let Some(i) = self.index(id) else {
            return;
        };
        self.list[i].enabled = on;
        if on {
            self.start(host, i);
        } else {
            self.stop(i);
        }
        self.save_list();
    }

    /// Removes a plugin (its files too, unless it is developed elsewhere) and its data.
    pub fn remove(&mut self, id: &str) {
        let Some(i) = self.index(id) else {
            return;
        };
        self.stop(i);
        let plugin = self.list.remove(i);
        if !plugin.dev {
            for file in pb_platform::fs::list(&plugin.dir) {
                let _ = pb_platform::fs::remove(&file);
            }
            #[cfg(not(target_arch = "wasm32"))]
            let _ = std::fs::remove_dir(&plugin.dir);
        }
        if let Some(dir) = &self.data_dir {
            let _ = pb_platform::fs::remove(&dir.join(format!("{id}.json")));
        }
        self.kept.remove(id);
        self.save_list();
    }

    /// Starts plugins developed in their own folders again when their files changed.
    pub fn reload_changed(&mut self, host: &mut dyn Host) {
        for i in 0..self.list.len() {
            let plugin = &self.list[i];
            if !plugin.dev || !plugin.enabled {
                continue;
            }
            let now = modified(&plugin.dir);
            if now.is_some() && now != plugin.changed {
                self.list[i].changed = now;
                match load(&self.list[i].dir, true) {
                    Ok(fresh) => self.list[i].manifest = fresh.manifest,
                    Err(err) => {
                        self.list[i].note("error", err);
                        continue;
                    }
                }
                self.stop(i);
                self.list[i].note("info", crate::i18n::tr("Reloaded: its files changed"));
                self.start(host, i);
            }
        }
    }

    /// Plugin `id`'s files as one `.pbplugin`, and a name for it.
    pub fn package(&self, id: &str) -> Result<(String, Vec<u8>), String> {
        let plugin = &self.list[self.index(id).ok_or("no such plugin")?];
        let mut files = Vec::new();
        for path in pb_platform::fs::list(&plugin.dir) {
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            if pb_plugin::package::plain_name(&name) && !name.ends_with(".pbplugin") {
                let bytes = pb_platform::fs::read(&path).map_err(|e| format!("{name}: {e}"))?;
                files.push((name, bytes));
            }
        }
        let name = format!(
            "{}-{}.{}",
            plugin.manifest.id,
            plugin.manifest.version,
            pb_plugin::package::EXTENSION
        );
        Ok((name, pb_plugin::package::pack(&files)))
    }

    /// Whether any plugin is developed from its own folder (and so watched).
    pub fn watching(&self) -> bool {
        self.list.iter().any(|p| p.dev && p.enabled)
    }

    /// Runs command `command` of plugin `id`, as one step for Ctrl+Z.
    pub fn run_command(&mut self, host: &mut dyn Host, id: &str, command: &str) {
        let selection: Vec<String> = host.selection().iter().map(ToString::to_string).collect();
        self.send(
            host,
            id,
            "command",
            json!({ "id": command, "selection": selection }),
        );
    }

    /// Something happened in plugin `id`'s panel: a button or list item (`data`).
    pub fn panel_event(&mut self, host: &mut dyn Host, id: &str, data: Value) {
        self.send(host, id, "panel", data);
    }

    pub fn close_panel(&mut self, id: &str) {
        if let Some(i) = self.index(id) {
            self.list[i].panel = None;
        }
    }

    fn send(&mut self, host: &mut dyn Host, id: &str, event: &str, data: Value) {
        let Some(i) = self.index(id) else {
            return;
        };
        host.group(true);
        self.deliver(host, i, &pb_plugin::event(event, data));
        host.group(false);
    }

    fn deliver(&mut self, host: &mut dyn Host, i: usize, message: &str) {
        let Some(engine) = &mut self.list[i].engine else {
            return;
        };
        if let Err(err) = engine.deliver(message) {
            self.fail(i, err);
            return;
        }
        self.pump(host, i);
    }

    /// The plugin misbehaved: stopped, with why.
    fn fail(&mut self, i: usize, err: String) {
        let plugin = &mut self.list[i];
        plugin.note("error", err.clone());
        plugin.error = Some(err);
        self.stop(i);
    }

    /// Answers what plugin `i` posted, and what it posts in reply, until it is quiet.
    fn pump(&mut self, host: &mut dyn Host, i: usize) {
        let mut answered = 0;
        loop {
            let Some(engine) = &mut self.list[i].engine else {
                return;
            };
            let posted = engine.take_posted();
            if posted.is_empty() {
                return;
            }
            for text in posted {
                let message = match Message::parse(&text) {
                    Ok(message) => message,
                    Err(err) => {
                        self.list[i].note("error", err);
                        continue;
                    }
                };
                match message {
                    Message::Log { level, text } => self.list[i].note(&level, text),
                    Message::Call { id, name, args } => {
                        answered += 1;
                        if answered > CALLS_PER_TURN {
                            self.fail(
                                i,
                                crate::i18n::tr("Stopped: too many calls at once").to_owned(),
                            );
                            return;
                        }
                        let reply = self.call(host, i, id, &name, &args);
                        if let Reply::Now(result) = reply {
                            let answer = pb_plugin::answer(id, result);
                            let Some(engine) = &mut self.list[i].engine else {
                                return;
                            };
                            if let Err(err) = engine.deliver(&answer) {
                                self.fail(i, err);
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Takes in what plugins posted on their own time (in the browser, and after fetches).
    pub fn poll(&mut self, host: &mut dyn Host) {
        for i in 0..self.list.len() {
            self.pump(host, i);
        }
    }

    /// The answer to a fetch of plugin `plugin`'s call `id`.
    pub fn fetched(
        &mut self,
        host: &mut dyn Host,
        plugin: &str,
        id: u64,
        result: Result<String, String>,
    ) {
        let Some(i) = self.index(plugin) else {
            return;
        };
        let value = match result {
            Ok(text) => Ok(json!({ "ok": true, "status": 200, "text": text })),
            Err(err) => match err.get(..3).and_then(|s| s.parse::<u16>().ok()) {
                Some(status) => Ok(json!({
                    "ok": false,
                    "status": status,
                    "text": err.get(3..).unwrap_or_default().trim_start_matches(':').trim(),
                })),
                None => Err(err),
            },
        };
        host.group(true);
        self.deliver(host, i, &pb_plugin::answer(id, value));
        host.group(false);
    }

    /// Tells plugins that may read the board about a new selection or changed board.
    pub fn tell_changes(&mut self, host: &mut dyn Host) {
        let selection = host.selection();
        let revision = host.document().revision();
        let selected = selection != self.told_selection;
        let changed = revision != self.told_revision;
        if !selected && !changed {
            return;
        }
        self.told_selection = selection.clone();
        self.told_revision = revision;
        let ids: Vec<String> = selection.iter().map(ToString::to_string).collect();
        for i in 0..self.list.len() {
            if !self.list[i].running() || !self.list[i].manifest.allows(&Permission::BoardRead) {
                continue;
            }
            if selected {
                self.deliver(host, i, &pb_plugin::event("selectionchange", json!(ids)));
            }
            if changed {
                self.deliver(host, i, &pb_plugin::event("boardchange", Value::Null));
            }
        }
        // What the plugins changed in answer is not news to them.
        self.told_selection = host.selection();
        self.told_revision = host.document().revision();
    }

    fn call(
        &mut self,
        host: &mut dyn Host,
        i: usize,
        call_id: u64,
        name: &str,
        args: &[Value],
    ) -> Reply {
        let manifest = &self.list[i].manifest;
        if let Some(needed) = required(name)
            && !manifest.allows(&needed)
        {
            return Reply::Now(Err(format!(
                "the plugin has no permission for {name}: add \"{}\" to its manifest",
                match needed {
                    Permission::BoardRead => "board:read",
                    Permission::BoardWrite => "board:write",
                    Permission::Clipboard => "clipboard",
                    Permission::Network(_) => "network:<host>",
                }
            )));
        }
        let arg = |n: usize| args.get(n).cloned().unwrap_or(Value::Null);
        let result = match name {
            "board.getObjects" => crate::plugin_api::objects_json(host.document(), &arg(0)),
            "board.getObject" => crate::plugin_api::parse_id(&arg(0)).map(|id| {
                host.document()
                    .get(id)
                    .map_or(Value::Null, |o| crate::plugin_api::object_json(host.document(), o))
            }),
            "board.getSelection" => Ok(json!(
                host.selection().iter().map(ToString::to_string).collect::<Vec<_>>()
            )),
            "board.select" => crate::plugin_api::parse_ids(&arg(0)).map(|ids| {
                let ids: Vec<_> = ids.into_iter().filter(|id| host.document().get(*id).is_some()).collect();
                host.select(ids);
                Value::Null
            }),
            "board.getViewport" => {
                let (view, zoom) = host.view();
                Ok(json!({
                    "x": view.x0, "y": view.y0, "width": view.width(), "height": view.height(), "zoom": zoom,
                }))
            }
            "board.zoomTo" => crate::plugin_api::parse_ids(&arg(0)).map(|ids| {
                host.zoom_to(&ids);
                Value::Null
            }),
            "board.getTags" => Ok(json!(
                host.document()
                    .tags()
                    .iter()
                    .map(|t| json!({ "id": &*t.id, "name": &*t.name, "color": crate::plugin_api::color_text(t.color) }))
                    .collect::<Vec<_>>()
            )),
            "board.create" => self.create(host, &arg(0)),
            "board.update" => crate::plugin_api::parse_id(&arg(0)).and_then(|id| {
                let commands = {
                    let (doc, mut measure) = host.measured();
                    crate::plugin_api::update_commands(doc, id, &arg(1), &mut *measure)
                }?;
                host.apply(commands).map(|_| Value::Null)
            }),
            "board.remove" => crate::plugin_api::parse_ids(&arg(0)).and_then(|ids| {
                let ids: Vec<_> = ids.into_iter().filter(|id| host.document().get(*id).is_some()).collect();
                if ids.is_empty() {
                    return Ok(Value::Null);
                }
                host.apply(vec![Command::Delete(ids)]).map(|_| Value::Null)
            }),
            "ui.addCommand" => {
                let command = arg(0);
                let id = command.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
                let title = command.get("title").and_then(Value::as_str).unwrap_or(&id).to_owned();
                if id.is_empty() {
                    Err("a command needs an id".to_owned())
                } else {
                    let selection = command.get("selection").and_then(Value::as_bool).unwrap_or(false);
                    let commands = &mut self.list[i].commands;
                    commands.retain(|c| c.id != id);
                    commands.push(PluginCommand { id, title, selection });
                    Ok(Value::Null)
                }
            }
            "ui.showPanel" => {
                self.list[i].panel = Some(arg(0));
                Ok(Value::Null)
            }
            "ui.closePanel" => {
                self.list[i].panel = None;
                Ok(Value::Null)
            }
            "ui.notify" => {
                let text = arg(0).as_str().unwrap_or_default().to_owned();
                let name = self.list[i].manifest.name.clone();
                host.notify(&name, &text);
                Ok(Value::Null)
            }
            "storage.get" => {
                let key = arg(0).as_str().unwrap_or_default().to_owned();
                Ok(self.storage(i).remove(&key).unwrap_or(Value::Null))
            }
            "storage.set" => {
                let key = arg(0).as_str().unwrap_or_default().to_owned();
                let mut data = self.storage(i);
                match arg(1) {
                    Value::Null => data.remove(&key),
                    value => data.insert(key, value),
                };
                let text = serde_json::to_string(&data).unwrap_or_default();
                if text.len() > STORAGE_LIMIT {
                    Err("storage is full (1 MB per plugin)".to_owned())
                } else {
                    self.keep(i, data, &text).map(|()| Value::Null)
                }
            }
            "clipboard.writeText" => {
                host.copy_text(arg(0).as_str().unwrap_or_default());
                Ok(Value::Null)
            }
            "net.fetch" => {
                let url = arg(0).as_str().unwrap_or_default().to_owned();
                match host_of(&url) {
                    Some(h) if self.list[i].manifest.may_fetch(h) => {
                        let id = self.list[i].manifest.id.clone();
                        host.fetch(&id, call_id, &url, &arg(1));
                        return Reply::Later;
                    }
                    Some(h) => Err(format!("the plugin has no permission to fetch from {h}: add \"network:{h}\" to its manifest")),
                    None => Err(format!("not a web address: {url}")),
                }
            }
            other => Err(format!("no such call: {other}")),
        };
        Reply::Now(result)
    }

    /// What plugin `i` keeps with `paintboard.storage`.
    fn storage(&self, i: usize) -> serde_json::Map<String, Value> {
        let id = &self.list[i].manifest.id;
        match &self.data_dir {
            Some(dir) => read_text(&dir.join(format!("{id}.json")))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default(),
            None => self.kept.get(id).cloned().unwrap_or_default(),
        }
    }

    fn keep(
        &mut self,
        i: usize,
        data: serde_json::Map<String, Value>,
        text: &str,
    ) -> Result<(), String> {
        let id = self.list[i].manifest.id.clone();
        match &self.data_dir {
            Some(dir) => {
                pb_platform::fs::write_atomic(&dir.join(format!("{id}.json")), text.as_bytes())
                    .map_err(|e| e.to_string())
            }
            None => {
                self.kept.insert(id, data);
                Ok(())
            }
        }
    }

    fn create(&mut self, host: &mut dyn Host, items: &Value) -> Result<Value, String> {
        let items = items.as_array().ok_or("create takes a list of objects")?;
        let (view, _) = host.view();
        let center = view.center();
        let mut contents = Vec::new();
        {
            let (doc, mut measure) = host.measured();
            for item in items {
                contents
                    .push(crate::plugin_api::new_object(item, doc, center, &mut *measure)?.into());
            }
        }
        if contents.is_empty() {
            return Ok(json!([]));
        }
        let created = host.apply(vec![Command::Create(contents)])?;
        Ok(json!(
            created.iter().map(ToString::to_string).collect::<Vec<_>>()
        ))
    }

    /// Commands of running plugins: (plugin id, plugin name, command).
    pub fn commands(&self) -> Vec<(String, String, PluginCommand)> {
        self.list
            .iter()
            .filter(|p| p.running())
            .flat_map(|p| {
                p.commands
                    .iter()
                    .map(|c| (p.manifest.id.clone(), p.manifest.name.clone(), c.clone()))
            })
            .collect()
    }
}

/// Asks before installing: what the plugin is and what it may do.
fn confirm_install(manifest: &Manifest) -> bool {
    use crate::i18n::{tr, trf};
    let mut text = trf(
        "Install the plugin \"{}\" {}",
        &[&manifest.name, &manifest.version],
    );
    if !manifest.author.is_empty() {
        text.push_str(&trf(" by {}", &[&manifest.author]));
    }
    text += "?\n\n";
    if manifest.permissions.is_empty() {
        text += tr("It asks for no permissions: it can only show its own commands and panel.");
    } else {
        text += tr("It will be able to:");
        for p in &manifest.permissions {
            text += "\n• ";
            text += &describe(p);
        }
    }
    text += "\n\n";
    text += tr("Install plugins only from people you trust.");
    pb_platform::dialog::confirm(tr("Install plugin"), &text)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    struct Fake {
        doc: Document,
        selection: Vec<ObjectId>,
        notes: Vec<String>,
    }

    impl Host for Fake {
        fn document(&self) -> &Document {
            &self.doc
        }
        fn apply(&mut self, commands: Vec<Command>) -> Result<Vec<ObjectId>, String> {
            let mut created = Vec::new();
            for command in commands {
                created.extend(self.doc.apply(command).map_err(|e| e.to_string())?.created);
            }
            Ok(created)
        }
        fn selection(&self) -> Vec<ObjectId> {
            self.selection.clone()
        }
        fn select(&mut self, ids: Vec<ObjectId>) {
            self.selection = ids;
        }
        fn view(&self) -> (Rect, f64) {
            (Rect::new(0.0, 0.0, 1000.0, 800.0), 1.0)
        }
        fn zoom_to(&mut self, _ids: &[ObjectId]) {}
        fn measured(&mut self) -> (&Document, Box<crate::plugin_api::Measure<'_>>) {
            (
                &self.doc,
                Box::new(|t: &str, s: f64, _| (t.len() as f64 * s / 2.0, s)),
            )
        }
        fn copy_text(&mut self, _text: &str) {}
        fn notify(&mut self, _name: &str, text: &str) {
            self.notes.push(text.to_owned());
        }
        fn fetch(&mut self, _plugin: &str, _id: u64, _url: &str, _options: &Value) {}
        fn group(&mut self, on: bool) {
            if on {
                self.doc.begin_undo_group();
            } else {
                self.doc.end_undo_group();
            }
        }
        fn waker(&self) -> Box<dyn Fn()> {
            Box::new(|| {})
        }
    }

    impl Plugins {
        fn add(&mut self, host: &mut dyn Host, manifest: &str, source: &str) -> usize {
            let manifest = Manifest::parse(manifest).unwrap();
            self.list.push(Plugin {
                manifest,
                dir: PathBuf::new(),
                dev: false,
                enabled: true,
                engine: Some(Engine::start("main.js", host.waker(), source).unwrap()),
                log: VecDeque::new(),
                commands: Vec::new(),
                panel: None,
                error: None,
                changed: None,
            });
            let i = self.list.len() - 1;
            self.pump(host, i);
            i
        }
    }

    const NUMBERS: &str = r#"
        paintboard.ui.addCommand({ id: "number", title: "Number them", selection: true }, async ({ selection }) => {
            let n = 1;
            for (const id of selection) {
                const o = await paintboard.board.getObject(id);
                if (o.type === "sticky") await paintboard.board.update(id, { text: `${n++}. ${o.text}` });
            }
            const [made] = await paintboard.board.create({ type: "text", text: `${n - 1} numbered`, x: 0, y: -100 });
            await paintboard.board.select([made]);
            await paintboard.ui.notify("done");
        });"#;

    #[test]
    fn a_command_changes_the_board_in_one_step() {
        let mut host = Fake {
            doc: Document::new(),
            selection: Vec::new(),
            notes: Vec::new(),
        };
        let mut plugins = Plugins::default();
        let i = plugins.add(
            &mut host,
            r#"{"id":"t.numbers","name":"Numbers","version":"1","api":1,"permissions":["board:read","board:write"]}"#,
            NUMBERS,
        );
        assert_eq!(plugins.list[i].commands.len(), 1);
        assert!(plugins.list[i].commands[0].selection);
        let mut notes = Vec::new();
        for text in ["a", "b"] {
            let data = crate::plugin_api::new_object(
                &json!({"type": "sticky", "text": text}),
                &host.doc,
                pb_geom::kurbo::Point::ZERO,
                &mut |_: &str, _, _| (1.0, 1.0),
            )
            .unwrap();
            host.doc.apply(Command::Create(vec![data.into()])).unwrap();
            notes.push(host.doc.objects().last().unwrap().id);
        }
        host.selection = notes.clone();
        plugins.run_command(&mut host, "t.numbers", "number");
        assert_eq!(plugins.list[i].error, None, "{:?}", plugins.list[i].log);
        let texts: Vec<_> = host
            .doc
            .objects()
            .iter()
            .filter_map(|o| match &o.data.kind {
                pb_core::ObjectKind::Sticky(s) => Some(s.text.to_string()),
                pb_core::ObjectKind::Text(t) => Some(t.text.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["1. a", "2. b", "2 numbered"]);
        assert_eq!(host.notes, ["done"]);
        assert_eq!(host.selection.len(), 1);
        // One Ctrl+Z takes it all back.
        host.doc.undo().unwrap();
        assert_eq!(host.doc.objects().len(), 2);
    }

    #[test]
    fn plugins_get_only_what_they_may() {
        let mut host = Fake {
            doc: Document::new(),
            selection: Vec::new(),
            notes: Vec::new(),
        };
        let mut plugins = Plugins::default();
        let i = plugins.add(
            &mut host,
            r#"{"id":"t.reader","name":"Reader","version":"1","api":1,"permissions":["board:read"]}"#,
            r#"paintboard.ui.addCommand({ id: "go", title: "Go" }, async () => {
                 const all = await paintboard.board.getObjects();
                 console.log("saw", all.length);
                 try { await paintboard.board.create({ type: "sticky" }); } catch (e) { console.log(e.message); }
                 try { await paintboard.net.fetch("https://example.com/"); } catch (e) { console.log(e.message); }
               });"#,
        );
        plugins.run_command(&mut host, "t.reader", "go");
        let log: Vec<_> = plugins.list[i]
            .log
            .iter()
            .map(|(_, t)| t.as_str())
            .collect();
        assert!(log.contains(&"saw 0"), "{log:?}");
        assert!(
            log.iter()
                .any(|l| l.contains("no permission for board.create")),
            "{log:?}"
        );
        assert!(
            log.iter()
                .any(|l| l.contains("no permission to fetch from example.com")),
            "{log:?}"
        );
        assert!(host.doc.objects().is_empty());
    }

    /// Every call the prelude makes is one the app answers.
    #[test]
    fn the_app_knows_every_call() {
        let mut host = Fake {
            doc: Document::new(),
            selection: Vec::new(),
            notes: Vec::new(),
        };
        let mut plugins = Plugins::default();
        let i = plugins.add(
            &mut host,
            r#"{"id":"t.all","name":"All","version":"1","api":1,
                "permissions":["board:read","board:write","clipboard","network:example.com"]}"#,
            "",
        );
        for name in pb_plugin::call_names() {
            if let Reply::Now(Err(err)) = plugins.call(&mut host, i, 1, name, &[]) {
                assert!(!err.starts_with("no such call"), "{name}: {err}");
            }
        }
    }

    /// The example plugins shipped for authors run without errors.
    #[test]
    fn the_examples_work() {
        let examples = [
            (
                include_str!("../../../packaging/plugins/examples/number-stickies/manifest.json"),
                include_str!("../../../packaging/plugins/examples/number-stickies/main.js"),
                "number",
                None,
            ),
            (
                include_str!("../../../packaging/plugins/examples/stickies-by-color/manifest.json"),
                include_str!("../../../packaging/plugins/examples/stickies-by-color/main.js"),
                "arrange",
                None,
            ),
            (
                include_str!("../../../packaging/plugins/examples/quick-notes/manifest.json"),
                include_str!("../../../packaging/plugins/examples/quick-notes/main.js"),
                "open",
                Some(
                    json!({ "button": "add", "values": { "lines": "one\ntwo\n\nthree", "color": "green" } }),
                ),
            ),
        ];
        for (manifest, source, command, panel) in examples {
            let mut host = Fake {
                doc: Document::new(),
                selection: Vec::new(),
                notes: Vec::new(),
            };
            let mut ids = Vec::new();
            for (text, fill) in [("b", "#ffeb7a"), ("a", "#a8d8ff"), ("c", "#ffeb7a")] {
                let data = crate::plugin_api::new_object(
                    &json!({"type": "sticky", "text": text, "fill": fill, "x": ids.len() as f64 * 300.0}),
                    &host.doc,
                    pb_geom::kurbo::Point::ZERO,
                    &mut |_: &str, _, _| (1.0, 1.0),
                )
                .unwrap();
                host.doc.apply(Command::Create(vec![data.into()])).unwrap();
                ids.push(host.doc.objects().last().unwrap().id);
            }
            host.selection = ids;
            let mut plugins = Plugins::default();
            let i = plugins.add(&mut host, manifest, source);
            let id = plugins.list[i].manifest.id.clone();
            plugins.run_command(&mut host, &id, command);
            if let Some(event) = panel {
                assert!(plugins.list[i].panel.is_some());
                plugins.panel_event(&mut host, &id, event);
            }
            let errors: Vec<_> = plugins.list[i]
                .log
                .iter()
                .filter(|(l, _)| l != "info")
                .collect();
            assert!(
                errors.is_empty() && plugins.list[i].error.is_none(),
                "{id}: {errors:?}"
            );
            let texts: Vec<String> = host
                .doc
                .objects()
                .iter()
                .filter_map(|o| match &o.data.kind {
                    pb_core::ObjectKind::Sticky(s) => Some(s.text.to_string()),
                    pb_core::ObjectKind::Text(t) => Some(t.text.to_string()),
                    _ => None,
                })
                .collect();
            match command {
                "number" => assert_eq!(texts, ["1. b", "2. a", "3. c"]),
                "arrange" => {
                    // Two columns, a heading over each.
                    assert_eq!(texts.len(), 5);
                    let xs: Vec<f64> = host.doc.objects()[..3]
                        .iter()
                        .map(|o| o.data.bounds.x0)
                        .collect();
                    assert_eq!(xs[0], xs[2]);
                    assert!(xs[1] > xs[0]);
                }
                _ => assert_eq!(&texts[3..], ["one", "two", "three"]),
            }
        }
    }

    #[test]
    fn plugins_in_development_pack_into_one_file() {
        let dir = std::env::temp_dir().join(format!("pb-plugin-pack-{}", std::process::id()));
        let example = "../../packaging/plugins/examples/quick-notes";
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(example);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["manifest.json", "main.js"] {
            std::fs::copy(example.join(name), dir.join(name)).unwrap();
        }
        // Not taken along: hidden files and earlier packages.
        std::fs::write(dir.join(".notes"), "x").unwrap();
        std::fs::write(dir.join("old.pbplugin"), "x").unwrap();
        let mut plugin = load(&dir, true).unwrap();
        plugin.enabled = false;
        let plugins = Plugins {
            list: vec![plugin],
            ..Plugins::default()
        };
        let (name, bytes) = plugins.package("paintboard.quick-notes").unwrap();
        assert_eq!(name, "paintboard.quick-notes-1.0.0.pbplugin");
        let mut files = pb_plugin::package::unpack(&bytes).unwrap();
        files.sort();
        let names: Vec<_> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["main.js", "manifest.json"]);
        assert_eq!(files[0].1, std::fs::read(dir.join("main.js")).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hosts_of_addresses() {
        assert_eq!(
            host_of("https://api.example.com:8443/x?y"),
            Some("api.example.com")
        );
        assert_eq!(host_of("http://user@site.org/"), Some("site.org"));
        assert_eq!(host_of("file:///c"), None);
    }
}
