// Relaunch after session-1: spaces, zones and folders come back, lazily.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const find = (title) => S().tabs.find((x) => x.title === title);

await t.until(() => S().tabs.length === 4 && S().spaces.length === 2, "4 tabs in 2 spaces restored");
const work = S().spaces.find((s) => s.name === "Work");
await t.assert(work, "space name restored");
await t.assert(find("Fixture Home")?.zone === "favorite", "favorite restored");
const docs = S().folders.find((f) => f.name === "Docs");
await t.assert(docs && find("Fixture Page 2")?.folder === docs.id, "folder and its tab restored");
await t.assert(S().tabs.find((x) => x.kind === "terminal")?.space === work.id, "terminal restored in its space");
await t.until(() => tab()?.title === "SPA" && tab()?.loaded, "active tab restored and loaded");
await t.assert(S().tabs.filter((x) => x.kind === "web" && x.id !== S().active).every((x) => !x.loaded), "background tabs stay unloaded (lazy)");
await t.assert(S().sidebarWidth === 300 && !S().sidebarCollapsed, "sidebar layout restored");

// Opening a background tab loads it
await t.invoke("activate_tab", { id: find("Fixture Page 2").id });
await t.until(() => tab()?.loaded && !tab()?.loading && tab()?.title === "Fixture Page 2", "lazy tab loads on activation");

// Switching to Work restores its last tab: the terminal, with a fresh shell
await t.invoke("activate_space", { id: work.id });
await t.until(() => tab()?.kind === "terminal", "Work space shows its terminal");
const term = S().active;
await t.sleep(1500);
await t.invoke("pty_write", { id: term, data: "echo restored-$((3*3))\r" });
await t.sleep(600);
await t.assert((await t.read("term", `window.__termText(${term})`)).includes("restored-9"), "restored terminal runs a shell");
await t.shot("session-2");
await t.quit();
