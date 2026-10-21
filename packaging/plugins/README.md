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

To give it to others, press **Pack…** next to it: PaintBoard saves the plugin as one
`.pbplugin` file. People install it by double-clicking it, dragging it onto PaintBoard's
window, or with **Install plugin…**. A `.pbplugin` is an ordinary zip of the plugin's
folder, so any zip tool makes one too: zip the folder (or its contents) and rename the
`.zip` to `.pbplugin`. Only plain files are taken (no subfolders), up to 16 MB in all.

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
- `board:write`: create, change and remove objects; fill table cells, define tags, group
  and ungroup.
- `network:example.com,api.example.org`: `paintboard.net.fetch` from these hosts (and their subdomains).
- `clipboard`: put text on the clipboard.

A plugin without permissions can still add commands, show its panel, notify, play sounds,
wait, and keep values in its storage.

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

## Tables, tags, links and groups

```js
const [table] = await paintboard.board.create({
  type: "table",
  rows: [["Name", "Age"], ["Ann", "31"]], // optional: columnWidth, rowHeight
  tags: ["People"],                       // tags are made if the board has none by that name
  link: "https://example.com/people",     // or the id of an object to go to
});
await paintboard.board.setCell(table, 1, 1, "32");          // row and column from 0
const t = await paintboard.board.getObject(table);          // t.cells: [["Name", "Age"], ["Ann", "32"]]
await paintboard.board.defineTag({ name: "Done", color: "#22a06b" });
await paintboard.board.update(table, { tags: ["People", "Done"], link: null });
const group = await paintboard.board.group([table, other]);
await paintboard.board.ungroup(group);
```

A connector's `text` is its label, in `create` and `update` alike.

Stickies, shapes and text objects with lists in their text also have `paragraphs` (each
paragraph's `text`, `list` — `"bullet"`, `"number"` or `null` — and nesting `level`), and
those with links in their text have `links` (the linked `text` and its `link`, an address
or an object id). Both are read only: setting `text` replaces the text with plain text.

## Sounds and waiting

`paintboard.ui.playSound(name)` plays one of the built-in sounds: `"chime"`, `"beep"`,
`"done"` or `"alarm"`. `await paintboard.ui.after(ms)` waits (up to a day); `setTimeout`
and `clearTimeout` work too (on the desktop they are made from `after`). Waits end when
the plugin is switched off or reloaded. See [`examples/countdown`](examples/countdown).
