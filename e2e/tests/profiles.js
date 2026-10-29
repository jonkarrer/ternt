// Profiles: each new space gets its own cookie jar (Google logins etc.),
// favorites and sign-in popups follow it, and spaces can share one.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const C = "http://localhost:8765/cookie.html";
const cookie = () => tab()?.title?.replace("cookie:", "");
const profile = (id) => S().spaces.find((s) => s.id === id)?.profile;
const favs = () => document.querySelectorAll(".favorites .fav").length;

// Home (shared store) sets a cookie
const home = S().activeSpace;
await t.invoke("new_tab", { url: `${C}?set=alpha` });
await t.until(() => cookie() === "alpha", "Home sets a cookie");
const homeTab = S().active;

// A new space starts signed out, in its own profile
const work = await t.invoke("create_space");
await t.assert(profile(work), "new spaces get their own profile");
await t.invoke("new_tab", { url: C });
await t.until(() => cookie() === "none", "new space can't see Home's cookie");
await t.invoke("navigate", { url: `${C}?set=beta` });
await t.until(() => cookie() === "beta", "Work sets its own cookie");
const workTab = S().active;
await t.sleep(250);
await t.shot("profiles-work");

// Sign-in popups opened from Work use Work's jar
await t.invoke("debug_allow_popups", { label: `tab-${workTab}` });
await t.eval(`tab-${workTab}`, "openPopup()");
await t.until(() => tab()?.title === "popup:beta", "popup sees the opener's profile");

// Home is unaffected
await t.invoke("activate_tab", { id: homeTab });
await t.invoke("navigate", { url: C });
await t.until(() => S().activeSpace === home && cookie() === "alpha", "Home still sees only its cookie");

// Favorites belong to the profile they were added in
await t.invoke("activate_tab", { id: workTab });
await t.invoke("move_tab", { id: workTab, zone: "favorite", folder: null, index: null });
await t.until(() => favs() === 1, "Work shows its favorite");
await t.invoke("close_tab", { id: workTab }); // unload; reloads under Work's jar
await t.invoke("activate_tab", { id: workTab });
await t.invoke("navigate", { url: C });
await t.until(() => S().active === workTab && cookie() === "beta", "favorite uses its profile's jar");
await t.invoke("activate_space", { id: home });
await t.until(() => S().activeSpace === home && favs() === 0, "Home doesn't show Work's favorites");

// A space can share another's profile
const work2 = await t.invoke("create_space");
await t.invoke("space_profile", { id: work2, profile: `space-${work}` });
await t.until(() => profile(work2) && profile(work2) === profile(work), "Work 2 shares Work's profile");
await t.until(() => favs() === 1, "Work 2 shows Work's favorites");
await t.invoke("new_tab", { url: C });
await t.until(() => cookie() === "beta", "Work 2 sees Work's cookie");
const work2Tab = S().active;

// Moving Work to the shared profile takes its favorite along; Work 2 keeps the old one
await t.invoke("space_profile", { id: work, profile: "shared" });
await t.until(() => !profile(work), "Work back on the shared profile");
await t.invoke("activate_tab", { id: workTab });
await t.invoke("navigate", { url: C });
await t.until(() => cookie() === "alpha", "Work's favorite now sees shared cookies");
await t.invoke("activate_tab", { id: work2Tab });
await t.invoke("navigate", { url: C });
await t.until(() => S().active === work2Tab && cookie() === "beta", "Work 2's profile survives");

await t.invoke("remove_space", { id: work2 });
await t.invoke("activate_tab", { id: homeTab });
await t.invoke("navigate", { url: `${C}?clear` });
await t.until(() => cookie() === "none", "test cookie cleared");
await t.quit();
