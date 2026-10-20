// The `paintboard` object plugins use, run before the plugin's own code. It turns calls
// into messages to the app and the app's answers into settled promises; the same file
// runs in QuickJS on the desktop (`__pb_post` is given by the app) and in a Web Worker in
// the browser (messages through `postMessage`).
//
// Plugin -> app: {"id": n, "call": "board.create", "args": [...]}, {"log": text, "level": ...}
// App -> plugin: {"id": n, "ok": value} or {"id": n, "error": text}, {"event": name, "data": ...}
(() => {
  "use strict";
  const post =
    typeof __pb_post === "function"
      ? __pb_post
      : (text) => self.postMessage(text);
  const pending = new Map();
  const listeners = new Map();
  const commands = new Map();
  let panelHandler = null;
  let next = 1;

  const call = (name, ...args) =>
    new Promise((resolve, reject) => {
      const id = next++;
      pending.set(id, { resolve, reject });
      post(JSON.stringify({ id, call: name, args }));
    });

  const show = (value) => {
    if (typeof value === "string") return value;
    if (value instanceof Error) return value.stack ? `${value}\n${value.stack}` : String(value);
    try {
      return JSON.stringify(value);
    } catch (_) {
      return String(value);
    }
  };
  const log = (level) => (...values) =>
    post(JSON.stringify({ log: values.map(show).join(" "), level }));
  const report = (error) => log("error")(error);
  globalThis.console = {
    log: log("info"),
    info: log("info"),
    debug: log("info"),
    warn: log("warn"),
    error: log("error"),
  };

  // Runs a plugin's handler; what it throws or rejects with goes to its log.
  const run = (handler, data) => {
    try {
      const result = handler(data);
      if (result && typeof result.catch === "function") result.catch(report);
    } catch (error) {
      report(error);
    }
  };

  const deliver = (text) => {
    const message = JSON.parse(text);
    if (message.event !== undefined) {
      if (message.event === "command") {
        const handler = commands.get(message.data && message.data.id);
        if (handler) run(handler, message.data);
      } else if (message.event === "panel") {
        if (panelHandler) run(panelHandler, message.data);
      }
      for (const handler of listeners.get(message.event) || []) run(handler, message.data);
      return;
    }
    const waiting = pending.get(message.id);
    if (!waiting) return;
    pending.delete(message.id);
    if ("error" in message) waiting.reject(new Error(message.error));
    else waiting.resolve(message.ok);
  };
  globalThis.__pb_deliver = deliver;
  if (typeof __pb_post !== "function") {
    self.onmessage = (event) => deliver(event.data);
    // In a worker the browser offers the network and storage: plugins go through the
    // app instead, which checks their permissions. (Best effort: the desktop's QuickJS
    // has none of these at all.)
    for (const name of [
      "fetch", "XMLHttpRequest", "WebSocket", "EventSource", "importScripts", "Worker",
      "SharedWorker", "indexedDB", "caches", "WebTransport", "BroadcastChannel",
    ]) {
      try {
        Object.defineProperty(globalThis, name, { value: undefined, writable: false, configurable: false });
      } catch (_) {}
    }
  }

  const list = (items) => (Array.isArray(items) ? items : [items]);

  globalThis.paintboard = Object.freeze({
    apiVersion: 1,
    /** Calls `handler` on "selectionchange" or "boardchange". */
    on(event, handler) {
      if (!listeners.has(event)) listeners.set(event, []);
      listeners.get(event).push(handler);
    },
    board: Object.freeze({
      getObjects: (filter) => call("board.getObjects", filter || {}),
      getObject: (id) => call("board.getObject", id),
      getSelection: () => call("board.getSelection"),
      select: (ids) => call("board.select", list(ids)),
      getViewport: () => call("board.getViewport"),
      zoomTo: (ids) => call("board.zoomTo", list(ids)),
      getTags: () => call("board.getTags"),
      create: (items) => call("board.create", list(items)),
      update: (id, props) => call("board.update", id, props),
      remove: (ids) => call("board.remove", list(ids)),
    }),
    ui: Object.freeze({
      /** Adds a command to the Plugins menu (and the object menu with `selection: true`). */
      addCommand(command, handler) {
        commands.set(command.id, handler);
        return call("ui.addCommand", command);
      },
      showPanel(panel, handler) {
        panelHandler = handler || null;
        return call("ui.showPanel", panel);
      },
      closePanel: () => call("ui.closePanel"),
      notify: (text) => call("ui.notify", String(text)),
    }),
    storage: Object.freeze({
      get: (key) => call("storage.get", String(key)),
      set: (key, value) => call("storage.set", String(key), value === undefined ? null : value),
    }),
    net: Object.freeze({
      fetch: (url, options) => call("net.fetch", String(url), options || {}),
    }),
    clipboard: Object.freeze({
      writeText: (text) => call("clipboard.writeText", String(text)),
    }),
  });
})();
