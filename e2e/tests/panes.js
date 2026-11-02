// Terminal panes: one terminal tab split into several shells (⌘D, ⇧⌘D),
// laid out by the terminal view, still one tab in the sidebar.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const panes = () => tab()?.panes;
const ids = () => {
  const out = [];
  const walk = (n) => (n.kind === "pane" ? out.push(n.id) : (walk(n.first), walk(n.second)));
  walk(panes().root);
  return out;
};
const text = (pane) => t.read("term", `window.__termText(${term}, ${pane})`);
const run = (pane, cmd) => t.invoke("pty_write", { id: term, pane, data: cmd + "\r" });
async function waitFor(pane, needle, message, timeout = 10000) {
  await t.until(async () => (await text(pane).catch(() => "")).includes(needle), message, timeout);
}
/** The panes on screen: `{pane, x, y, w, h}`, and how many dividers. */
async function layout() {
  const json = await t.read(
    "term",
    `JSON.stringify({
      panes: [...document.querySelectorAll(".pane:not([hidden])")].map((el) => {
        const r = el.getBoundingClientRect();
        return { pane: +el.dataset.pane, x: r.x, y: r.y, w: r.width, h: r.height, dimmed: el.classList.contains("dimmed") };
      }),
      dividers: document.querySelectorAll(".divider").length,
    })`,
  );
  return JSON.parse(json);
}
const near = (a, b, slack = 2) => Math.abs(a - b) <= slack;

await t.invoke("menu_action", { id: "new-terminal" });
await t.until(() => tab()?.kind === "terminal", "a terminal tab");
const term = S().active;
await t.assert(panes()?.focused === 0 && ids().length === 1, "it starts as one pane");
await t.until(async () => (await text(0).catch(() => "")).length > 0, "the shell started", 15000);
await t.sleep(1000);
await run(0, "cd /tmp && echo first-$((1+1))");
await waitFor(0, "first-2", "the first pane runs commands");
const full = (await layout()).panes[0];

// ⌘D: a pane to the right, in the same folder, with the keyboard
await t.invoke("menu_action", { id: "split-right" });
await t.until(() => ids().length === 2, "⌘D splits the terminal");
const right = panes().focused;
await t.assert(right !== 0, "the new pane has the focus");
await t.assert(S().tabs.filter((x) => x.kind === "terminal").length === 1, "still one tab");
await t.until(async () => (await layout()).panes.length === 2, "both panes on screen");
let l = await layout();
let [a, b] = [l.panes.find((p) => p.pane === 0), l.panes.find((p) => p.pane === right)];
await t.assert(near(a.x, full.x) && near(b.x + b.w, full.x + full.w), `side by side, edge to edge (${a.x}+${a.w} | ${b.x}+${b.w})`);
await t.assert(near(a.w, b.w) && b.x > a.x + a.w, "halves, with a gap between");
await t.assert(near(a.h, full.h) && near(b.h, full.h), "full height");
await t.assert(l.dividers === 1, "one divider");
await t.assert(a.dimmed && !b.dimmed, "the pane without the keyboard is dimmed");
await t.until(async () => (await text(right).catch(() => "")).length > 0, "the new pane's shell started", 15000);
await t.sleep(1000);
await run(right, "echo here:$(pwd)");
await waitFor(right, "here:/private/tmp", "the new pane starts in the folder it split from");
await t.assert(!(await text(right)).includes("first-2"), "panes don't share output");

// ⇧⌘D: the right pane splits into two stacked ones
await t.invoke("menu_action", { id: "split-down" });
await t.until(() => ids().length === 3, "⇧⌘D splits again");
const bottom = panes().focused;
await t.assert(JSON.stringify(ids()) === JSON.stringify([0, right, bottom]), "reading order: left, top right, bottom right");
await t.until(async () => (await layout()).panes.length === 3, "three panes on screen");
l = await layout();
const [top, low] = [l.panes.find((p) => p.pane === right), l.panes.find((p) => p.pane === bottom)];
await t.assert(near(top.x, low.x) && near(top.w, low.w), "stacked in the right column");
await t.assert(near(top.y, full.y) && near(low.y + low.h, full.y + full.h) && low.y > top.y + top.h, "top and bottom halves");
await t.assert(l.dividers === 2, "two dividers");
await t.sleep(1500);
await t.shot("panes-three");

