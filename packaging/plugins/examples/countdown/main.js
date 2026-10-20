// A countdown: type the minutes, press Start, and when the time is up PaintBoard chimes
// and says so. Shows waiting with paintboard.ui.after, sounds, and a panel that changes
// as time goes by. It asks for no permissions: it does not touch the board.

// Each start counts up, so a countdown that was stopped or started again knows to end.
let run = 0;
let lastMinutes = "5";

function clock(seconds) {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

function showIdle(status) {
  const items = [
    { type: "input", id: "minutes", label: "Minutes", value: lastMinutes },
    { type: "button", id: "start", label: "Start" },
  ];
  if (status) items.push({ type: "text", text: status });
  return paintboard.ui.showPanel({ title: "Countdown", items }, onPanel);
}

function showRunning(seconds) {
  return paintboard.ui.showPanel(
    {
      title: "Countdown",
      items: [
        { type: "text", text: `Time left: ${clock(seconds)}` },
        { type: "button", id: "stop", label: "Stop" },
      ],
    },
    onPanel,
  );
}

async function onPanel({ button, values }) {
  if (button === "stop") {
    run++;
    await showIdle("Stopped.");
    return;
  }
  if (button !== "start") return;
  const minutes = Number(String(values.minutes || "").replace(",", "."));
  if (!(minutes > 0 && minutes <= 24 * 60)) {
    await paintboard.ui.notify("Type the minutes: a number above 0.");
    return;
  }
  lastMinutes = String(values.minutes).trim();
  const mine = ++run;
  // Counting whole seconds down, one wait at a time, keeps the panel up to date.
  for (let left = Math.max(1, Math.round(minutes * 60)); left > 0; left--) {
    await showRunning(left);
    await paintboard.ui.after(1000);
    if (run !== mine) return;
  }
  await paintboard.ui.playSound("chime");
  await paintboard.ui.notify("Time is up!");
  await showIdle("Time is up.");
}

paintboard.ui.addCommand({ id: "open", title: "Countdown…" }, () => showIdle(""));
