// Settings (Ternt → Settings…, ⌘,): its own window, one at a time. The
// first page is the main "Ternt" profile; Profiles lists one per space with
// its own, named after the space until renamed.
const S = () => t.state();
const windows = () => t.invoke("debug_windows");
const read = (js) => t.read("settings", js);
const click = (selector) => t.eval("settings", `document.querySelector(${JSON.stringify(selector)}).click()`);
const profileNames = () => S().profiles.map((p) => p.name).join(",");

await t.assert(profileNames() === "Ternt", "a fresh session has only the main profile");

await t.invoke("menu_action", { id: "settings" });
await t.until(async () => (await windows()).includes("settings"), "Settings opens a window");
await t.until(async () => (await read(`document.querySelectorAll(".page-tab").length`)) === "6", "six pages");
await t.assert(
  (await read(`[...document.querySelectorAll(".page-tab")].map((b) => b.textContent.trim()).join(",")`)) ===
    "Ternt,General,Profiles,Links,Shortcuts,Advanced",
  "pages in order: Ternt, General, Profiles, Links, Shortcuts, Advanced",
);
const title = () => read(`document.querySelector("main .title")?.textContent.trim()`);
await t.until(async () => (await title()) === "Ternt", "Settings opens on the main Ternt profile");
await t.assert((await read(`document.querySelector("main .row .label").textContent.trim()`)) === "Home", "Home signs in with Ternt");
await t.shot("settings-ternt", "settings");
await click(`.page-tab:nth-child(2)`);
await t.until(async () => (await read(`document.querySelector("main").textContent`)).includes("Version"), "General shows the version");

// Opening again reuses the window
await t.invoke("menu_action", { id: "settings" });
await t.sleep(200);
await t.assert((await windows()).filter((w) => w === "settings").length === 1, "only one Settings window");

// Profiles: only space profiles; a new space adds its own, named after it
await click(`.page-tab:nth-child(3)`);
await t.until(async () => (await read(`document.querySelector(".empty")?.textContent`))?.includes("No space profiles"), "no space profiles yet");
const work = await t.invoke("create_space");
await t.invoke("rename_space", { id: work, name: "Work" });
await t.until(() => profileNames() === "Ternt,Work", "the new space's profile is named after it");
const listed = () => read(`[...document.querySelectorAll(".profile .name")].map((n) => n.textContent.trim()).join(",")`);
await t.until(async () => (await listed()) === "Work", "Profiles lists Work (Ternt has its own page)");

// Rename the Work profile from its page
await t.until(async () => (await read(`document.querySelector(".title-field")?.value`)) === "Work", "Work's page opens");
await t.eval(
  "settings",
  `(() => { const i = document.querySelector(".title-field"); i.value = "Job"; i.dispatchEvent(new Event("blur")); })()`,
);
await t.until(() => profileNames() === "Ternt,Job", "renaming a profile reaches the model");
await t.until(() => document.querySelector(".space-header .profile")?.textContent.trim() === "Job", "the sidebar badge shows the profile name");
await t.shot("settings-profiles", "settings");

// Move Work onto the main profile from Settings: its own profile goes away
await t.eval(
  "settings",
  `(() => { const s = document.querySelector(".detail select"); s.value = "shared"; s.dispatchEvent(new Event("change", { bubbles: true })); })()`,
);
await t.until(() => profileNames() === "Ternt" && S().spaces.find((s) => s.id === work)?.profile === null, "Work moved to Ternt");
await t.until(async () => (await read(`document.querySelector(".empty")?.textContent`))?.includes("No space profiles"), "Profiles is empty again");
await click(`.page-tab:nth-child(1)`);
await t.until(async () => (await read(`document.querySelectorAll("main .dot").length`)) === "2", "Ternt now lists Home and Work");

// Links: where links from other apps go, and the default browser
await click(`.page-tab:nth-child(4)`);
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