// The tab is named after the focused pane
await run(0, "printf '\\033]2;left-title\\007'");
await run(bottom, "printf '\\033]2;bottom-title\\007'");
await t.until(() => tab()?.title === "bottom-title", "the focused pane names the tab");
await t.sleep(300);
await t.assert(tab()?.title === "bottom-title", "an unfocused pane's title doesn't");

// ⌘] and ⌘[ move between panes (Back and Forward don't apply to terminals)
await t.invoke("menu_action", { id: "forward" });
await t.until(() => panes().focused === 0, "⌘] wraps to the first pane");
await t.until(() => tab()?.title === "left-title", "the tab takes that pane's title");
await t.invoke("menu_action", { id: "back" });
await t.until(() => panes().focused === bottom, "⌘[ goes back");

// Clicking into a pane focuses it
await t.invoke("debug_focus", { label: "term" });
await t.eval("term", `document.querySelector('.pane[data-pane="${right}"] textarea').focus()`);
await t.until(() => panes().focused === right, "focusing a pane tells Rust");
await t.until(async () => (await layout()).panes.find((p) => p.pane === bottom).dimmed, "the others dim");

// Dragging the divider resizes (the ratio is clamped)
const rootSplit = panes().root.id;
await t.assert((await t.invoke("set_pane_ratio", { id: term, split: rootSplit, ratio: 0.3 })) === 0.3, "ratio set");
await t.until(async () => {
  const p = (await layout()).panes.find((p) => p.pane === 0);
  return near(p.w / (full.w - 13), 0.3, 0.01);
}, "the left pane takes 30%");
await t.assert((await t.invoke("set_pane_ratio", { id: term, split: rootSplit, ratio: 2 })) === 0.9, "clamped to 90%");
await t.invoke("set_pane_ratio", { id: term, split: rootSplit, ratio: 0.5 });

// ⌘W closes the focused pane; its neighbor takes the room
await t.invoke("menu_action", { id: "close-tab" });
await t.until(() => ids().length === 2, "⌘W closes the focused pane");
await t.assert(panes().focused === bottom, "the pane below takes the focus");
await t.assert(S().tabs.some((x) => x.id === term), "the tab stays");
await t.until(async () => {
  const p = (await layout()).panes.find((p) => p.pane === bottom);
  return p && near(p.h, full.h);
}, "the remaining right pane is full height");

// The terminal's sidebar row has split buttons too
const rowButton = (label) => document.querySelector(`.tab[data-tab="${term}"] button[aria-label="${label}"]`);
await t.assert(rowButton("Split right") && rowButton("Split down"), "the row shows split buttons");
await t.assert(!document.querySelector(`.tab:not(.terminal) button[aria-label="Split right"]`), "web tabs don't");
const before = ids().length;
rowButton("Split down").click();
await t.until(() => ids().length === before + 1, "clicking one splits the focused pane");
await t.until(() => {
  const n = panes().root;
  const walk = (x) => x.kind === "split" && (x.direction === "down" || walk(x.first) || walk(x.second));
  return walk(n);
}, "in that direction");
await t.sleep(300);
await t.shot("panes-row-buttons");
await t.invoke("menu_action", { id: "close-tab" });
await t.until(() => ids().length === before, "⌘W closes it again");

// `exit` closes a pane; the last one closes the tab
await run(bottom, "exit");
await t.until(() => ids().length === 1 && panes().focused === 0, "exiting a shell closes its pane");
await t.until(async () => {
  const l = await layout();
  return l.panes.length === 1 && near(l.panes[0].w, full.w) && l.dividers === 0;
}, "the last pane fills the tab again");
await t.assert((await text(0)).includes("first-2"), "the first pane kept its shell");
await t.sleep(300);
await t.shot("panes-one");
await run(0, "exit");
await t.until(() => !S().tabs.some((x) => x.id === term), "exiting the last shell closes the tab");
await t.quit();
