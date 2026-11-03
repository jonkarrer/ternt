// Copying from a terminal: a dragged selection, ⌘C with nothing selected
// (keeps the clipboard), programs copying with OSC 52 (tmux, vim, Claude
// Code over ssh), and Option-drag while a program has the mouse.
const term = await t.invoke("new_terminal");
const text = () => t.read("term", `window.__termText(${term})`).catch(() => "");
const run = (cmd) => t.invoke("pty_write", { id: term, pane: 0, data: cmd + "\r" });
const clipboard = () => t.invoke("debug_clipboard");
const copy = () => t.invoke("debug_send_action", { action: "copy:" });
await t.until(async () => (await text()).length > 0, "the shell starts", 15000);
await t.invoke("debug_focus", { label: "term" });

await run("clear; echo copy-me-42");
await t.until(async () => /^copy-me-42$/m.test(await text()), "a line to copy");
// Drag across a screen row (1-based), from its start to past its end
const drag = async (row, alt = false) => {
  const [x, y] = JSON.parse(await t.read("term", `JSON.stringify(window.__termRow(${term}, ${row}))`));
  await t.invoke("debug_click", { label: "term", x, y, to: [x + 400, y], alt });
};
await drag(1);
await t.until(async () => (await t.read("term", `window.__termSelection(${term})`)).includes("copy-me-42"), "dragging selects");
await copy();
await t.until(async () => (await clipboard()).trim() === "copy-me-42", "⌘C copies the selection");

const [cx, cy] = JSON.parse(await t.read("term", `JSON.stringify(window.__termRow(${term}, 6))`));
await t.invoke("debug_click", { label: "term", x: cx + 200, y: cy });
await t.until(async () => !(await t.read("term", `window.__termSelection(${term})`)), "a click clears the selection");
await copy();
await t.sleep(300);
await t.assert((await clipboard()).trim() === "copy-me-42", "⌘C with nothing selected leaves the clipboard alone");

await run(`printf '\\033]52;c;%s\\007' "$(printf 'from-osc-52 ✓' | base64)"`);
await t.until(async () => (await clipboard()) === "from-osc-52 ✓", "a program copies with OSC 52");
await run(`clear; printf '\\033]52;c;?\\007'; echo done-query`);
await t.until(async () => /^done-query$/m.test(await text()), "asking to read the clipboard");
await t.sleep(300);
await t.assert(!(await text()).includes("from-osc"), "gets nothing back");

// A program takes the mouse: a plain drag goes to it, Option-drag selects
await run("clear; echo mouse-mode; printf '\\033[?1000h'");
await t.until(async () => /^mouse-mode$/m.test(await text()), "mouse reporting on");
await drag(1, true);
await t.until(async () => (await t.read("term", `window.__termSelection(${term})`)).includes("mouse-mode"), "Option-drag still selects");
await copy();
await t.until(async () => (await clipboard()).trim() === "mouse-mode", "and copies");
await run("printf '\\033[?1000l'");
await t.quit();
