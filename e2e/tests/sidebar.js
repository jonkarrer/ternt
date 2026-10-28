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
await t.sleep(300);
await t.shot("sidebar-pinned");
await t.quit();
