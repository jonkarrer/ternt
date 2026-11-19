// Air Traffic Control: links from other apps that match a rule open in its
// space (Settings → Links); the rest go where Settings says (Little Ternt).
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const windows = () => t.invoke("debug_windows");
const littles = async () => (await windows()).filter((w) => w.startsWith("little-"));
const read = (js) => t.read("settings", js);
const home = S().activeSpace;

const work = await t.invoke("create_space");
await t.invoke("rename_space", { id: work, name: "Work" });
await t.invoke("activate_space", { id: home });
await t.until(() => S().activeSpace === home, "two spaces, in Home");

const prefs = await t.invoke("get_prefs");
await t.invoke("set_prefs", {
  prefs: {
    ...prefs,
    linkRules: [
      { kind: "regex", pattern: String.raw`/page2\.html$`, space: home },
      { kind: "host", pattern: "127.0.0.1", space: work },
      { kind: "host", pattern: "  ", space: work },
    ],
  },
});
await t.assert((await t.invoke("get_prefs")).linkRules.length === 2, "rules are saved, blank ones dropped");

await t.invoke("debug_open_external", { url: "http://127.0.0.1:8765/" });
await t.until(() => S().activeSpace === work && tab()?.url.startsWith("http://127.0.0.1:8765/") && tab().space === work, "a host rule opens the link as a tab in its space");
await t.until(() => tab()?.title === "Fixture Home", "and loads it");

await t.invoke("debug_open_external", { url: "http://127.0.0.1:8765/page2.html" });
await t.until(() => S().activeSpace === home && tab()?.url.endsWith("/page2.html") && tab().space === home, "the first rule that matches wins (a regex, to Home)");

const count = (await littles()).length;
await t.invoke("debug_open_external", { url: `${t.F}/links.html` });
await t.until(async () => (await littles()).length === count + 1, "a link no rule matches goes to Little Ternt, as Settings says");

// A rule for a deleted space is skipped
await t.invoke("set_prefs", { prefs: { ...(await t.invoke("get_prefs")), linkRules: [{ kind: "host", pattern: "localhost", space: 999 }] } });
await t.invoke("debug_open_external", { url: `${t.F}/` });
await t.until(async () => (await littles()).length === count + 2, "a rule for a deleted space is skipped");

// Settings → Links
await t.invoke("set_prefs", {
  prefs: { ...(await t.invoke("get_prefs")), linkRules: [{ kind: "host", pattern: "github.com/acme", space: work }] },
});
await t.invoke("menu_action", { id: "settings" });
await t.until(async () => (await windows()).includes("settings"), "Settings opens");
await t.until(async () => (await read(`document.querySelectorAll(".page-tab").length`)) === "6", "Settings is ready");
await t.eval("settings", `document.querySelector(".page-tab:nth-child(4)").click()`);
await t.until(async () => (await read(`document.querySelectorAll(".rule").length`)) === "1", "Links lists the rule");
const typeIn = (selector, value, event = "change") =>
  t.eval("settings", `(() => { const i = ${selector}; i.value = ${JSON.stringify(value)}; i.dispatchEvent(new Event("input", { bubbles: true })); i.dispatchEvent(new Event(${JSON.stringify(event)}, { bubbles: true })); })()`);
await typeIn(`document.querySelector(".sample")`, "https://github.com/acme/repo", "input");
await t.until(async () => (await read(`document.querySelector(".verdict")?.textContent.trim()`)) === "Work (rule 1)", "the tester says where a link goes");
await typeIn(`document.querySelector(".sample")`, "github.com/other", "input");
await t.until(async () => (await read(`document.querySelector(".verdict")?.textContent.trim()`)) === "No rule: Little Ternt", "and when no rule takes it");

await t.eval("settings", `document.querySelector(".add").click()`);
await t.until(async () => (await read(`document.querySelectorAll(".rule").length`)) === "2", "Add Rule adds a row");
await typeIn(`document.querySelectorAll(".rule .pattern")[1]`, "linear.app");
await t.until(async () => (await t.invoke("get_prefs")).linkRules.map((r) => r.pattern).join() === "github.com/acme,linear.app", "it's saved once it has a pattern");
await t.eval("settings", `(() => { const s = document.querySelectorAll(".rule .kind")[1]; s.value = "regex"; s.dispatchEvent(new Event("change", { bubbles: true })); })()`);
await typeIn(`document.querySelectorAll(".rule .pattern")[1]`, "(unclosed");
await t.until(async () => (await read(`document.querySelector(".problem")?.textContent ?? ""`)).length > 0, "a regex that won't compile says why");
await t.shot("atc-settings", "settings");
await typeIn(`document.querySelectorAll(".rule .pattern")[1]`, String.raw`^https://linear\.app/`);
await t.until(async () => !(await read(`document.querySelector(".problem")?.textContent ?? ""`)), "fixed, the warning goes");
await t.eval("settings", `document.querySelectorAll(".rule button[aria-label='Move rule up']")[1].click()`);
await t.until(async () => (await t.invoke("get_prefs")).linkRules[0].kind === "regex", "a rule moves up");
await t.eval("settings", `document.querySelectorAll(".rule button[aria-label='Remove rule']")[0].click()`);
await t.until(async () => (await t.invoke("get_prefs")).linkRules.map((r) => r.pattern).join() === "github.com/acme", "and is removed");
