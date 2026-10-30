const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const find = (title) => S().tabs.find((x) => x.title === title);
const visible = () => [...document.querySelectorAll(".tabs .tab .title")].map((el) => el.textContent.trim());

const home = S().activeSpace;
await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home", "tab in Home");

// New space starts empty
const work = await t.invoke("create_space");
await t.until(() => S().activeSpace === work && S().active === null, "new space is active and empty");
await t.until(() => visible().length === 0 && document.querySelector(".empty"), "sidebar and content show an empty space");
await t.invoke("rename_space", { id: work, name: "Work" });
await t.invoke("space_color", { id: work, color: "green" });
await t.until(() => S().spaces.find((s) => s.id === work)?.color === "green", "space renamed and recolored");

// Spaces sharing a profile share favorites (new spaces get their own; see profiles.js)
await t.invoke("space_profile", { id: work, profile: "shared" });
await t.until(() => !S().spaces.find((s) => s.id === work)?.profile, "Work on the shared profile");

// Tabs belong to their space; favorites are shared by the profile
await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2" && tab()?.space === work, "tab opens in Work");
await t.invoke("new_tab", { url: `${F}/spa/x` });
await t.until(() => tab()?.title === "SPA", "spa opened");
await t.invoke("move_tab", { id: find("SPA").id, zone: "favorite", folder: null, index: null });
await t.until(() => visible().join() === "Fixture Page 2" && document.querySelectorAll(".fav").length === 1, "Work shows its tab plus the favorite");
await t.sleep(250);
await t.shot("spaces-work");

// ⌃1 back to Home: its last tab comes back, the favorite is still there
await t.invoke("menu_action", { id: "select-space-1" });
await t.until(() => S().activeSpace === home && tab()?.title === "Fixture Home", "ctrl+1 restores Home and its last tab");
await t.until(() => visible().join() === "Fixture Home" && document.querySelectorAll(".fav").length === 1, "Home shows its own tabs and the shared favorite");
await t.sleep(250);
await t.shot("spaces-home");

// Next space returns to Work's last tab (the favorite)
await t.invoke("menu_action", { id: "next-space" });
await t.until(() => S().activeSpace === work && tab()?.title === "SPA", "next space restores Work's last tab");

// Two-finger swipe over the sidebar switches spaces (wraps to Home)
const sidebar = document.querySelector(".sidebar");
for (let i = 0; i < 4; i++) sidebar.dispatchEvent(new WheelEvent("wheel", { deltaX: 45, bubbles: true }));
await t.until(() => S().activeSpace === home, "horizontal swipe switches space");

// A mouse: each sideways wheel tick switches once (quick repeats count once),
// and Shift turns the plain wheel sideways
await t.sleep(300);
const tick = (init) => sidebar.dispatchEvent(new WheelEvent("wheel", { deltaMode: WheelEvent.DOM_DELTA_LINE, bubbles: true, ...init }));
tick({ deltaX: 1 });
tick({ deltaX: 1 });
await t.until(() => S().activeSpace === work, "a mouse tick switches to the next space");
await t.sleep(400);
await t.assert(S().activeSpace === work, "a quick second tick doesn't switch again");
tick({ deltaY: -1, shiftKey: true });
await t.until(() => S().activeSpace === home, "Shift + wheel switches back");

// Opening a tab from another space switches to it
await t.invoke("activate_tab", { id: find("Fixture Page 2").id });
await t.until(() => S().activeSpace === work, "activating a tab follows it to its space");

// Deleting a space removes its tabs but not favorites
await t.invoke("remove_space", { id: work });
await t.until(() => S().spaces.length === 1 && S().activeSpace === home, "space deleted, back in Home");
await t.assert(!find("Fixture Page 2") && find("SPA")?.zone === "favorite", "its tabs are gone, the favorite stays");
await t.quit();
