//! Plugins in the browser: each runs in its own Web Worker, away from the page, so it
//! cannot touch the page or slow it down. Its messages arrive later; `wake` tells the
//! app to come and take them.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Blob, BlobPropertyBag, ErrorEvent, MessageEvent, Url, Worker};

pub struct Engine {
    worker: Worker,
    posted: Rc<RefCell<Vec<String>>>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(ErrorEvent)>,
}

fn message(value: &wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            value
                .dyn_ref::<js_sys::Error>()
                .map(|e| String::from(e.message()))
        })
        .unwrap_or_else(|| format!("{value:?}"))
}

impl Engine {
    /// Starts `source` (the plugin's script) after the prelude in a new worker; `wake` is
    /// called whenever it posts something.
    pub fn start(name: &str, wake: Box<dyn Fn()>, source: &str) -> Result<Self, String> {
        let script = format!("{}\n;\n{source}\n//# sourceURL={name}", crate::PRELUDE);
        let parts = js_sys::Array::of1(&script.into());
        let options = BlobPropertyBag::new();
        options.set_type("text/javascript");
        let blob =
            Blob::new_with_str_sequence_and_options(&parts, &options).map_err(|e| message(&e))?;
        let url = Url::create_object_url_with_blob(&blob).map_err(|e| message(&e))?;
        let worker = Worker::new(&url).map_err(|e| message(&e));
        let _ = Url::revoke_object_url(&url);
        let worker = worker?;
        let posted = Rc::new(RefCell::new(Vec::new()));
        let wake = Rc::new(wake);
        let (inbox, woken) = (posted.clone(), wake.clone());
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            if let Some(text) = event.data().as_string() {
                inbox.borrow_mut().push(text);
                woken();
            }
        });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        let inbox = posted.clone();
        let on_error = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
            let text = format!(
                "{}:{}: {}",
                event.filename(),
                event.lineno(),
                event.message()
            );
            inbox
                .borrow_mut()
                .push(serde_json::json!({ "log": text, "level": "error" }).to_string());
            wake();
        });
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        Ok(Self {
            worker,
            posted,
            _on_message: on_message,
            _on_error: on_error,
        })
    }

    /// Sends the plugin a message; its replies come later through `wake`.
    pub fn deliver(&mut self, message_text: &str) -> Result<(), String> {
        self.worker
            .post_message(&message_text.into())
            .map_err(|e| message(&e))
    }

    /// What the plugin posted since the last call.
    pub fn take_posted(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.posted.borrow_mut())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.worker.terminate();
    }
}
