// Renaming tabs inline: the name replaces the title in the sidebar until it's
// cleared, and survives the page (or shell) changing its own title.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const byId = (id) => S().tabs.find((x) => x.id === id);
const F = "http://localhost:8765";
const row = (id) => document.querySelector(`.tab[data-tab="${id}"]`);
const input = (id) => row(id)?.querySelector("input.rename");
const label = (id) => row(id)?.querySelector(".title")?.textContent.trim();
const key = (el, key) => el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
async function rename(id, name) {
  const field = input(id);
  field.value = name;
  key(field, "Enter");
  await t.until(() => !input(id), "the field closes");
}

const page = await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home", "a page");

// Double-click the title
row(page).querySelector(".title").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
await t.until(() => input(page), "double-clicking the title edits it");
await t.assert(input(page).value === "Fixture Home", "starting from the title");
await rename(page, "  My Page  ");
await t.until(() => byId(page).name === "My Page", "Enter saves the name (trimmed)");
await t.until(() => label(page) === "My Page", "the row shows the name");
await t.assert(byId(page).title === "Fixture Home", "the page keeps its own title");

// The page's title changing doesn't touch the name
await t.invoke("navigate", { url: `${F}/page2.html` });
await t.until(() => byId(page).title === "Fixture Page 2", "the page navigates");
await t.assert(label(page) === "My Page", "the name stays");

// The tab menu's Rename Tab…; Escape cancels
await t.invoke("menu_action", { id: `ctx-rename-tab:${page}` });
await t.until(() => input(page), "Rename Tab… edits it too");
input(page).value = "Nope";
key(input(page), "Escape");
await t.until(() => !input(page), "Escape closes the field");
await t.sleep(200);
await t.assert(byId(page).name === "My Page", "and keeps the old name");

// Empty goes back to the title
await t.invoke("menu_action", { id: `ctx-rename-tab:${page}` });
await t.until(() => input(page), "editing again");
await rename(page, "");
await t.until(() => byId(page).name === null && label(page) === "Fixture Page 2", "an empty name goes back to the title");

// Terminals: the shell's title doesn't override a name
const term = await t.invoke("new_terminal");
await t.until(() => row(term), "a terminal");
await t.invoke("menu_action", { id: `ctx-rename-tab:${term}` });
await t.until(() => input(term), "renaming a terminal");
await rename(term, "Server");
await t.until(() => label(term) === "Server", "the terminal is named");
await t.until(async () => (await t.read("term", `window.__termText(${term})`).catch(() => "")).length > 0, "its shell started", 15000);
await t.invoke("pty_write", { id: term, pane: 0, data: "printf '\\033]2;from-shell\\007'\r" });
await t.until(() => byId(term).title === "from-shell", "the shell sets its title");
await t.assert(label(term) === "Server", "the name still shows");
await t.until(() => document.querySelector(".omnibox")?.textContent.includes("Server"), "the top bar shows it too");
await t.sleep(200);
await t.shot("rename");

// Closing and reopening keeps the name
await t.invoke("activate_tab", { id: page });
await t.invoke("menu_action", { id: `ctx-rename-tab:${page}` });
await t.until(() => input(page), "naming the page again");
await rename(page, "Keeper");
await t.until(() => byId(page).name === "Keeper", "named");
await t.invoke("close_tab", { id: page });
await t.until(() => !byId(page), "closed");
await t.invoke("menu_action", { id: "reopen-tab" });
await t.until(() => tab()?.url.includes("page2.html") && tab()?.name === "Keeper", "reopened with its name");
await t.quit();
