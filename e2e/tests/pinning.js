// Pinning a tab by dragging it into the pinned area, with or without folders.
const S = () => t.state();
const F = "http://localhost:8765";
const find = (title) => S().tabs.find((x) => x.title === title);
const pointer = (el, type, x, y) =>
  el.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y }));
const center = (el) => {
  const r = el.getBoundingClientRect();
  return [r.left + r.width / 2, r.top + r.height / 2];
};
const rowOf = (title) => [...document.querySelectorAll(".tab")].find((el) => el.textContent.includes(title));
const pinHint = () => document.querySelector(".pinned-list .hint");
async function dragTo(title, target, shot) {
  const row = rowOf(title);
  const [x, y] = center(row);
  pointer(row, "pointerdown", x, y);
  pointer(row, "pointermove", x, y + 8);
  await t.sleep(50);
  const el = target();
  if (!el) {
    pointer(row, "pointerup", x, y + 8);
    return false;
  }
  const [tx, ty] = center(el);
  pointer(row, "pointermove", tx, ty);
  await t.sleep(50);
  if (shot) await t.shot(shot);
  pointer(row, "pointerup", tx, ty);
  return true;
}

for (const page of ["page2.html", "spa/a", ""]) await t.invoke("new_tab", { url: `${F}/${page}` });
await t.until(() => S().tabs.length === 3 && S().tabs.every((x) => x.title), "three tabs");

// Nothing pinned yet: dragging shows a place to pin
await t.assert(await dragTo("Fixture Home", pinHint), "with nothing pinned, dragging shows where to pin");
await t.until(() => find("Fixture Home")?.zone === "pinned" && find("Fixture Home")?.folder === null, "and dropping there pins the tab");

// Only a folder (its own tab pinned loose moved into it): still a place to pin outside it
await t.invoke("menu_action", { id: "new-folder" });
await t.until(() => S().folders.length === 1 && document.querySelector(".folder .rename"), "a folder");
document.querySelector(".folder .rename").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
const folder = S().folders[0].id;
await dragTo("Fixture Home", () => document.querySelector(".folder"));
await t.until(() => find("Fixture Home")?.folder === folder, "the pinned tab moves into the folder");
await t.assert(await dragTo("SPA", pinHint, "pinning-outside-folder"), "with only a folder, dragging still shows where to pin outside it");
await t.until(() => find("SPA")?.zone === "pinned" && find("SPA")?.folder === null, "and dropping there pins the tab, not in the folder");
await t.assert(!pinHint(), "the hint goes once the drag ends");
await t.quit();
