// Pinned tabs and folders, driven through real DOM drag and drop.
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
async function dragTo(title, target) {
  const row = rowOf(title);
  const [x, y] = center(row);
  pointer(row, "pointerdown", x, y);
  pointer(row, "pointermove", x, y + 8); // starts the drag (drop zones appear)
  await t.sleep(50);
  const [tx, ty] = center(typeof target === "function" ? target() : target);
  pointer(row, "pointermove", tx, ty);
  await t.sleep(50);
  pointer(row, "pointerup", tx, ty);
}

for (const page of ["page2.html", "spa/a", ""]) await t.invoke("new_tab", { url: `${F}/${page}` });
await t.until(() => S().tabs.length === 3 && S().tabs.every((x) => x.title), "three tabs");

// A folder; drag a Today tab onto its header
await t.invoke("menu_action", { id: "new-folder" });
await t.until(() => S().folders.length === 1 && document.querySelector(".folder .rename"), "new folder starts renaming inline");
const input = document.querySelector(".folder .rename");
input.value = "Reading";
input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
await t.until(() => S().folders[0].name === "Reading", "inline rename commits on enter");
const folder = S().folders[0].id;

await dragTo("Fixture Home", document.querySelector(".folder"));
await t.until(() => find("Fixture Home")?.zone === "pinned" && find("Fixture Home")?.folder === folder, "drop on a folder pins into it");

// Drag a Today tab into the (empty) favorites area that appears while dragging
await dragTo("SPA", () => document.querySelector(".favorites"));
await t.until(() => find("SPA")?.zone === "favorite", "drop on the favorites area adds a favorite");

// Drag the remaining Today tab above the folder's tab
await dragTo("Fixture Page 2", () => rowOf("Fixture Home"));
await t.until(() => find("Fixture Page 2")?.folder === folder, "drop between pinned rows lands in that folder");
const inFolder = S().tabs.filter((x) => x.folder === folder).map((x) => x.title);
await t.assert(inFolder.join() === "Fixture Page 2,Fixture Home", `drop marker position respected (${inFolder})`);
// Rearranging inside the folder: drag the bottom tab above the top one
await dragTo("Fixture Home", () => rowOf("Fixture Page 2"));
await t.until(() => S().tabs.filter((x) => x.folder === folder).map((x) => x.title).join() === "Fixture Home,Fixture Page 2", "tabs reorder within a folder");
const row = rowOf("Fixture Home");
await t.assert(row.closest(".folder-group")?.dataset.folder === String(folder), "the folder's tabs sit inside its block");
const header = document.querySelector(".folder-group .folder").getBoundingClientRect();
await t.assert(row.firstElementChild.getBoundingClientRect().left > header.left + 30, "and are indented under it");
await t.sleep(250);
await t.shot("folders");

// Folders reorder by dragging their header
await t.invoke("menu_action", { id: "new-folder" });
await t.until(() => S().folders.length === 2 && document.querySelector(".folder .rename"), "a second folder");
const second = document.querySelector(".folder .rename");
second.value = "Later";
second.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
await t.until(() => S().folders[1]?.name === "Later", "named");
const names = () => S().folders.map((f) => f.name).join();
const headerOf = (name) => [...document.querySelectorAll(".folder")].find((el) => el.textContent.includes(name));
async function dragFolder(name, target, where) {
  const el = headerOf(name);
  const [x, y] = center(el);
  pointer(el, "pointerdown", x, y);
  pointer(el, "pointermove", x, y + 8);
  await t.sleep(50);
  const r = target.getBoundingClientRect();
  const ty = where === "before" ? r.top + 4 : r.bottom - 4;
  pointer(el, "pointermove", r.left + r.width / 2, ty);
  await t.sleep(50);
  await t.until(() => document.querySelector(`.folder-mark.${where}`), `drop marker ${where}`);
  pointer(el, "pointerup", r.left + r.width / 2, ty);
}
await dragFolder("Later", headerOf("Reading").closest(".folder-group"), "before");
await t.until(() => names() === "Later,Reading", "drag a folder above another");
await t.assert(find("Fixture Home")?.folder === folder, "its tabs go with it");
await dragFolder("Later", headerOf("Reading").closest(".folder-group"), "after");
await t.until(() => names() === "Reading,Later", "and back below");
const open = S().folders.find((f) => f.name === "Reading").open;
pointer(headerOf("Reading"), "pointerdown", ...center(headerOf("Reading")));
pointer(headerOf("Reading"), "pointerup", ...center(headerOf("Reading")));
await t.until(() => S().folders.find((f) => f.name === "Reading").open === !open, "a click (no drag) still opens or closes it");
await t.invoke("set_folder_open", { id: folder, open: true });
await t.invoke("remove_folder", { id: S().folders.find((f) => f.name === "Later").id });

// Collapsing hides the folder's tabs and skips them for ⌘1–9
await t.invoke("set_folder_open", { id: folder, open: false });
await t.until(() => !rowOf("Fixture Home"), "collapsed folder hides its tabs");
await t.invoke("menu_action", { id: "select-tab-9" });
await t.until(() => S().active === find("SPA").id, "cmd+9 skips collapsed folder tabs");

// Deleting the folder keeps its tabs pinned
await t.invoke("remove_folder", { id: folder });
await t.until(() => S().folders.length === 0, "folder deleted");
await t.assert(S().tabs.filter((x) => x.zone === "pinned" && x.folder === null).length === 2, "its tabs stay pinned");

// Terminals can't be pinned
const term = await t.invoke("new_terminal");
await t.invoke("move_tab", { id: term, zone: "pinned", folder: null, index: null });
await t.assert(S().tabs.find((x) => x.id === term).zone === "today", "terminals stay in Today");
await t.quit();
