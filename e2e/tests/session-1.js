// Builds a session; session-2 relaunches and checks it was restored.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const byUrl = (part) => S().tabs.find((x) => x.url.includes(part));

await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home", "home loaded");
await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2", "page2 loaded");
await t.invoke("new_tab", { url: `${F}/spa/start` });
await t.until(() => tab()?.title === "SPA", "spa loaded");
await t.assert(S().tabs.map((x) => x.title).join() === "SPA,Fixture Page 2,Fixture Home", "newest tabs on top");

// Pin
const home = S().tabs.find((x) => x.url === `${F}/`).id;
await t.invoke("pin_tab", { id: home, pinned: true });
await t.until(() => S().tabs[0].id === home && S().tabs[0].pinned, "pinned tab moves to the pinned group");

// Reorder: move page2 above spa
const page2 = byUrl("page2").id;
await t.invoke("move_tab", { id: page2, index: 1 });
await t.until(() => S().tabs[1].id === page2, "move_tab reorders");
await t.invoke("move_tab", { id: page2, index: 0 });
await t.assert(S().tabs[0].id === home, "regular tabs can't move into the pinned group");

// Closing a pinned tab unloads it but keeps it
await t.invoke("activate_tab", { id: home });
await t.until(() => S().active === home, "pinned tab active");
await t.invoke("menu_action", { id: "close-tab" });
await t.until(() => S().active !== home, "cmd+w on pinned tab switches away");
const pinnedTab = S().tabs.find((x) => x.id === home);
await t.assert(pinnedTab && !pinnedTab.loaded, "pinned tab stays, unloaded");
await t.invoke("activate_tab", { id: home });
await t.until(() => S().tabs.find((x) => x.id === home)?.loaded && tab()?.title === "Fixture Home", "unloaded pinned tab reloads on click");

// Sidebar collapse
await t.invoke("menu_action", { id: "toggle-sidebar" });
await t.until(() => S().sidebarCollapsed, "cmd+s collapses sidebar");
await t.sleep(300);
await t.shot("session-collapsed");
await t.invoke("menu_action", { id: "toggle-sidebar" });
await t.until(() => !S().sidebarCollapsed, "cmd+s expands sidebar");
const width = await t.invoke("set_sidebar_width", { width: 300 });
await t.assert(width === 300, "sidebar width set");

// A terminal, then end on the spa tab
await t.invoke("new_terminal");
await t.until(() => tab()?.kind === "terminal", "terminal opened");
await t.invoke("activate_tab", { id: byUrl("/spa/").id });
await t.until(() => tab()?.title === "SPA", "spa active");
await t.sleep(300);
await t.shot("session-1");
await t.quit();
