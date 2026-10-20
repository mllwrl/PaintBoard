// A panel where CSV text (or cells copied from a spreadsheet, which come separated by
// tabs) becomes a table in the middle of the view; or the address of a CSV file on
// GitHub, fetched. Shows tables, the network permission and panels.

const MAX_ROWS = 500;
const MAX_COLUMNS = 50;

/** The rows of `text`: cells separated by tabs, semicolons or commas (whichever the first
 * line has most of), with "quoted" cells that may hold separators, quotes ("") and lines. */
function parseCsv(text) {
  const first = text.split("\n", 1)[0];
  const count = (sep) => first.split(sep).length;
  const sep = [",", ";", "\t"].reduce((best, s) => (count(s) > count(best) ? s : best));
  const rows = [];
  let row = [];
  let cell = "";
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c !== '"') cell += c;
      else if (text[i + 1] === '"') {
        cell += '"';
        i++;
      } else quoted = false;
    } else if (c === '"' && cell === "") {
      quoted = true;
    } else if (c === sep) {
      row.push(cell);
      cell = "";
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
    } else {
      cell += c;
    }
  }
  row.push(cell);
  rows.push(row);
  return rows.filter((r) => r.some((c) => c.trim() !== ""));
}

async function makeTable(text) {
  let rows = parseCsv(text);
  if (rows.length === 0) {
    await paintboard.ui.notify("There is nothing to make a table of.");
    return;
  }
  if (rows.length > MAX_ROWS || rows.some((r) => r.length > MAX_COLUMNS)) {
    rows = rows.slice(0, MAX_ROWS).map((r) => r.slice(0, MAX_COLUMNS));
    await paintboard.ui.notify(`Only the first ${MAX_ROWS} rows and ${MAX_COLUMNS} columns were taken.`);
  }
  const [id] = await paintboard.board.create({ type: "table", rows });
  await paintboard.board.select(id);
}

async function onPanel({ button, values }) {
  if (button === "make") {
    await makeTable(values.csv || "");
  } else if (button === "fetch") {
    const url = (values.url || "").trim();
    if (!url) {
      await paintboard.ui.notify("Type the address of a CSV file first.");
      return;
    }
    try {
      const answer = await paintboard.net.fetch(url);
      if (answer.ok) await makeTable(answer.text);
      else await paintboard.ui.notify(`The file could not be fetched (${answer.status}).`);
    } catch (error) {
      await paintboard.ui.notify(error.message);
    }
  }
}

paintboard.ui.addCommand({ id: "open", title: "Table from CSV…" }, () =>
  paintboard.ui.showPanel(
    {
      title: "Table from CSV",
      items: [
        { type: "text", text: "Paste CSV, or cells copied from a spreadsheet:" },
        { type: "input", id: "csv", multiline: true, placeholder: "Name,Age\nAnn,31" },
        { type: "button", id: "make", label: "Make a table" },
        { type: "separator" },
        {
          type: "input",
          id: "url",
          label: "Or the address of a CSV file on GitHub",
          placeholder: "https://raw.githubusercontent.com/…/data.csv",
        },
        { type: "button", id: "fetch", label: "Fetch and make a table" },
      ],
    },
    onPanel,
  ),
);
