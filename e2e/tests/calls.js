// Video calls: pages get the camera and microphone (WebKit hid
// navigator.mediaDevices without Info.plist's usage strings), once the user
// allows the site in a sheet; the answer is remembered. WebKit's fake devices
// stand in for real ones.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const page = await t.invoke("new_tab", { url: `${F}/call.html` });
await t.until(() => tab()?.title === "Fixture Call", "a call page");
await t.invoke("debug_mock_media", { label: `tab-${page}` });
await t.invoke("navigate", { url: `${F}/call.html?mock` });
await t.until(() => tab()?.url.endsWith("?mock") && tab()?.title === "Fixture Call" && !tab().loading, "loaded again with fake devices");
const run = (js) => t.invoke("debug_eval", { label: `tab-${page}`, js });
await run(`document.title = "api " + typeof navigator.mediaDevices?.getUserMedia + " " + typeof navigator.mediaDevices?.getDisplayMedia`);
await t.until(() => tab()?.title.startsWith("api "), "read the media API");
await t.assert(tab().title === "api function function", `pages have the camera, microphone and screen APIs (${tab().title})`);

const answer = (ok) => t.invoke("debug_answer_dialog", { ok, text: null });
let texts = null;

await run(`join()`);
await t.until(async () => (texts = await answer(false)) !== null, "joining asks first");
await t.assert(texts.includes("Allow localhost to use your camera and microphone?"), `naming the site and what it wants (${texts})`);
await t.until(() => tab()?.title === "refused: NotAllowedError", `Don't Allow refuses (${tab()?.title})`);
const prefs = () => t.invoke("get_prefs");
await t.assert((await prefs()).mediaSites.length === 0, "and isn't remembered");

await run(`join()`);
await t.until(async () => (texts = await answer(true)) !== null, "asked again");
await t.until(() => tab()?.title === "joined: audio,video", `Allow gives the camera and microphone (${tab()?.title})`);
await t.assert((await prefs()).mediaSites.includes("http://localhost:8765"), "and the site is remembered");
await t.invoke("debug_eval", { label: `tab-${page}`, js: `document.title = "playing " + (document.getElementById("me").videoWidth > 0)` });
await t.until(() => tab()?.title.startsWith("playing "), "check the video");
await t.until(async () => { await run(`document.title = "playing " + (document.getElementById("me").videoWidth > 0)`); return tab()?.title === "playing true"; }, "the camera shows");

await run(`leave()`);
await t.until(() => tab()?.title === "left", "leave the call");
await t.invoke("navigate", { url: `${F}/call.html?again` });
await t.until(() => tab()?.url.endsWith("?again") && tab()?.title === "Fixture Call" && !tab().loading, "come back later");
await run(`join({ audio: true })`);
await t.until(() => tab()?.title === "joined: audio", `a remembered site isn't asked again (${tab()?.title})`);
await t.assert((await answer(true)) === null, "no sheet");

// Settings → General lists it; Remove makes it ask again
const read = (js) => t.read("settings", js);
await t.invoke("menu_action", { id: "settings" });
await t.until(async () => (await t.invoke("debug_windows")).includes("settings"), "Settings opens");
await t.until(async () => (await read(`document.querySelectorAll(".page-tab").length`)) === "6", "Settings is ready");
await t.eval("settings", `document.querySelector(".page-tab:nth-child(2)").click()`);
await t.until(async () => (await read(`document.querySelectorAll(".media .site").length`)) === "1", "General lists the allowed site");
await t.assert((await read(`document.querySelector(".media .origin").textContent`)) === "http://localhost:8765", "by its origin");
await t.shot("calls-settings", "settings");
await t.eval("settings", `document.querySelector(".media .site button").click()`);
await t.until(async () => (await prefs()).mediaSites.length === 0, "Remove forgets it");
await t.until(async () => (await read(`document.querySelector(".media .none")?.textContent`)) === "None yet.", "and the list says so");
// (WebKit keeps a grant for the page it was given to.)
await t.invoke("navigate", { url: `${F}/call.html?later` });
await t.until(() => tab()?.url.endsWith("?later") && tab()?.title === "Fixture Call" && !tab().loading, "the site's next visit");
await run(`join({ audio: true })`);
await t.until(async () => (await answer(false)) !== null, "asks again");
await t.quit();
