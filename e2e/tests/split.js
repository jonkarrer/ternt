// Split view: two tabs side by side (a page and a terminal, or two pages).
// Rust places both panes; clicking into one makes its tab active.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const view = (label) => t.invoke("debug_view", { label });
const near = (a, b) => Math.abs(a - b) <= 1;
const INSET = 8;

const a = await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home", "page A");
const b = await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2", "page B");
const full = await view(`tab-${b}`);

// Open A beside B
await t.invoke("split_view", { id: a });
await t.until(() => S().splits.length === 1, "split created");
const split = S().splits[0];
await t.assert(split.left === b && split.right === a, "the active tab on the left, the other on the right");
await t.assert(S().active === b, "the active tab stays active");
let left = await view(`tab-${b}`);
let right = await view(`tab-${a}`);
await t.until(async () => !(await view(`tab-${a}`)).hidden, "both panes visible");
left = await view(`tab-${b}`);
right = await view(`tab-${a}`);
const room = full.width - INSET;
await t.assert(near(left.x, full.x) && near(left.width, Math.round(room / 2)), `left pane: half (${left.x}, ${left.width})`);
await t.assert(near(right.x, left.x + left.width + INSET) && near(right.x + right.width, full.x + full.width), `right pane: the rest (${right.x}, ${right.width})`);
await t.assert(near(left.height, full.height) && near(right.height, full.height), "full height");
await t.until(() => document.querySelectorAll(".tab .split-glyph").length === 2, "both rows marked as split");
await t.until(() => document.querySelector(".tab.beside")?.dataset.tab === String(a), "the other pane's row is highlighted");
await t.until(() => document.querySelector(".split-handle"), "divider between the panes");
await t.sleep(300);
await t.shot("split-pages");

// Resize: the divider ratio is clamped, both panes follow
await t.invoke("set_split_ratio", { ratio: 0.3 });
await t.until(async () => near((await view(`tab-${b}`)).width, Math.round(room * 0.3)), "left pane at 30%");
right = await view(`tab-${a}`);
await t.assert(near(right.x + right.width, full.x + full.width), "right pane still reaches the edge");
await t.assert((await t.invoke("set_split_ratio", { ratio: 0.01 })) === 0.2, "ratio clamped to 20%");

// Focus moving into the other pane makes it the active tab
await t.invoke("debug_focus", { label: `tab-${a}` });
await t.until(() => S().active === a, "focusing the right pane activates its tab");
await t.assert(!(await view(`tab-${b}`)).hidden, "the left pane stays on screen");

// Switching to another tab hides both panes; back to either shows both
const c = await t.invoke("new_tab", { url: `${F}/spa/x` });
await t.until(() => tab()?.title === "SPA", "a third tab");
await t.until(async () => (await view(`tab-${a}`)).hidden && (await view(`tab-${b}`)).hidden, "the split is hidden");
await t.until(async () => near((await view(`tab-${c}`)).width, full.width), "the third tab is full width");
await t.invoke("activate_tab", { id: b });
await t.until(async () => !(await view(`tab-${a}`)).hidden && !(await view(`tab-${b}`)).hidden, "both panes back");
await t.assert((await view(`tab-${c}`)).hidden, "the third tab hidden");

// Close Split View: the active tab gets the whole content area
await t.invoke("menu_action", { id: "unsplit" });
await t.until(() => S().splits.length === 0, "split closed");
await t.until(async () => near((await view(`tab-${b}`)).width, full.width) && (await view(`tab-${a}`)).hidden, "back to one full-width tab");

// A terminal beside a page (⌥⌘\)
await t.invoke("menu_action", { id: "split-terminal" });
await t.until(() => tab()?.kind === "terminal" && S().splits.length === 1, "a terminal opens beside the page");
const term = S().active;
await t.assert(S().splits[0].left === b && S().splits[0].right === term, "page left, terminal right");
await t.until(async () => {
  const v = await view("term");
  return !v.hidden && near(v.x, left.x + Math.round(room / 2) + INSET);
}, "the terminal view sits in the right pane");
await t.until(async () => (await t.read("term", `window.__termText(${term})`).catch(() => "")).length > 0, "the shell started", 15000);
await t.sleep(400);
await t.shot("split-terminal");

// Focusing the page pane keeps the terminal on screen
await t.invoke("debug_focus", { label: `tab-${b}` });
await t.until(() => S().active === b, "page pane active");
await t.assert(!(await view("term")).hidden, "terminal still shown beside it");
await t.invoke("debug_focus", { label: "term" });
await t.until(() => S().active === term, "terminal pane active again");

// Closing one pane gives the other the whole area
await t.invoke("close_tab", { id: term });
await t.until(() => S().splits.length === 0 && S().active === b, "closing the terminal ends the split");
await t.until(async () => near((await view(`tab-${b}`)).width, full.width) && (await view("term")).hidden, "page full width, terminal view hidden");

// Making a split tab a favorite ends the split
await t.invoke("split_view", { id: a });
await t.until(() => S().splits.length === 1, "split again");
await t.invoke("move_tab", { id: a, zone: "favorite", folder: null, index: null });
await t.until(() => S().splits.length === 0, "favorite ends the split");
await t.until(async () => near((await view(`tab-${b}`)).width, full.width), "back to full width");

// Resizing the window keeps the panes side by side, edge to edge
await t.invoke("move_tab", { id: a, zone: "today", folder: null, index: null });
await t.invoke("activate_tab", { id: b });
await t.invoke("split_view", { id: a });
await t.until(() => S().splits.length === 1, "split for the resize");
await t.resize(1000, 700);
await t.until(async () => {
  const [l, r] = [await view(`tab-${b}`), await view(`tab-${a}`)];
  const roomNow = l.width + r.width;
  return near(r.x, l.x + l.width + INSET) && near(l.width, Math.round(roomNow / 2)) && r.x + r.width < 1000;
}, "panes re-laid out after a window resize");
await t.resize(1280, 800);

// The command bar offers the split commands
await t.palette.open();
await t.palette.type("split");
await t.until(async () => (await t.palette.rows()).includes("Split View with New Terminal"), "command bar: Split View with New Terminal");
await t.palette.key("Escape");

await t.quit();
