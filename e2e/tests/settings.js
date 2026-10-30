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
await t.until(async () => (await read(`document.querySelectorAll(".page-tab").length`)) === "6", "six pages");
await t.assert(
  (await read(`[...document.querySelectorAll(".page-tab")].map((b) => b.textContent.trim()).join(",")`)) ===
    "Account,General,Profiles,Links,Shortcuts,Advanced",
  "pages in order: Account, General, Profiles, Links, Shortcuts, Advanced",
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
await t.assert((await read(`document.querySelector("main .card .label").textContent`)).includes("Home"), "the account lists Default, used by Home");
await t.shot("settings-account", "settings");
await click(`.page-tab:nth-child(2)`);
await t.until(async () => (await read(`document.querySelector("main").textContent`)).includes("Version"), "General shows the version");

// Opening again reuses the window
await t.invoke("menu_action", { id: "settings" });
await t.sleep(200);
await t.assert((await windows()).filter((w) => w === "settings").length === 1, "only one Settings window");

// Profiles: Default first; a new space adds its own, named after it
await click(`.page-tab:nth-child(3)`);
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
