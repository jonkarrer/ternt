// Separate profiles: a space with its own cookie jar.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const C = "http://localhost:8765/cookie.html";
const cookie = () => tab()?.title?.replace("cookie:", "");

// Home (shared store) sets a cookie
const home = S().activeSpace;
await t.invoke("new_tab", { url: `${C}?set=alpha` });
await t.until(() => cookie() === "alpha", "Home sets a cookie");
const homeTab = S().active;

// A new space shares it by default
const work = await t.invoke("create_space");
await t.invoke("new_tab", { url: C });
await t.until(() => cookie() === "alpha", "new spaces share cookies by default");
const workTab = S().active;

// Isolating Work reloads its tabs in a fresh, empty jar
await t.invoke("space_profile", { id: work, isolated: true });
await t.until(() => S().spaces.find((s) => s.id === work)?.profile, "Work has its own profile");
await t.until(() => S().active === workTab && cookie() === "none", "isolated space starts with no cookies");
await t.invoke("navigate", { url: `${C}?set=beta` });
await t.until(() => cookie() === "beta", "Work sets its own cookie");
await t.sleep(250);
await t.shot("profiles-work");

// Home is unaffected
await t.invoke("activate_tab", { id: homeTab });
await t.invoke("navigate", { url: C });
await t.until(() => S().activeSpace === home && cookie() === "alpha", "Home still sees only its cookie");

// Favorites always use the shared jar, even when opened from Work
await t.invoke("activate_space", { id: work });
await t.invoke("move_tab", { id: workTab, zone: "favorite", folder: null, index: null });
await t.invoke("close_tab", { id: workTab }); // unload, reload under the shared jar
await t.invoke("activate_tab", { id: workTab });
await t.invoke("navigate", { url: C });
await t.until(() => S().active === workTab && cookie() === "alpha", "favorites use the shared jar");
await t.invoke("move_tab", { id: workTab, zone: "today", folder: null, index: null });

// Turning isolation off returns Work to the shared jar
await t.invoke("space_profile", { id: work, isolated: false });
await t.until(() => !S().spaces.find((s) => s.id === work)?.profile, "Work back on the shared profile");
await t.invoke("activate_tab", { id: workTab });
await t.invoke("navigate", { url: C });
await t.until(() => cookie() === "alpha", "shared cookies visible again");

await t.invoke("navigate", { url: `${C}?clear` });
await t.until(() => cookie() === "none", "test cookie cleared");
await t.quit();
