// History: visited pages show up in the command bar, ranked, per profile.
// (history-2 checks they survive a restart.)
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const request = () => t.invoke("palette_request");
const P = (js) => t.eval("palette", js);
const type = (text) =>
  P(`(() => { const i = document.querySelector("input"); i.value = ${JSON.stringify(text)}; i.dispatchEvent(new Event("input", { bubbles: true })); })()`);
const key = (k) =>
  P(`document.querySelector("input").dispatchEvent(new KeyboardEvent("keydown", { key: ${JSON.stringify(k)}, bubbles: true }))`);
const rows = async () =>
  JSON.parse(await t.read("palette", `JSON.stringify([...document.querySelectorAll("li")].map((li) => (li.querySelector(".label") ?? li).textContent.trim()))`));
const openPalette = async () => {
  await t.invoke("menu_action", { id: "new-tab" });
  await t.until(request, "palette open");
  await t.sleep(200);
};
const search = (q) => t.invoke("history_search", { query: q, limit: 10 });

// Visit pages, then close them
await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2" && !tab().loading, "page 2 loaded");
await t.invoke("navigate", { url: `${F}/spa/history` });
await t.until(() => tab()?.title === "SPA" && !tab().loading, "spa loaded in the same tab");
await t.invoke("navigate", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2" && !tab().loading, "page 2 again");
await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home" && !tab().loading, "home loaded");
await t.until(async () => (await search("fixture")).length === 2, "visits recorded with titles");
const hits = await search("fixture");
await t.assert(hits[0].title === "Fixture Page 2", `the twice-visited page ranks first (${hits.map((h) => h.title)})`);
await t.assert((await search("about")).length === 0, "non-web pages aren't recorded");
for (const x of S().tabs.filter((x) => x.title !== "Fixture Home")) await t.invoke("close_tab", { id: x.id });

// The bar suggests closed pages; open tabs show as tabs instead
await openPalette();
await type("fixture");
await t.until(async () => (await rows()).includes("History"), "history section shows");
const listed = await rows();
await t.assert(listed.some((r) => r.startsWith("Fixture Page 2")), "closed page offered from history");
await t.assert(listed.filter((r) => r.startsWith("Fixture Home")).length <= 1, "open tab not repeated as history");
await t.shot("history-suggest");

// Typing the start of an address puts that page on top, above search
await type("localh");
await t.until(async () => (await rows())[0]?.startsWith("Fixture Page 2"), "address prefix completes to the page");
await key("Enter");
await t.until(() => tab()?.title === "Fixture Page 2", "enter opens it");

// Another profile has its own (empty) history
const work = await t.invoke("create_space");
await t.until(() => S().activeSpace === work, "in Work");
await t.assert((await search("fixture")).length === 0, "Work's profile has no history");
await t.invoke("new_tab", { url: `${F}/spa/work` });
await t.until(() => tab()?.title === "SPA" && !tab().loading, "Work visits a page");
await t.until(async () => (await search("spa")).length === 1, "Work records its own visit");
await t.invoke("activate_space", { id: S().spaces[0].id });
await t.until(async () => (await search("spa")).map((h) => h.url).join() === `${F}/spa/history`, "Home doesn't see Work's visit");

// Clear History empties the current profile's
await t.invoke("remove_space", { id: work });
await t.invoke("new_tab", { url: `${F}/spa/temp` });
await t.until(async () => (await search("temp")).length === 1, "temp visit recorded");
await openPalette();
await type("clear hist");
await t.until(async () => (await rows())[0]?.startsWith("Clear History"), "Clear History command offered");
await key("Enter");
await t.until(async () => (await search("fixture")).length === 0, "history cleared");

// Leave one visit for history-2 to find after a restart
await t.invoke("navigate", { url: `${F}/page2.html?kept` });
await t.until(async () => (await search("kept")).length === 1, "visit to keep");
await t.quit();
