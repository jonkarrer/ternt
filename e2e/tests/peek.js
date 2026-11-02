// With the sidebar hidden (⌘S), touching the window's left edge shows it over
// the page (in the "peek" view); leaving it hides it again. The page itself
// never moves.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const view = (label) => t.invoke("debug_view", { label });
const hover = (el, type) => el.dispatchEvent(new PointerEvent(type, { bubbles: false, pointerId: 1 }));
/** Run in the peek view. */
const inPeek = (js) => t.read("peek", js);
const hoverPeek = (type) =>
  t.eval("peek", `document.querySelector(".sidebar.peek").dispatchEvent(new PointerEvent("${type}", { pointerId: 1 }))`);

const page = await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home", "a page");
const ui = await view("ui");
await t.assert((await view("peek")).hidden, "the peek view waits hidden");

await t.invoke("menu_action", { id: "toggle-sidebar" });
await t.until(() => S().sidebarCollapsed && !document.querySelector(".sidebar"), "⌘S hides the sidebar");
await t.until(async () => (await view(`tab-${page}`)).x < 20, "the page takes the window");
const pageFrame = await view(`tab-${page}`);

// Touch the left edge
await t.until(() => document.querySelector(".peek-edge"), "a hover strip along the left edge");
hover(document.querySelector(".peek-edge"), "pointerenter");
await t.until(() => S().sidebarPeeking, "the sidebar peeks out");
await t.assert(S().sidebarCollapsed, "it's still collapsed (just showing)");
await t.until(async () => (await inPeek(`!!document.querySelector(".sidebar.peek .omnibox, .sidebar.peek")`)) === "true", "the peek view draws the sidebar");
await t.until(async () => (await inPeek(`document.querySelector(".sidebar.peek").innerText.includes("Fixture Home")`)) === "true", "with its tabs");
await t.until(async () => {
  const [v, p] = [await view("peek"), await view(`tab-${page}`)];
  return !v.hidden && v.stack > p.stack && v.width === S().sidebarWidth + 24 && v.x === 0;
}, "it floats over the page at the left edge, the sidebar's width");
let p = await view(`tab-${page}`);
await t.assert(p.x === pageFrame.x && p.width === pageFrame.width, "the page doesn't move");
const u = await view("ui");
await t.assert(u.width === ui.width && u.stack < p.stack, "the main sidebar view stays put, under the page");
await t.sleep(400);
await t.shot("peek");

// Using it: a new tab's view must not cover it
const other = await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2", "a tab opened while peeking");
await t.until(async () => (await view("peek")).stack > (await view(`tab-${other}`)).stack, "the sidebar stays on top of the new view");

// Moving back onto the sidebar cancels hiding; leaving hides it
await hoverPeek("pointerleave");
await t.sleep(100);
await hoverPeek("pointerenter");
await t.sleep(400);
await t.assert(S().sidebarPeeking, "coming back in time keeps it");
await hoverPeek("pointerleave");
await t.until(() => !S().sidebarPeeking, "leaving hides it");
await t.until(async () => (await view("peek")).hidden, "the peek view hides");

// ⌘S while peeking keeps it open for good
hover(document.querySelector(".peek-edge"), "pointerenter");
await t.until(() => S().sidebarPeeking, "peeking again");
await t.invoke("menu_action", { id: "toggle-sidebar" });
await t.until(() => !S().sidebarCollapsed && !S().sidebarPeeking, "⌘S keeps it open");
await t.until(async () => (await view("peek")).hidden, "the peek view hides");
await t.until(async () => (await view(`tab-${other}`)).x > S().sidebarWidth - 1, "the page makes room again");
await t.until(() => document.querySelector(".sidebar:not(.peek)"), "the normal sidebar is back");
await t.quit();
