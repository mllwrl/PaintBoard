// A panel where each line typed or pasted becomes a sticky note, laid out in a grid in
// the middle of the view. Shows panels, storage and creating many objects in one step.

const COLORS = { yellow: "#ffeb7a", green: "#b8f2b0", blue: "#a8d8ff", pink: "#ffc4dd" };
const SIZE = 200;
const GAP = 20;

async function showPanel() {
  const color = (await paintboard.storage.get("color")) || "yellow";
  await paintboard.ui.showPanel(
    {
      title: "Quick notes",
      items: [
        { type: "text", text: "One sticky note per line:" },
        { type: "input", id: "lines", multiline: true, placeholder: "Idea one\nIdea two" },
        { type: "input", id: "color", label: "Color (yellow, green, blue, pink)", value: color },
        { type: "button", id: "add", label: "Add notes" },
      ],
    },
    addNotes,
  );
}

async function addNotes({ button, values }) {
  if (button !== "add") return;
  const lines = (values.lines || "")
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
  if (lines.length === 0) {
    await paintboard.ui.notify("Type some lines first.");
    return;
  }
  const colorName = (values.color || "yellow").trim().toLowerCase();
  const fill = COLORS[colorName] || COLORS.yellow;
  await paintboard.storage.set("color", COLORS[colorName] ? colorName : "yellow");
  const view = await paintboard.board.getViewport();
  const perRow = Math.ceil(Math.sqrt(lines.length));
  const rows = Math.ceil(lines.length / perRow);
  const left = view.x + view.width / 2 - (perRow * (SIZE + GAP) - GAP) / 2;
  const top = view.y + view.height / 2 - (rows * (SIZE + GAP) - GAP) / 2;
  const ids = await paintboard.board.create(
    lines.map((text, i) => ({
      type: "sticky",
      text,
      fill,
      x: left + (i % perRow) * (SIZE + GAP),
      y: top + Math.floor(i / perRow) * (SIZE + GAP),
    })),
  );
  await paintboard.board.select(ids);
}

paintboard.ui.addCommand({ id: "open", title: "Quick notes…" }, showPanel);
