// Search engines in the command bar: `you` ⇥ puts a YouTube pill before the
// query, which then goes to YouTube (src/lib/engines.ts).
const S = () => t.state();
const { type, key, rows, request } = t.palette;
const pill = () => t.read("palette", `document.querySelector(".engine")?.textContent.trim() ?? ""`);

await t.palette.open("new-tab");
await type("you");
await t.until(async () => (await rows()).includes("Search YouTube"), "typing the start of a name offers that engine");
await key("Tab");
await t.until(async () => (await pill()) === "YouTube", "Tab puts its pill in the box");
await t.until(async () => (await t.read("palette", `document.querySelector("input").value`)) === "", "and clears the word");
await type("lofi beats");
await t.until(async () => (await rows())[0] === "Search YouTube for “lofi beats”", "the query goes to it");
await t.until(async () => (await rows()).length === 1, "and nothing else is offered");
await t.shot("engines-pill");

// Backspace in an empty box, or Esc, drops the pill
await type("");
await key("Backspace");
await t.until(async () => (await pill()) === "", "Backspace on an empty query drops the pill");
await type("goo");
await key("Tab");
await t.until(async () => (await pill()) === "Google", "goo ⇥ is Google");
await key("Escape");
await t.until(async () => (await pill()) === "", "Esc drops the pill");
await t.assert(await request(), "without closing the command bar");

// Enter opens the search in a new tab
await type("perp");
await key("Tab");
await t.until(async () => (await pill()) === "Perplexity", "perp ⇥ is Perplexity");
await type("why is the sky blue");
await t.sleep(100);
await key("Enter");
await t.until(async () => !(await request()), "Enter closes the command bar");
await t.until(() => S().tabs.some((x) => x.url.startsWith("https://www.perplexity.ai/search?q=why%20is%20the%20sky%20blue")), "and searches Perplexity in a new tab");

// A word that names no engine: Tab does nothing
await t.palette.open("new-tab");
await type("zzz");
await key("Tab");
await t.sleep(150);
await t.assert((await pill()) === "" && (await request()), "no engine, no pill");
await key("Escape");
