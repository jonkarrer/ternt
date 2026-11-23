// Settings (Ternt → Settings…, ⌘,): its own window, one at a time. The
// first page is your account (name, email); Profiles lists Default and one
// per space with its own, named after the space until renamed.
const S = () => t.state();
const windows = () => t.invoke("debug_windows");
const read = (js) => t.read("settings", js);
const click = (selector) => t.eval("settings", `document.querySelector(${JSON.stringify(selector)}).click()`);
const profileNames = () => S().profiles.map((p) => p.name).join(",");
const setField = (selector, value) =>
  t.eval(
    "settings",
    `(() => { const i = document.querySelector(${JSON.stringify(selector)}); i.value = ${JSON.stringify(value)}; i.dispatchEvent(new Event("blur")); })()`,
  );

await t.assert(profileNames() === "Default", "a fresh session has only the Default profile");

await t.invoke("menu_action", { id: "settings" });
await t.until(async () => (await windows()).includes("settings"), "Settings opens a window");
await t.until(async () => (await read(`document.querySelectorAll(".page-tab").length`)) === "7", "seven pages");
await t.assert(
  (await read(`[...document.querySelectorAll(".page-tab")].map((b) => b.textContent.trim()).join(",")`)) ===
    "Account,Agent,General,Profiles,Links,Shortcuts,Advanced",
  "pages in order: Account, Agent, General, Profiles, Links, Shortcuts, Advanced",
);

