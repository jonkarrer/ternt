// Links that WebKit alone drops (dead clicks): downloads and links for other
// apps (mailto:, zoommtg:…). Tests save downloads under e2e/.out/downloads
// and record other-app links instead of opening them.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const click = (id, el) => t.invoke("debug_eval", { label: `tab-${id}`, js: `document.getElementById(${JSON.stringify(el)}).click()` });
const links = () => t.invoke("debug_other_app_links");
const downloads = () => t.invoke("debug_downloads");

const page = await t.invoke("new_tab", { url: `${F}/links.html` });
await t.until(() => tab()?.title === "Fixture Links", "a page of links");
const tabs = S().tabs.length;

await click(page, "download");
await t.until(async () => (await downloads()).some(([p, ok, text]) => p.endsWith("saved-page.html") && ok && text.includes("Fixture Page 2")), "a download link saves the file");
await click(page, "attachment");
await t.until(async () => (await downloads()).some(([p, ok, text]) => p.endsWith("report.csv") && ok && text === "attached file"), "so does a file the server sends as an attachment");
await t.assert(tab()?.url.endsWith("/links.html") && S().tabs.length === tabs, "the page stays put");

await click(page, "mail");
await t.until(async () => (await links()).includes("mailto:someone@example.com"), "mailto: goes to the mail app");
await click(page, "app");
await t.until(async () => (await links()).includes("ternt-test-app:open?x=1"), "another app's link goes to that app");
await click(page, "app-blank");
await t.until(async () => (await links()).includes("ternt-test-app:blank"), "even in a new window");
await t.sleep(300);
await t.assert(S().tabs.length === tabs && tab()?.url.endsWith("/links.html"), "without a tab or a navigation");

// A real click (a user gesture) on a target=_blank link opens a tab. Tauri's
// opener plugin script used to catch these and drop them.
await t.invoke("debug_eval", { label: `tab-${page}`, js: `(() => { const r = document.getElementById("blank").getBoundingClientRect(); document.title = "at " + (r.x + 5) + "," + (r.y + r.height / 2); })()` });
await t.until(() => tab()?.title.startsWith("at "), "find the new-tab link");
const [x, y] = tab().title.slice(3).split(",").map(Number);
await t.invoke("debug_click", { label: `tab-${page}`, x, y });
await t.until(() => tab()?.url.endsWith("/page2.html?blank") && S().tabs.length === tabs + 1, "clicking a target=_blank link opens a new tab");
await t.invoke("close_tab", { id: S().active });
await t.invoke("activate_tab", { id: page });

await click(page, "next");
await t.until(() => tab()?.title === "Fixture Page 2", "ordinary links still work");

// Pages get none of Tauri's scripts: its bridge locked window.ipc, which
// broke Google Docs, Sheets and Forms (they declare their own ipc).
await t.invoke("navigate", { url: `${F}/ipc.html` });
await t.until(() => tab()?.title.startsWith("ipc:"), "a page declaring its own global ipc runs");
await t.assert(tab().title === "ipc: page's own, tauri: undefined", `and sees no Tauri internals (${tab().title})`);
const fresh = await t.invoke("new_tab", { url: `${F}/ipc.html` });
await t.until(() => S().tabs.find((x) => x.id === fresh)?.title === "ipc: page's own, tauri: undefined", "the same in a new tab's first page");

// Pages see their own frame as the window: outerWidth was 0, and Google Docs
// (which scales its canvas by outer / inner width) drew blurry pages.
await t.invoke("debug_eval", { label: `tab-${fresh}`, js: `document.title = "outer " + (outerWidth === innerWidth && outerHeight >= innerHeight && innerWidth > 0)` });
await t.until(() => S().tabs.find((x) => x.id === fresh)?.title.startsWith("outer "), "read the page's window size");
await t.assert(S().tabs.find((x) => x.id === fresh).title === "outer true", "a page's outer size is its tab's, not 0");
await t.quit();
