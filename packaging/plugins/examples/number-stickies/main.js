// Numbers the selected sticky notes in reading order: rows top to bottom, left to right
// within a row. Running it again replaces the old numbers.

const NUMBER = /^\d+\.\s*/;

async function numberSelected({ selection }) {
  const objects = await Promise.all(selection.map((id) => paintboard.board.getObject(id)));
  const stickies = objects.filter((o) => o && o.type === "sticky");
  if (stickies.length === 0) {
    await paintboard.ui.notify("Select some sticky notes first.");
    return;
  }
  // Notes whose tops are within half a note's height are on one row.
  stickies.sort((a, b) => {
    const sameRow = Math.abs(a.y - b.y) < Math.min(a.height, b.height) / 2;
    return sameRow ? a.x - b.x : a.y - b.y;
  });
  let n = 1;
  for (const sticky of stickies) {
    const text = sticky.text.replace(NUMBER, "");
    await paintboard.board.update(sticky.id, { text: `${n++}. ${text}` });
  }
  await paintboard.ui.notify(`Numbered ${stickies.length} notes.`);
}

paintboard.ui.addCommand(
  { id: "number", title: "Number selected stickies", selection: true },
  numberSelected,
);
