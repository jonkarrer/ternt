// Relaunch after session-1: everything comes back, lazily.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";

await t.until(() => S().tabs.length === 4, "four tabs restored");
const tabs = S().tabs;
await t.assert(tabs[0].pinned && tabs[0].url === `${F}/` && tabs.filter((x) => x.pinned).length === 1, "pinned tab restored first");
await t.assert(tabs[1].kind === "terminal", "terminal restored in place");
await t.assert(tabs.slice(2).map((x) => x.title).join() === "Fixture Page 2,SPA", "tab order restored");
await t.until(() => tab()?.title === "SPA" && tab()?.loaded, "active tab restored and loaded");
await t.assert(tabs.filter((x) => x.kind === "web" && x.id !== S().active).every((x) => !x.loaded), "background tabs stay unloaded (lazy)");
await t.assert(S().sidebarWidth === 300 && !S().sidebarCollapsed, "sidebar layout restored");

// Opening a background tab loads it
const page2 = tabs.find((x) => x.url.includes("page2"));
await t.invoke("activate_tab", { id: page2.id });
await t.until(() => tab()?.id === page2.id && tab()?.loaded && !tab()?.loading && tab()?.title === "Fixture Page 2", "lazy tab loads on activation");

// Restored terminal starts a shell when first shown
await t.invoke("activate_tab", { id: tabs[1].id });
await t.sleep(1500);
await t.invoke("pty_write", { id: tabs[1].id, data: "echo restored-$((3*3))\r" });
await t.sleep(600);
const text = await t.read("term", `window.__termText(${tabs[1].id})`);
await t.assert(text.includes("restored-9"), "restored terminal runs a shell");
await t.shot("session-2");
await t.quit();
