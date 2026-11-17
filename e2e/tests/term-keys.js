// Keys a terminal translates: Shift+Enter types a newline (ESC + CR, as
// Option+Enter) instead of submitting, so Claude Code's prompt gets a new line;
// ⌘+ / ⌘− / ⌘0 size the text of every terminal, and the size is saved; ⌃F
// and ⌃H go to the shell (forward-char, backspace) rather than starting
// Explore Tabs or the help sheet.
const S = () => t.state();
const term = await t.invoke("new_terminal");
const text = () => t.read("term", `window.__termText(${term})`).catch(() => "");
const run = (cmd) => t.invoke("pty_write", { id: term, pane: 0, data: cmd + "\r" });
const press = (init) =>
  t.read("term", `(() => { const el = document.querySelector(".xterm-helper-textarea"); for (const type of ["keydown", "keyup"]) el.dispatchEvent(new KeyboardEvent(type, { bubbles: true, cancelable: true, ...${JSON.stringify(init)} })); return el.tagName; })()`);
await t.until(async () => (await text()).length > 0, "the shell starts", 15000);
await t.invoke("debug_focus", { label: "term" });

// `od -c` shows exactly the bytes a key sends
await run("clear; stty raw -echo; head -c 2 | od -c; stty sane; echo keys-done");
await t.sleep(500);
await t.assert((await press({ key: "Enter", code: "Enter", keyCode: 13, shiftKey: true })) === "TEXTAREA", "keys go to the terminal");
await t.until(async () => /keys-done/.test(await text()), "the key arrives");
const out = await text();
await t.assert(/033\s+\\r/.test(out), `Shift+Enter sends ESC + CR (${out.split("\n").find((l) => l.startsWith("0000000"))})`);

// Zoom: bigger text means fewer columns
const font = async (id = term) => JSON.parse(await t.read("term", `JSON.stringify(window.__termFont(${id}) ?? null)`));
const size = async () => (await t.invoke("get_prefs")).terminalFontSize;
await t.assert((await size()) === 13, "text starts at 13px");
const before = await font();
await press({ key: "=", code: "Equal", metaKey: true });
await press({ key: "=", code: "Equal", metaKey: true });
await t.until(async () => (await size()) === 15, "⌘+ twice: 15px, saved");
await t.until(async () => (await font())?.size === 15, "the terminal draws at 15px");
await t.until(async () => (await font()).cols < before.cols, `with fewer columns (${before.cols} → ${(await font()).cols})`);
await press({ key: "-", code: "Minus", metaKey: true });
await t.until(async () => (await size()) === 14, "⌘− makes it smaller");
const other = await t.invoke("new_terminal");
await t.until(async () => (await font(other))?.size === 14, "a new terminal starts at that size");
await press({ key: "0", code: "Digit0", metaKey: true });
await t.until(async () => (await size()) === 13, "⌘0 goes back to 13px");

// ⌃F, pressed as a key: the terminal gets it, Explore Tabs doesn't start
const exploring = () => !!document.querySelector(".app.exploring");
await t.invoke("activate_tab", { id: term });
await t.invoke("debug_focus", { label: "term" });
await run("clear; stty raw -echo; head -c 1 | od -c; stty sane; echo ctrl-f-done");
await t.sleep(500);
await t.invoke("debug_ctrl_key", { key: "f" });
await t.until(async () => /ctrl-f-done/.test(await text()), "⌃F reaches the shell");
await t.assert(/006/.test(await text()), "as ^F (forward-char)");
await t.assert(!exploring(), "without exploring tabs");
// ⌃H likewise: backspace in the shell, not the help sheet
const helping = async () => (await t.palette.request())?.mode === "help";
await run("clear; stty raw -echo; head -c 1 | od -c; stty sane; echo ctrl-h-done");
await t.sleep(500);
await t.invoke("debug_ctrl_key", { key: "h" });
await t.until(async () => /ctrl-h-done/.test(await text()), "⌃H reaches the shell");
await t.assert(/\\b/.test(await text()), "as ^H (backspace)");
await t.assert(!(await helping()), "without opening the help sheet");
// Anywhere else it explores tabs
const page = await t.invoke("new_tab", { url: "http://localhost:8765/" });
await t.until(() => S().tabs.find((x) => x.id === page)?.title === "Fixture Home", "on a page");
await t.invoke("debug_focus", { label: `tab-${page}` });
await t.invoke("debug_ctrl_key", { key: "f" });
await t.until(exploring, "⌃F explores tabs");
await t.invoke("menu_action", { id: "explore" });
await t.until(() => !exploring(), "stop exploring");
await t.invoke("debug_focus", { label: `tab-${page}` });
await t.invoke("debug_ctrl_key", { key: "h" });
await t.until(helping, "⌃H opens the help sheet");
await t.quit();
