//! Plugins on the desktop: each runs in its own QuickJS runtime on the app's thread, with
//! limits on memory and on how long it may run at once, so a plugin that loops forever
//! or eats memory is stopped without taking the app down.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::{CatchResultExt, Context, Function, Runtime};

/// Memory a plugin may use.
const MEMORY: usize = 64 << 20;

/// How long a plugin may run without stopping, from one message to it.
const TIME: Duration = Duration::from_secs(2);

pub struct Engine {
    runtime: Runtime,
    context: Context,
    /// Messages the plugin posted, in order.
    outbox: Rc<RefCell<Vec<String>>>,
    /// When the plugin is stopped if still running.
    deadline: Rc<Cell<Option<Instant>>>,
}

impl Engine {
    /// Runs `source` (the plugin's script, named `name` in errors) after the prelude.
    /// Everything happens right away here, so `_wake` (for messages arriving later, as
    /// in the browser) is not needed.
    pub fn start(name: &str, _wake: Box<dyn Fn()>, source: &str) -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(MEMORY);
        runtime.set_max_stack_size(1 << 20);
        let deadline = Rc::new(Cell::new(None::<Instant>));
        let watched = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            watched.get().is_some_and(|at| Instant::now() > at)
        })));
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        let outbox = Rc::new(RefCell::new(Vec::new()));
        let engine = Self {
            runtime,
            context,
            outbox,
            deadline,
        };
        let posted = engine.outbox.clone();
        engine.context.with(|ctx| {
            let post = Function::new(ctx.clone(), move |text: String| {
                posted.borrow_mut().push(text);
            })
            .map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__pb_post", post)
                .map_err(|e| e.to_string())
        })?;
        engine.run("prelude", crate::PRELUDE)?;
        engine.run(name, source)?;
        Ok(engine)
    }

    /// Evaluates `source`, then whatever it left to do (settled promises).
    fn run(&self, name: &str, source: &str) -> Result<(), String> {
        self.deadline.set(Some(Instant::now() + TIME));
        let result = self.context.with(|ctx| {
            ctx.eval::<(), _>(source)
                .catch(&ctx)
                .map_err(|e| format!("{name}: {e}"))
        });
        let jobs = self.run_jobs();
        self.deadline.set(None);
        result.and(jobs)
    }

    /// Runs promise callbacks until there are none, within the time limit.
    fn run_jobs(&self) -> Result<(), String> {
        loop {
            match self.runtime.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(err) => {
                    let text = err.0.with(|ctx| {
                        let caught = ctx.catch();
                        caught
                            .as_exception()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| format!("{caught:?}"))
                    });
                    // A callback that threw: the plugin's own error, reported, not fatal
                    // unless it was stopped for taking too long.
                    if self.deadline.get().is_some_and(|at| Instant::now() > at) {
                        return Err(format!("stopped: it ran for over {} s", TIME.as_secs()));
                    }
                    self.outbox
                        .borrow_mut()
                        .push(serde_json::json!({ "log": text, "level": "error" }).to_string());
                }
            }
        }
    }

    /// Hands the plugin a message (an answer or an event); what it posts in reply waits in
    /// [`Engine::take_posted`]. An error means it misbehaved (ran too long, out of
    /// memory): better stop it.
    pub fn deliver(&mut self, message: &str) -> Result<(), String> {
        self.deadline.set(Some(Instant::now() + TIME));
        let result = self.context.with(|ctx| {
            let deliver: Function = ctx
                .globals()
                .get("__pb_deliver")
                .map_err(|e| e.to_string())?;
            deliver
                .call::<_, ()>((message.to_owned(),))
                .catch(&ctx)
                .map_err(|e| e.to_string())
        });
        let jobs = self.run_jobs();
        self.deadline.set(None);
        result.and(jobs)
    }

    /// What the plugin posted since the last call.
    pub fn take_posted(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.outbox.borrow_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, answer, event};

    /// Answers every call with `reply(name, args)` until the plugin is quiet; returns the
    /// calls and logs seen.
    fn serve(
        engine: &mut Engine,
        reply: impl Fn(&str, &[serde_json::Value]) -> Result<serde_json::Value, String>,
    ) -> Vec<Message> {
        let mut seen = Vec::new();
        loop {
            let posted = engine.take_posted();
            if posted.is_empty() {
                return seen;
            }
            for text in posted {
                let message = Message::parse(&text).unwrap();
                if let Message::Call { id, name, args } = &message {
                    engine.deliver(&answer(*id, reply(name, args))).unwrap();
                }
                seen.push(message);
            }
        }
    }

    #[test]
    fn calls_become_messages_and_answers_settle_them() {
        let mut engine = Engine::start(
            "main.js",
            Box::new(|| {}),
            r#"
            paintboard.ui.addCommand({ id: "go", title: "Go" }, async () => {
                const objects = await paintboard.board.getObjects();
                const ids = await paintboard.board.create(objects.map(o => ({ type: "sticky", text: o.text + "!" })));
                console.log("made", ids);
                try { await paintboard.board.remove("nope"); } catch (e) { console.warn(e.message); }
            });
            "#,
        )
        .unwrap();
        let seen = serve(&mut engine, |_, _| Ok(serde_json::Value::Null));
        assert!(matches!(&seen[..], [Message::Call { name, .. }] if name == "ui.addCommand"));

        engine
            .deliver(&event("command", serde_json::json!({ "id": "go" })))
            .unwrap();
        let seen = serve(&mut engine, |name, args| match name {
            "board.getObjects" => Ok(serde_json::json!([{ "text": "a" }, { "text": "b" }])),
            "board.create" => {
                assert_eq!(args[0][1]["text"], "b!");
                Ok(serde_json::json!(["1@1", "2@1"]))
            }
            _ => Err("no such object".into()),
        });
        let logs: Vec<_> = seen
            .iter()
            .filter_map(|m| match m {
                Message::Log { level, text } => Some((level.as_str(), text.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            logs,
            [
                ("info", r#"made ["1@1","2@1"]"#),
                ("warn", "no such object")
            ]
        );
    }

    #[test]
    fn errors_are_reported_not_fatal() {
        let mut engine = Engine::start(
            "main.js",
            Box::new(|| {}),
            r#"paintboard.on("selectionchange", () => { throw new Error("oops"); });
               paintboard.on("boardchange", async () => { null.x; });"#,
        )
        .unwrap();
        engine
            .deliver(&event("selectionchange", serde_json::json!([])))
            .unwrap();
        engine
            .deliver(&event("boardchange", serde_json::Value::Null))
            .unwrap();
        let errors: Vec<_> = engine
            .take_posted()
            .iter()
            .map(|t| Message::parse(t).unwrap())
            .collect();
        assert_eq!(errors.len(), 2);
        assert!(
            errors
                .iter()
                .all(|m| matches!(m, Message::Log { level, .. } if level == "error"))
        );
        // A script that does not even start says why.
        let err = Engine::start("main.js", Box::new(|| {}), "this is not javascript")
            .err()
            .unwrap();
        assert!(err.contains("main.js"), "{err}");
    }

    #[test]
    fn runaway_plugins_are_stopped() {
        let mut engine = Engine::start(
            "main.js",
            Box::new(|| {}),
            r#"paintboard.on("boardchange", () => { for (;;) {} });
               paintboard.on("selectionchange", () => { const a = []; for (;;) a.push(new Array(1e5)); });"#,
        )
        .unwrap();
        let started = Instant::now();
        assert!(
            engine
                .deliver(&event("boardchange", serde_json::Value::Null))
                .is_err()
        );
        assert!(started.elapsed() < TIME * 2);
        // Running out of memory is an error in the plugin, which goes to its log; once the
        // garbage is gone it works on.
        engine.take_posted();
        engine
            .deliver(&event("selectionchange", serde_json::Value::Null))
            .unwrap();
        let posted = engine.take_posted();
        assert!(posted.iter().any(|t| t.contains("memory")), "{posted:?}");
        assert!(Engine::start("main.js", Box::new(|| {}), "for (;;) {}").is_err());
    }

    #[test]
    fn plugins_cannot_reach_the_system() {
        let mut engine = Engine::start(
            "main.js",
            Box::new(|| {}),
            r#"console.log(typeof require, typeof std, typeof os, typeof fetch, typeof Deno);"#,
        )
        .unwrap();
        let posted = engine.take_posted();
        assert_eq!(
            Message::parse(&posted[0]).unwrap(),
            Message::Log {
                level: "info".into(),
                text: "undefined undefined undefined undefined undefined".into()
            }
        );
    }
}
