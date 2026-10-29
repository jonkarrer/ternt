// Every context-menu and command-bar action, through the same action ids the
// native menus send (`ctx-<action>:<id>[:<arg>]`, see src-tauri/src/menu.rs).
// The native menus themselves can't be opened from a test (they're modal).
const S = () => t.state();
const { F } = t;
const act = (id) => t.invoke("menu_action", { id });
const get = (id) => S().tabs.find((x) => x.id === id);
const space = (id) => S().spaces.find((s) => s.id === id);

const a = await t.invoke("new_tab", { url: `${F}/page2.html` });
const b = await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => get(a)?.title === "Fixture Page 2" && get(b)?.title === "Fixture Home", "two tabs");

// Copy URL (command bar) and Copy Link (tab menu)
await act("copy-url");
await t.until(async () => (await t.invoke("debug_clipboard")) === `${F}/`, "Copy URL copies the active tab's address");
await act(`ctx-copy-link:${a}`);
await t.until(async () => (await t.invoke("debug_clipboard")) === `${F}/page2.html`, "Copy Link copies that tab's address");

// Pin / unpin / favorite / unfavorite
await act(`ctx-pin:${a}`);
await t.until(() => get(a)?.zone === "pinned", "Pin Tab");
await act(`ctx-today:${a}`);
await t.until(() => get(a)?.zone === "today", "Unpin Tab");
await act(`ctx-favorite:${a}`);
await t.until(() => get(a)?.zone === "favorite" && document.querySelector(`.fav[data-tab="${a}"]`), "Add to Favorites");
await act(`ctx-today:${a}`);
await t.until(() => get(a)?.zone === "today", "Remove from Favorites");

// Folders: new folder from a tab (starts renaming), move between folders, out again
await act(`ctx-new-folder:${a}`);
await t.until(() => S().folders.length === 1 && get(a)?.folder === S().folders[0].id, "New Folder files the tab");
await t.until(() => document.querySelector(".folder .rename"), "and starts renaming it");
const rename = document.querySelector(".folder .rename");
rename.value = "Reading";
rename.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
await t.until(() => S().folders[0].name === "Reading", "folder renamed inline");
const reading = S().folders[0].id;
const second = await t.invoke("create_folder");
await act(`ctx-move-folder:${a}:${second}`);
await t.until(() => get(a)?.folder === second, "Move to Folder");
await act(`ctx-pin:${a}`);
await t.until(() => get(a)?.zone === "pinned" && get(a)?.folder === null, "Remove from Folder");
await act(`ctx-rename-folder:${reading}`);
await t.until(() => document.querySelector(".folder .rename"), "Rename Folder starts inline rename");
document.querySelector(".folder .rename").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
await t.until(() => !document.querySelector(".rename") && S().folders.find((f) => f.id === reading)?.name === "Reading", "escape cancels a rename");
await act(`ctx-delete-folder:${second}`);
await act(`ctx-delete-folder:${reading}`);
await t.until(() => S().folders.length === 0, "Delete Folder");

// Unload a pinned tab: its page goes away, the tab stays; showing it reloads
await act(`ctx-close:${a}`);
await t.until(() => get(a) && !get(a).loaded, "closing a pinned tab unloads it");
await t.invoke("activate_tab", { id: a });
await t.until(() => get(a)?.loaded && get(a)?.title === "Fixture Page 2", "showing it loads it again");

// Duplicate, then close the copy for real
const before = S().tabs.length;
await act(`ctx-duplicate:${b}`);
await t.until(() => S().tabs.length === before + 1, "Duplicate Tab");
const copy = S().tabs.find((x) => x.id !== a && x.id !== b);
await t.until(() => get(copy.id)?.url === `${F}/`, "the copy opens the same page");
await act(`ctx-close:${copy.id}`);
await t.until(() => !get(copy.id), "closing a Today tab removes it");

// Spaces: rename (inline), color, profile choices, delete
const work = await t.invoke("create_space");
await act(`ctx-rename-space:${work}`);
await t.until(() => document.querySelector(".space-header .rename"), "Rename Space starts inline rename");
const spaceInput = document.querySelector(".space-header .rename");
spaceInput.value = "Work";
spaceInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
await t.until(() => space(work)?.name === "Work", "space renamed");
await act(`ctx-space-color:${work}:purple`);
await t.until(() => space(work)?.color === "purple", "Color");
await act(`ctx-space-color:${work}:not-a-color`);
await t.sleep(100);
await t.assert(space(work)?.color === "purple", "unknown colors are ignored");
const own = space(work).profile;
await act(`ctx-space-profile:${work}:new`);
await t.until(() => space(work)?.profile && space(work).profile !== own, "New Profile replaces the old one");
await act(`ctx-space-profile:${work}:shared`);
await t.until(() => space(work)?.profile === null, "Shared Profile");
const home = S().spaces[0].id;
await act(`ctx-space-profile:${home}:space-${work}`);
await t.sleep(100);
await t.assert(space(home).profile === null, "sharing a space that uses the shared profile changes nothing");
await act(`ctx-delete-space:${work}`);
await t.until(() => !space(work) && S().activeSpace === home, "Delete Space");
await act(`ctx-delete-space:${home}`);
await t.sleep(100);
await t.assert(S().spaces.length === 1, "the last space can't be deleted");

// Command bar: clear Today keeps pinned tabs
await act("clear-today");
await t.until(() => S().tabs.every((x) => x.zone !== "today"), "Clear Today closes Today tabs");
await t.assert(get(a), "pinned tab survives");
await t.quit();
