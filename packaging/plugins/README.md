# Writing PaintBoard plugins

A plugin is a folder with a `manifest.json` and a JavaScript file. PaintBoard runs it in
its own sandbox: on the desktop in a built-in QuickJS engine, in the browser in a Web
Worker. The plugin sees the board only through the `paintboard` object, and only as far
as its permissions allow. The full API, with types, is in [`paintboard.d.ts`](paintboard.d.ts);
working examples are in [`examples/`](examples).

## A first plugin

`manifest.json`:

```json
{
  "id": "you.hello",
  "name": "Hello",
  "version": "1.0.0",
  "author": "You",
  "description": "Says hello on a sticky note.",
  "api": 1,
  "permissions": ["board:write"],
  "main": "main.js"
}
```

`main.js`:

```js
paintboard.ui.addCommand({ id: "hello", title: "Say hello" }, async () => {
  const [id] = await paintboard.board.create({ type: "sticky", text: "Hello!" });
  await paintboard.board.select(id);
});
```

In PaintBoard: **Plugins → Manage plugins… → Develop from a folder…** and pick the
`manifest.json`. The command shows in the **Plugins** menu. Every time you save a file of
the plugin, PaintBoard starts it again; `console.log` lines and errors go to the plugin's
**Log** in the same window.

To give it to others, send the folder. They install it with **Install plugin…** (in the
browser, by picking `manifest.json` together with the scripts).

## The manifest

| Field | |
|---|---|
| `id` | Unique: letters, digits, `.`, `-`, `_`, like `author.plugin-name`. Installing a plugin with the same id replaces it. |
| `name`, `version`, `author`, `description` | Shown in the plugins window and when installing. |
| `api` | The API version it is written for: `1`. Newer PaintBoard versions keep running older plugins; a plugin asking for a newer API than the app has is refused. |
| `permissions` | What it may do (below). It is asked once, when installing. |
| `main` | The script, in the plugin's folder (`main.js` by default). |

Permissions:

- `board:read`: read objects, the selection, the view and tags; get `selectionchange` and `boardchange` events.
- `board:write`: create, change and remove objects.
- `network:example.com,api.example.org`: `paintboard.net.fetch` from these hosts (and their subdomains).
- `clipboard`: put text on the clipboard.

A plugin without permissions can still add commands, show its panel, notify and keep
values in its storage.

## How it runs

- Every call returns a promise: use `await`.
- What a plugin changes while handling one command or panel event (and while starting)
  is **one step for Ctrl+Z**.
- Objects are plain data: `{id, type, x, y, width, height, rotation, fill, stroke, text, …}`.
  Change them with `paintboard.board.update(id, {…})`, not by editing the object.
- On a shared board, a plugin's changes reach the others like anyone's edits. Only the
  person running it needs the plugin.
- A plugin that runs for over 2 seconds at once, or uses over 64 MB, is stopped; errors
  are written to its log, and it can be started again by switching it off and on.
- On the desktop the engine has no file, network or system access at all. In the browser
  the worker's own network functions are removed, but browsers isolate workers less
  strictly: install plugins only from people you trust.

## Panels

```js
await paintboard.ui.showPanel(
  {
    title: "Search",
    items: [
      { type: "input", id: "q", label: "Text to find" },
      { type: "button", id: "go", label: "Find" },
      { type: "list", id: "found", items: ["…"] },
    ],
  },
  async ({ button, list, index, values }) => {
    if (button === "go") { /* values.q is what was typed */ }
  },
);
```

Item types: `text`, `separator`, `input` (`multiline` for several lines), `button`, `list`.
Showing the panel again replaces it (to update a list, say); `closePanel()` hides it.
