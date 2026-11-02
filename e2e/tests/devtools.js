// Developer Tools: web pages can be inspected (Inspect Element), and ⌥⌘I
// opens and closes the Web Inspector on the active page. Ternt's own views
// stay closed to it in release builds (debug builds can inspect everything).
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const view = (label) => t.invoke("debug_view", { label });

const page = await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home", "a page");
await t.assert((await view(`tab-${page}`)).inspectable, "pages can be inspected");
await t.assert(!(await view(`tab-${page}`)).devtools, "the inspector starts closed");

const frame = await view(`tab-${page}`);
await t.invoke("menu_action", { id: "devtools" });
await t.until(async () => (await view(`tab-${page}`)).devtools, "⌥⌘I opens the Web Inspector");
await t.sleep(800);
const after = await view(`tab-${page}`);
await t.assert(after.x === frame.x && after.width === frame.width && after.height === frame.height, "in its own window: the page keeps its place beside the sidebar");
await t.shot("devtools");
await t.invoke("menu_action", { id: "devtools" });
await t.until(async () => !(await view(`tab-${page}`)).devtools, "⌥⌘I again closes it");

// A terminal has no page to inspect: nothing happens
await t.invoke("new_terminal");
await t.invoke("menu_action", { id: "devtools" });
await t.sleep(300);
await t.assert(!(await view(`tab-${page}`)).devtools, "not from a terminal tab");
await t.quit();