// Account: name and email, saved in prefs
await t.until(async () => (await read(`document.querySelector("main .name")?.placeholder`)) === "Your name", "Settings opens on the account");
await setField("main .name", "  Ada Lovelace ");
await setField("main .email", "ada@example.com");
await t.until(async () => {
  const { account } = await t.invoke("get_prefs");
  return account.name === "Ada Lovelace" && account.email === "ada@example.com";
}, "name and email are saved (trimmed)");
await t.until(async () => (await read(`document.querySelector("main .avatar").textContent.trim()`)) === "A", "the avatar shows the initial");
await t.assert((await read(`[...document.querySelectorAll("main .card .label")].map((el) => el.textContent).join()`)).includes("Home"), "the account lists Default, used by Home");
await t.assert(
  /Ternt (is|isn't) your default browser/.test(await read(`document.querySelector("main .card .label").textContent`)),
  "the account shows the default browser card first",
);
await t.shot("settings-account", "settings");
await click(`.page-tab:nth-child(2)`);
const space = S().activeSpace;
await t.until(async () => (await read(`document.querySelector("select.space")?.value`)) === String(space), "Agent shows the active space");
await t.until(async () => (await read(`document.querySelector("select.provider")?.value`)) === "fireworks", "on Fireworks AI by default");
await t.until(async () => (await read(`document.querySelector("main").textContent`)).includes("stand-in agent"), "and that the stub is running");
await t.assert((await read(`document.querySelector('input.key[data-provider="fireworks"]')?.placeholder`)) === "fw_…", "with a field for the Fireworks key");
await t.assert((await read(`document.querySelector('input.key[data-provider="anthropic"]')?.placeholder`)) === "sk-ant-…", "and one for the Anthropic key");
const generation = (await t.invoke("agent_status")).generation;
await t.eval("settings", `const s = document.querySelector("select.provider"); s.value = "anthropic"; s.dispatchEvent(new Event("change", { bubbles: true }))`);
await t.until(() => S().spaces.find((s) => s.id === space)?.agent.provider === "anthropic", "choosing Anthropic is saved with the space");
await t.until(async () => (await t.invoke("agent_space_status", { space })).provider === "anthropic", "and the space's agent runs on it");
await t.assert((await t.invoke("agent_status")).generation === generation, "without restarting the process");
await t.assert(S().spaces.every((s) => s.id === space || s.agent.provider === "fireworks"), "other spaces keep their own provider");

// Tools: switched on per space, with a key of their own under Keys
const search = `document.querySelector('.tool[data-tool="web_search"] input')`;
await t.assert((await read(`String(${search}?.checked)`)) === "false", "web search is off until switched on");
await t.assert((await read(`document.querySelector('input.key[data-provider="exa"]')?.placeholder`)) === "Web search…", "and Keys has a field for the Exa key");
await t.assert((await read(`document.querySelector('input.key[data-provider="typesafe"]')?.placeholder`)) === "Recommended…", "and one for the Jev key, marked recommended");
await t.assert((await read(`document.querySelector(".badge")?.parentElement.textContent`)).includes("Jev"), "by a badge too");
await t.assert((await read(`String(document.querySelector("input.enabled")?.checked)`)) === "true", "the agent is on, with a switch to turn it off");
await click(`.tool[data-tool="web_search"] input`);
await t.until(() => S().spaces.find((s) => s.id === space)?.agent.tools.includes("web_search"), "switching it on is saved with the space");
await t.until(async () => (await t.invoke("agent_space_status", { space })).tools.includes("web_search"), "and the space's agent has the tool");
await t.assert((await t.invoke("agent_space_status", { space })).provider === "anthropic", "on the provider it was on");
await t.assert((await t.invoke("agent_status")).generation === generation, "without restarting the process");
await t.assert(S().spaces.every((s) => s.id === space || s.agent.tools.length === 0), "other spaces keep their own tools");
await t.until(async () => (await read(`String(${search}?.checked)`)) === "true", "the switch shows it on");
await click(`.tool[data-tool="web_search"] input`);
await t.until(async () => (await t.invoke("agent_space_status", { space })).tools.length === 0, "and off again takes it away");

// Memory: what the space's agent learned; edit, forget, wipe
const memoryTexts = () => read(`[...document.querySelectorAll(".memory .text")].map((el) => el.textContent).join("|")`);
const remembered = () => t.invoke("agent_memories", { space });
await t.until(async () => (await read(`document.querySelector("main .empty")?.textContent`)) === "Nothing remembered yet.", "Memory has nothing remembered yet");
await t.assert((await read(`String(document.querySelector("button.wipe").disabled)`)) === "true", "and nothing to wipe");
await t.invoke("agent_send", { space, text: "remember the stub", thread: null });
await t.until(async () => (await remembered()).length === 1, "the stub learns one fact per answered question", 10000);
await t.eval("settings", `window.dispatchEvent(new Event("focus"))`);
await t.until(async () => (await memoryTexts()).includes("remember the stub"), "coming back to Settings lists it");
await click(".memory button.edit");
await t.until(async () => ((await read(`document.querySelector("input.memory-text")?.value`)) ?? "").includes("remember the stub"), "Edit opens the fact's text");
await t.eval("settings", `(() => { const i = document.querySelector("input.memory-text"); i.value = "Likes tea"; i.dispatchEvent(new Event("input", { bubbles: true })); i.form.requestSubmit(); })()`);
await t.until(async () => (await memoryTexts()) === "Likes tea", "saving replaces the fact");
await t.until(async () => { const m = await remembered(); return m.length === 1 && m[0].text === "Likes tea" && m[0].version === 2; }, "as a new version at the agent");
await t.invoke("agent_send", { space, text: "and another", thread: null });
await t.until(async () => (await remembered()).length === 2, "a second fact", 10000);
await t.eval("settings", `window.dispatchEvent(new Event("focus"))`);
await t.until(async () => (await read(`document.querySelectorAll(".memory").length`)) === "2", "both listed, newest first");
await t.assert((await memoryTexts()).startsWith("In the ") && (await memoryTexts()).endsWith("|Likes tea"), `newest first: ${await memoryTexts()}`);
await t.shot("settings-agent", "settings");
await click(".memory button.forget");
await t.until(async () => (await memoryTexts()) === "Likes tea", "Forget removes one");
await t.until(async () => (await remembered()).length === 1, "at the agent too");
await click("button.wipe");
await t.until(async () => (await read(`document.querySelector(".count").textContent`)).includes("no undo"), "Wipe asks first");
await click("button.wipe-confirm");
await t.until(async () => (await read(`document.querySelector("main .empty")?.textContent`)) === "Nothing remembered yet.", "and then forgets everything");
await t.until(async () => (await remembered()).length === 0, "the agent has nothing left");
await t.until(async () => (await t.invoke("agent_threads", { space })).length === 2, "the conversations stay");
await click(`.page-tab:nth-child(3)`);
await t.until(async () => (await read(`document.querySelector("main").textContent`)).includes("Version"), "General shows the version");
await t.until(async () => (await read(`String(document.querySelector("input.unload")?.checked)`)) === "true", "unloading idle tabs is on by default");
await t.sleep(200);
await t.shot("settings-general", "settings");
await click(`input.unload`);
await t.until(async () => (await t.invoke("get_prefs")).unloadTabs === false, "its switch turns it off");
await click(`input.unload`);
await t.until(async () => (await t.invoke("get_prefs")).unloadTabs === true, "and on");

// Opening again reuses the window
await t.invoke("menu_action", { id: "settings" });
await t.sleep(200);
await t.assert((await windows()).filter((w) => w === "settings").length === 1, "only one Settings window");

// Profiles: Default first; a new space adds its own, named after it
await click(`.page-tab:nth-child(4)`);
const listed = () => read(`[...document.querySelectorAll(".profile .name")].map((n) => n.textContent.trim()).join(",")`);
await t.until(async () => (await listed()) === "Default", "Profiles lists Default");
await t.until(async () => (await read(`document.querySelector(".detail .title")?.textContent.trim()`)) === "Default", "Default's page is shown");
const work = await t.invoke("create_space");
await t.invoke("rename_space", { id: work, name: "Work" });
await t.until(() => profileNames() === "Default,Work", "the new space's profile is named after it");
await t.until(async () => (await listed()) === "Default,Work", "Profiles lists Default, then Work");

// Rename the Work profile from its page
await t.eval("settings", `document.querySelectorAll(".profile")[1].click()`);
await t.until(async () => (await read(`document.querySelector(".title-field")?.value`)) === "Work", "Work's page opens");
await setField(".title-field", "Job");
await t.until(() => profileNames() === "Default,Job", "renaming a profile reaches the model");
await t.assert(!document.querySelector(".space-header .profile"), "the sidebar shows no profile badge: it's just a space");

// Each space's color can be changed here
await t.eval("settings", `document.querySelector('.detail .swatch[aria-label="green"]').click()`);
await t.until(() => S().spaces.find((s) => s.id === work)?.color === "green", "a swatch sets the space's color");
await t.until(async () => (await read(`document.querySelector(".detail .swatch.selected")?.getAttribute("aria-label")`)) === "green", "and shows as selected");
await t.shot("settings-profiles", "settings");

// Move Work onto Default from Settings: its own profile goes away
await t.eval(
  "settings",
  `(() => { const s = document.querySelector(".detail select"); s.value = "shared"; s.dispatchEvent(new Event("change", { bubbles: true })); })()`,
);
await t.until(() => profileNames() === "Default" && S().spaces.find((s) => s.id === work)?.profile === null, "Work moved to Default");
await t.until(async () => (await listed()) === "Default", "only Default is left");
await t.until(async () => (await read(`document.querySelectorAll(".detail .space-glyph").length`)) === "2", "Default now lists Home and Work");

// Links: where links from other apps go, and the default browser
await click(`.page-tab:nth-child(5)`);
await t.until(async () => (await read(`document.querySelector("main").textContent`)).includes("default browser"), "Links shows the default browser");
await t.until(async () => (await read(`document.querySelector("input[type=radio]:checked")?.parentElement.textContent`)).includes("Little Ternt"), "Little Ternt is the default choice");
await t.eval("settings", `document.querySelectorAll("input[type=radio]")[1].click()`);
await t.until(async () => (await t.invoke("get_prefs")).externalLinks === "tab", "choosing New tab saves the preference");
await t.shot("settings-links", "settings");
await t.eval("settings", `document.querySelectorAll("input[type=radio]")[0].click()`);
await t.until(async () => (await t.invoke("get_prefs")).externalLinks === "little", "and back");

// ⌘W closes Settings while it's in front, not a tab
await t.invoke("new_tab", { url: `${t.F}/a.html` });
const tabs = S().tabs.length;
await t.eval("settings", `window.focus()`);
await t.invoke("menu_action", { id: "settings" });
await t.sleep(300);
// The test app can't always make itself frontmost, and without a key window
// ⌘W rightly goes to the browser: only check when Settings is in front.
if ((await read("document.hasFocus()")) === "true") {
  await t.invoke("menu_action", { id: "close-tab" });
  await t.until(async () => !(await windows()).includes("settings"), "⌘W closes the Settings window");
  await t.assert(S().tabs.length === tabs, "no tab closed");
} else {
  await t.log("warn - ⌘W check skipped: Settings isn't the key window (app in the background)");
}

await t.quit();
