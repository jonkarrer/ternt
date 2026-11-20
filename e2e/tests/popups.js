// New windows a page opens, by real clicks: each becomes a tab whose view
// WebKit loads the request in, so the page keeps a real window (to point
// somewhere after opening it blank, post a form into, talk to). ⌘-click
// opens a link in a tab behind; ⇧⌘-click shows it.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const byId = (id) => S().tabs.find((x) => x.id === id);
const F = "http://localhost:8765";
const page = await t.invoke("new_tab", { url: `${F}/popups.html` });
await t.until(() => tab()?.title === "Fixture Popups", "a page that opens windows");

async function click(id, opts = {}) {
  await t.invoke("activate_tab", { id: page });
  await t.until(() => S().active === page, "on the page");
  await t.invoke("debug_eval", { label: `tab-${page}`, js: `(() => { const r = document.getElementById(${JSON.stringify(id)}).getBoundingClientRect(); document.title = "at " + (r.x + 5) + "," + (r.y + r.height / 2); })()` });
  await t.until(() => byId(page)?.title.startsWith("at "), `find #${id}`);
  const [x, y] = byId(page).title.slice(3).split(",").map(Number);
  await t.invoke("debug_eval", { label: `tab-${page}`, js: `document.title = "Fixture Popups"` });
  await t.until(() => byId(page)?.title === "Fixture Popups", "title back");
  const before = S().tabs.map((x) => x.id);
  await t.invoke("debug_click", { label: `tab-${page}`, x, y, ...opts });
  return before;
}
const added = (before) => S().tabs.filter((x) => !before.includes(x.id));

let before = await click("then-set");
await t.until(() => added(before).length === 1 && tab()?.url === `${F}/page2.html?then-set`, "a window opened blank, then pointed at a page, shows that page in a new tab");
await t.until(() => tab()?.title === "Fixture Page 2", "loaded");
await t.invoke("debug_eval", { label: `tab-${S().active}`, js: `document.title = "history " + history.length` });
await t.until(() => tab()?.title.startsWith("history "), "read its history");
await t.assert(tab().title === "history 1", `the page is its first (${tab().title})`);
await t.until(() => tab()?.canGoBack, "which can go back");
await t.invoke("go_back");
await t.until(() => S().active === page && added(before).length === 0, "Back closes it and returns to the page");

before = await click("blank-fetch");
await t.until(() => added(before).length === 1 && tab()?.url === `${F}/page2.html?blank-fetch`, "the same after a fetch");

before = await click("post");
await t.until(() => tab()?.title === "echo: POST q=posted" && added(before).length === 1, `a form posted into a new tab arrives as a POST (${tab()?.title})`);

before = await click("talk");
await t.until(() => added(before).length === 1 && tab()?.url === `${F}/page2.html?talk`, "window.open opens a tab");
await t.until(() => byId(page)?.title.startsWith("opened: "), "the page hears back");
await t.assert(byId(page).title === "opened: true", `and holds the window it opened (${byId(page).title})`);

// Later, from a timer: no longer the click's doing, as after a slow request
before = await click("closes");
await t.until(() => added(before).length === 1 && tab()?.title === "Closes itself", "a page a timer opens loads in a new tab");
await t.until(() => added(before).length === 0 && S().active === page, "and can close itself, back to the page");

before = await click("named");
await t.until(() => added(before).length === 1 && tab()?.url === `${F}/page2.html?named`, "a link to a named window opens a tab");
const named = S().active;
await t.invoke("navigate", { url: `${F}/index.html` });
await t.until(() => !tab()?.url.includes("named"), "which goes elsewhere");
before = await click("named");
await t.until(() => S().active === named && byId(named)?.url === `${F}/page2.html?named`, "the same link loads in that tab again, brought forward");
await t.assert(added(before).length === 0, "not a second one");

before = await click("plain", { keys: ["cmd"] });
await t.until(() => added(before).length === 1, "⌘-click opens a new tab");
const behind = added(before)[0].id;
await t.until(() => byId(behind)?.title === "Fixture Page 2", "loaded behind");
await t.assert(S().active === page && byId(page).url === `${F}/popups.html`, "while the page stays put");
before = await click("plain", { keys: ["cmd", "shift"] });
await t.until(() => added(before).length === 1 && tab()?.url === `${F}/page2.html?plain` && S().active !== page, "⇧⌘-click shows it");
await t.quit();
