// Explore mode (⌘F): dim the page, move a cursor over the tabs with j/k,
// peek with Space, open with Enter, go back with Escape.
const S = () => t.state();
const F = "http://localhost:8765";
const title = (id) => S().tabs.find((x) => x.id === id)?.title;
const exploring = () => !!document.querySelector(".app.exploring");
const cursor = () => document.querySelector(".sidebar .cursor .title")?.textContent;
const key = (k) => window.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true }));
const explore = async () => {
  await t.invoke("menu_action", { id: "explore" });
  await t.until(exploring, "explore mode on");
};

for (const page of ["page2.html", "spa/x", ""]) await t.invoke("new_tab", { url: `${F}/${page}` });
await t.until(() => S().tabs.map((x) => x.title).join() === "Fixture Home,SPA,Fixture Page 2", "three tabs");
const [home, spa, page2] = S().tabs.map((x) => x.id);

// The cursor starts on the open tab; moving it doesn't change the page
await explore();
await t.until(() => cursor() === "Fixture Home", "cursor starts on the active tab");
key("j");
await t.until(() => cursor() === "SPA", "j moves down");
await t.sleep(100);
await t.assert(S().active === home, "moving the cursor doesn't switch pages");
await t.shot("explore-cursor");

// Space peeks without leaving the mode
key(" ");
await t.until(() => S().active === spa, "space shows the tab");
await t.assert(exploring(), "still exploring after a peek");
key("ArrowDown");
key("k");
key("ArrowDown");
await t.until(() => cursor() === "Fixture Page 2", "arrows and k move too");
key("j");
await t.sleep(50);
await t.assert(cursor() === "Fixture Page 2", "cursor stops at the end");
await t.shot("explore-peek");

// Escape returns to where exploring started
key("Escape");
await t.until(() => !exploring() && S().active === home, "escape goes back to the starting tab");

// Enter opens the cursor's tab and leaves the mode
await explore();
key("j");
key("j");
key("Enter");
await t.until(() => !exploring() && S().active === page2, "enter opens the tab");

// ⌘F again leaves the mode on what's showing
await explore();
key("k");
key(" ");
await t.until(() => S().active === spa, "peek SPA");
await t.invoke("menu_action", { id: "explore" });
await t.until(() => !exploring() && S().active === spa, "⌘F again keeps the peeked tab");

// Clicking a tab opens it and leaves the mode
await explore();
const row = [...document.querySelectorAll(".tabs .tab")].find((el) => el.textContent.includes("Fixture Home"));
const y = row.getBoundingClientRect().top + 10;
for (const type of ["pointerdown", "pointerup"]) {
  row.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, pointerId: 1, clientX: 50, clientY: y }));
}
await t.until(() => !exploring() && S().active === home, "click opens and exits");

// h/l switch spaces; escape returns to the starting space
const work = await t.invoke("create_space");
await t.invoke("new_tab", { url: `${F}/page2.html` });
const workTab = S().active;
await explore();
key("h");
await t.until(() => S().activeSpace !== work && cursor() === "Fixture Home", "h goes to the previous space, cursor follows");
key("l");
await t.until(() => S().activeSpace === work && S().active === workTab, "l comes back");
key("h");
await t.until(() => S().activeSpace !== work, "h again");
key("Escape");
await t.until(() => !exploring() && S().activeSpace === work && S().active === workTab, "escape restores space and tab");

// A collapsed sidebar opens for exploring and collapses again after
await t.invoke("menu_action", { id: "toggle-sidebar" });
await t.until(() => S().sidebarCollapsed, "sidebar collapsed");
await t.invoke("menu_action", { id: "explore" });
await t.until(() => exploring() && !S().sidebarCollapsed, "explore opens the sidebar");
key("Escape");
await t.until(() => !exploring() && S().sidebarCollapsed, "and collapses it after");
await t.invoke("menu_action", { id: "toggle-sidebar" });

// The command bar takes over from explore mode
await explore();
await t.invoke("menu_action", { id: "new-tab" });
await t.until(() => !exploring(), "opening the command bar ends explore mode");
await t.invoke("close_palette");
await t.quit();
