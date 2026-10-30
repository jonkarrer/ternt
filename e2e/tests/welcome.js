// First launch (no session file): the welcome page opens as a tab, and Help →
// Welcome to Ternt brings it back later. Every other suite starts from an
// empty session (see run.sh), so this is the only one that sees it.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const WELCOME = "ternt://welcome/";

await t.until(() => S().tabs.length === 1 && tab()?.url === WELCOME, "first launch opens the welcome page");
await t.until(() => tab()?.title === "Welcome to Ternt" && !tab().loading, "welcome page loaded");
await t.assert(!(await t.palette.request()), "the command bar stays closed on first launch");
await t.until(
  () => document.querySelector(".tabs .tab .bolt") != null && document.querySelector(".omnibox .host")?.textContent === "ternt://welcome",
  "sidebar shows the bolt icon and the page address",
);
await t.shot("welcome");

// Not a web page: history doesn't record it
await t.sleep(300);
const hits = await t.invoke("history_search", { query: "welcome", limit: 5 });
await t.assert(hits.length === 0, "welcome page is not in history");

// Help → Welcome to Ternt opens a fresh copy
await t.invoke("menu_action", { id: "welcome" });
await t.until(() => S().tabs.length === 2 && tab()?.url === WELCOME, "Help menu opens the welcome page again");

// Closing it does not reopen anything
await t.invoke("menu_action", { id: "close-tab" });
await t.invoke("menu_action", { id: "close-tab" });
await t.until(() => S().tabs.length === 0, "welcome tabs close like any tab");

await t.quit();
