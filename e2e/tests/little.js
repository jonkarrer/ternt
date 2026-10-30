// Little Ternt: links from other apps (Ternt as the default browser) open in
// a small window with a bar on top; "Open in Ternt" moves the page into a
// tab. Settings → Links can send them to tabs instead.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const windows = () => t.invoke("debug_windows");
const littles = async () => (await windows()).filter((w) => w.startsWith("little-")).sort();
const bar = (label, js) => t.read(`${label}-bar`, js).catch(() => "");

await t.assert((await t.invoke("get_prefs")).externalLinks === "little", "links from other apps go to Little Ternt by default");

// A link from another app opens a Little Ternt window, not a tab
await t.invoke("debug_open_external", { url: `${F}/page2.html` });
await t.until(async () => (await littles()).length === 1, "a Little Ternt window opens");
const [little] = await littles();
await t.assert(S().tabs.length === 0, "no tab was opened");
await t.until(async () => (await bar(little, `document.querySelector(".title")?.textContent`)) === "Fixture Page 2", "the bar shows the page title");
await t.assert((await bar(little, `document.querySelector(".host").textContent`)) === "localhost:8765", "and its address");
await t.sleep(300);
await t.shot("little", little);

// Open in Ternt: the page moves to a tab in the active space
await t.eval(`${little}-bar`, `document.querySelector(".open").click()`);
await t.until(async () => (await littles()).length === 0, "Open in Ternt closes the window");
await t.until(() => tab()?.url === `${F}/page2.html`, "and opens the page as a tab");

// The window signs in with the active space's profile
const C = `${F}/cookie.html`;
await t.invoke("navigate", { url: `${C}?set=alpha` });
await t.until(() => tab()?.title === "cookie:alpha", "Home sets a cookie");
await t.invoke("debug_open_external", { url: C });
await t.until(async () => {
  const [w] = await littles();
  return w && (await bar(w, `document.querySelector(".title")?.textContent`)) === "cookie:alpha";
}, "Little Ternt sees Home's cookie");
for (const w of await littles()) await t.eval(`${w}-bar`, `window.close()`).catch(() => {});
const work = await t.invoke("create_space");
await t.until(() => S().activeSpace === work, "in a space with its own profile");
const before = (await littles()).length;
await t.invoke("debug_open_external", { url: C });
await t.until(async () => (await littles()).length > before, "another Little Ternt window");
const fresh = (await littles()).at(-1);
await t.until(async () => (await bar(fresh, `document.querySelector(".title")?.textContent`)) === "cookie:none", "it uses Work's profile (signed out)");

// Only web links
const count = (await littles()).length;
await t.invoke("debug_open_external", { url: "file:///etc/hosts" });
await t.sleep(300);
await t.assert((await littles()).length === count, "file links are ignored");

// Settings → Links: new tab instead
await t.invoke("set_prefs", { prefs: { externalLinks: "tab" } });
await t.assert((await t.invoke("get_prefs")).externalLinks === "tab", "preference saved");
const tabs = S().tabs.length;
await t.invoke("debug_open_external", { url: `${F}/` });
await t.until(() => S().tabs.length === tabs + 1 && tab()?.title === "Fixture Home", "the link opens as a tab");
await t.assert((await littles()).length === count, "no new window");
await t.assert(tab()?.space === work, "in the active space");

await t.quit();
