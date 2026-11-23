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

// And the setup window over it: the agent on or off, and its keys
const setup = (js) => t.read("onboarding", js);
const press = (selector) => t.eval("onboarding", `document.querySelector(${JSON.stringify(selector)}).click()`);
await t.until(async () => (await t.invoke("debug_windows")).includes("onboarding"), "first launch opens the setup window");
await t.until(async () => (await setup(`document.querySelector("h1")?.textContent`)) === "Set up Ternt", "which says what it is");
await t.until(async () => (await setup(`String(document.querySelector("input.agent")?.checked)`)) === "true", "with the AI agent on");
await t.assert(
  (await setup(`[...document.querySelectorAll("input.key")].map((i) => i.dataset.provider).join()`)) === "fireworks,anthropic,typesafe,exa",
  "and a field for each key: the two providers, Jev, Exa",
);
await t.assert((await setup(`document.querySelector(".keys .badge")?.parentElement.textContent`)).includes("Jev"), "Jev is marked recommended");
// The stand-in agent needs no key, so Continue is open; a real one asks for a provider's first.
await t.until(async () => (await setup(`String(document.querySelector(".continue").disabled)`)) === "false", "the stand-in agent needs no key to continue");
await t.sleep(300);
await t.shot("onboarding", "onboarding");
await t.assert(document.querySelector(".titlebar .ask") != null, "the sidebar has its Ask button");
await press("input.agent");
await t.until(async () => (await setup(`document.querySelector(".off")?.textContent`))?.includes("browser and a terminal"), "switched off, it says what Ternt is without the agent");
await press(".continue");
await t.until(async () => !(await t.invoke("debug_windows")).includes("onboarding"), "Continue closes the window");
await t.until(async () => (await t.invoke("get_prefs")).agentEnabled === false, "and the choice is saved");
await t.until(() => document.querySelector(".titlebar .ask") == null, "the Ask button is gone");
await t.until(async () => (await t.invoke("agent_status")).state === "down", "the agent process is stopped");
await t.invoke("menu_action", { id: "agent" });
await t.sleep(300);
await t.assert((await t.invoke("agent_request")) == null, "⌘J does nothing while it is off");

// Help → Set Up Ternt… brings it back; on again starts the agent
await t.invoke("menu_action", { id: "onboarding" });
await t.until(async () => (await setup(`String(document.querySelector("input.agent")?.checked)`)) === "false", "Help → Set Up Ternt… opens it again, as it was left");
await press("input.agent");
await t.until(async () => (await setup(`String(document.querySelector(".continue").disabled)`)) === "true", "with no agent running to say it needs no key, a provider's key is asked for");
await t.invoke("set_prefs", { prefs: { ...(await t.invoke("get_prefs")), agentEnabled: true } });
await t.until(async () => (await t.invoke("agent_status")).state === "running", "switched on (as Settings does it), the agent starts", 10000);
await t.until(() => document.querySelector(".titlebar .ask") != null, "and the Ask button is back");
await t.until(async () => (await setup(`String(document.querySelector(".continue").disabled)`)) === "false", "Continue opens once the stand-in agent is up");
await press(".continue");
await t.until(async () => !(await t.invoke("debug_windows")).includes("onboarding"), "and closes the window");

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
