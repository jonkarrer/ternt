const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "localhost:8765";
const request = () => t.invoke("palette_request");

// Drive the palette webview like a user would.
const P = (js) => t.eval("palette", js);
const type = (text) =>
  P(`(() => { const i = document.querySelector("input"); i.value = ${JSON.stringify(text)}; i.dispatchEvent(new Event("input", { bubbles: true })); })()`);
const key = (k) =>
  P(`document.querySelector("input").dispatchEvent(new KeyboardEvent("keydown", { key: ${JSON.stringify(k)}, bubbles: true }))`);
const openPalette = async (id) => {
  await t.invoke("menu_action", { id });
  await t.until(request, `${id} opens palette`);
  await t.sleep(250);
};

// New tab from the palette
await openPalette("new-tab");
await t.assert((await request()).mode === "newTab", "cmd+t opens in new-tab mode");
await t.shot("palette-empty");
await type(`${F}/`);
await key("Enter");
await t.until(async () => !(await request()), "enter closes palette");
await t.until(() => tab()?.title === "Fixture Home", "typed URL opened as tab");

await openPalette("new-tab");
await type(`${F}/page2.html`);
await key("Enter");
await t.until(() => S().tabs.length === 2 && tab()?.title === "Fixture Page 2", "second tab opened");

// Edit mode sits above the page webview and is prefilled
await openPalette("open-location");
const edit = await request();
await t.assert(edit.mode === "edit" && edit.url.endsWith("/page2.html"), "cmd+l prefills current URL");
await type("fixture home");
await t.sleep(100);
await t.shot("palette-over-page");
await key("Enter");
await t.until(() => tab()?.title === "Fixture Home", "title-matching tab ranks above search");

// Edit mode navigates the current tab in place
await openPalette("open-location");
await type(`${F}/spa/edited`);
await key("Enter");
await t.until(() => tab()?.url.endsWith("/spa/edited") && S().tabs.length === 2, "edit mode navigates in place");

// Commands by name
await openPalette("new-tab");
await type("close tab");
await key("ArrowDown");
await key("Enter");
await t.until(() => S().tabs.length === 1, "'Close Tab' command runs");

// Escape dismisses
await openPalette("new-tab");
await key("Escape");
await t.until(async () => !(await request()), "escape closes palette");

// Search
await openPalette("new-tab");
await type("tauri multiwebview");
await key("Enter");
await t.until(() => tab()?.url.startsWith("https://www.google.com/search?q=tauri"), "plain text searches");
// The bar centers over the content pane, with or without the sidebar
const centered = async (label) => {
  const r = JSON.parse(
    await t.read("palette", `JSON.stringify((() => { const p = document.querySelector(".panel").getBoundingClientRect(); return { mid: p.left + p.width / 2, vw: innerWidth }; })())`),
  );
  const left = S().sidebarCollapsed ? 8 : S().sidebarWidth;
  const expected = left + (r.vw - left - 8) / 2;
  await t.assert(Math.abs(r.mid - expected) < 2, `${label}: centered on content pane (${Math.round(r.mid)} vs ${Math.round(expected)})`);
};
await openPalette("new-tab");
await centered("sidebar shown");
await t.shot("palette-centered");
await key("Escape");
await t.invoke("menu_action", { id: "toggle-sidebar" });
await t.until(() => S().sidebarCollapsed, "sidebar collapsed");
await openPalette("new-tab");
await centered("sidebar hidden");
await key("Escape");
await t.quit();
