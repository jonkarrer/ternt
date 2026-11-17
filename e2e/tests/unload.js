// Tabs off screen for a while unload to save memory (e2e/tests/unload.env
// makes "a while" 2 seconds), stay in the sidebar and reload when shown.
// The tabs on screen, and pages playing sound, stay loaded; Settings can
// turn it off.
const S = () => t.state();
const tab = (id) => S().tabs.find((x) => x.id === id);
const F = t.F;

const a = await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => tab(a)?.loaded && !tab(a).loading, "a page");
const b = await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab(b)?.loaded && !tab(b).loading, "then another, on screen");
await t.until(() => !tab(a).loaded, "the one off screen unloads after a while", 10000);
await t.assert(tab(a).title === "Fixture Page 2", "and stays in the sidebar as it was");
await t.sleep(2500);
await t.assert(tab(b).loaded, "the tab on screen stays loaded");

await t.invoke("activate_tab", { id: a });
await t.until(() => tab(a).loaded && !tab(a).loading && tab(a).title === "Fixture Page 2", "showing it loads it again");

// A group on screen: both of its tabs stay
await t.invoke("split_view", { id: b });
await t.until(() => S().screen.length === 2, "a and b side by side");
await t.sleep(3000);
await t.assert(tab(a).loaded && tab(b).loaded, "tabs shown together stay loaded");
await t.invoke("unsplit_active").catch(() => {});

// A page playing sound stays (if WebKit lets the fixture play here)
const audio = await t.invoke("new_tab", { url: `${F}/audio.html` });
await t.until(() => tab(audio)?.title === "Audio idle" && !tab(audio).loading, "a page with a tone");
await t.invoke("debug_click", { label: `tab-${audio}`, x: 50, y: 50 });
await t.until(() => /^Audio (running|suspended)/.test(tab(audio).title), "a click plays it");
const playing = tab(audio).title === "Audio running";
const other = await t.invoke("new_tab", { url: `${F}/page2.html?x` });
await t.until(() => tab(other)?.loaded && !tab(other).loading, "another tab in front");
await t.sleep(3500);
if (playing) await t.assert(tab(audio).loaded, "the page playing sound stays loaded");
else await t.log("warn - the fixture's audio didn't start; skipped the playing-sound check");

// Off in Settings: nothing unloads
await t.invoke("set_prefs", { prefs: { ...(await t.invoke("get_prefs")), unloadTabs: false } });
await t.invoke("activate_tab", { id: a });
await t.until(() => tab(a).loaded && !tab(a).loading, "a again");
await t.invoke("activate_tab", { id: other });
await t.sleep(3500);
await t.assert(tab(a).loaded, "with unloading off, tabs off screen stay loaded");
await t.quit();
