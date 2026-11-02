// Sidebar interactions driven with real DOM pointer events.
const S = () => t.state();
const F = "http://localhost:8765";
const titles = () => S().tabs.map((x) => x.title).join(",");
const rows = () => [...document.querySelectorAll(".tabs .tab")];
const pointer = (el, type, y) =>
  el.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, pointerId: 1, clientX: 50, clientY: y }));

for (const page of ["page2.html", "spa/x", ""]) {
  await t.invoke("new_tab", { url: `${F}/${page}` });
}
await t.until(() => titles() === "Fixture Home,SPA,Fixture Page 2", "three tabs, newest first");

// Click activates
const second = rows()[1];
const y = second.getBoundingClientRect().top + 10;
pointer(second, "pointerdown", y);
pointer(second, "pointerup", y);
await t.until(() => S().active === S().tabs[1].id, "clicking a row activates it");

// Drag the top row down two places
const top = rows()[0];
const y0 = top.getBoundingClientRect().top + 10;
pointer(top, "pointerdown", y0);
for (let dy = 0; dy <= 88; dy += 11) pointer(top, "pointermove", y0 + dy);
await t.sleep(50);
await t.shot("sidebar-dragging");
pointer(top, "pointerup", y0 + 88);
await t.until(() => titles() === "SPA,Fixture Page 2,Fixture Home", "dragging reorders tabs");

// Pinned tabs render as tiles
await t.invoke("move_tab", { id: S().tabs.find((x) => x.title === "Fixture Page 2").id, zone: "favorite", folder: null, index: null });
await t.until(() => document.querySelectorAll(".favorites .fav").length === 1 && rows().length === 2, "favorite moves to the tile grid");
await t.until(
  () => document.querySelector(".favorites .fav img.site-icon")?.src.startsWith("data:image/svg+xml"),
  "the tile shows the site's app icon (its apple-touch-icon link)",
);
await t.sleep(300);
await t.shot("sidebar-pinned");

// "Clear" on the divider closes Today's web tabs, keeping terminals and favorites
await t.invoke("new_terminal");
await t.until(() => rows().length === 3, "terminal added to Today");
document.querySelector(".divider .clear").click();
await t.until(() => rows().length === 1 && S().tabs.filter((x) => x.zone === "today")[0]?.kind === "terminal", "clear keeps only the terminal");
await t.assert(document.querySelectorAll(".favorites .fav").length === 1, "favorite survives clear");
await t.invoke("close_tab", { id: S().tabs.find((x) => x.kind === "terminal").id });
await t.until(() => !document.querySelector(".divider .clear"), "clear hides with no Today web tabs");

// A short window: the tab list scrolls instead of running under the spaces
for (let i = 0; i < 12; i++) await t.invoke("new_tab", { url: `${t.F}/spa/n${i}` });
await t.until(() => S().tabs.filter((x) => x.zone === "today").length >= 12, "a dozen tabs");
await t.resize(900, 420);
await t.sleep(400);
const scroller = document.querySelector(".space-scroll");
const footer = document.querySelector("footer.spaces").getBoundingClientRect();
await t.assert(scroller.scrollHeight > scroller.clientHeight, "the list overflows its area");
await t.assert(scroller.getBoundingClientRect().bottom <= footer.top + 1, "and stops above the spaces");
const newTab = [...document.querySelectorAll(".row.action")].find((el) => el.textContent.includes("New Tab")).getBoundingClientRect();
await t.assert(newTab.bottom <= footer.top || newTab.top >= scroller.getBoundingClientRect().bottom, "New Tab never sits under the spaces");
scroller.scrollTop = scroller.scrollHeight;
await t.sleep(100);
const last = [...document.querySelectorAll(".tabs .tab")].at(-1).getBoundingClientRect();
await t.assert(last.bottom <= footer.top + 1, "scrolling reaches the last tab");
await t.shot("sidebar-short");
await t.assert(!document.querySelector("button.update, .update-row"), "no update pill (the card is all)");
await t.quit();
