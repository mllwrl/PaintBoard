// Gathers sticky notes into a column per fill color, starting at the leftmost and topmost
// of them, with a heading over each column. Works on the selection, or on every sticky
// note when nothing is selected.

const GAP = 24;

async function arrange({ selection }) {
  let stickies = await paintboard.board.getObjects({ type: "sticky" });
  if (selection.length > 0) stickies = stickies.filter((s) => selection.includes(s.id));
  if (stickies.length === 0) {
    await paintboard.ui.notify("There are no sticky notes to arrange.");
    return;
  }
  const columns = new Map();
  for (const s of stickies) {
    if (!columns.has(s.fill)) columns.set(s.fill, []);
    columns.get(s.fill).push(s);
  }
  let x = Math.min(...stickies.map((s) => s.x));
  const top = Math.min(...stickies.map((s) => s.y));
  for (const [fill, notes] of columns) {
    notes.sort((a, b) => a.y - b.y || a.x - b.x);
    const width = Math.max(...notes.map((s) => s.width));
    await paintboard.board.create({
      type: "text",
      text: `${notes.length}`,
      x,
      y: top - 60,
      fontSize: 32,
      stroke: fill.length === 7 ? fill : "#1f2937",
    });
    let y = top;
    for (const note of notes) {
      await paintboard.board.update(note.id, { x, y });
      y += note.height + GAP;
    }
    x += width + GAP * 2;
  }
  await paintboard.board.select(stickies.map((s) => s.id));
}

paintboard.ui.addCommand(
  { id: "arrange", title: "Arrange stickies by color", selection: true },
  arrange,
);
