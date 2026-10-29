// Builds a session across two spaces; session-2 relaunches and checks it.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const find = (title) => S().tabs.find((x) => x.title === title);

await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home", "home loaded");
await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2", "page2 loaded");
await t.invoke("new_tab", { url: `${F}/spa/start` });
await t.until(() => tab()?.title === "SPA", "spa loaded");

// Home -> favorite, page2 -> pinned in a folder
const home = find("Fixture Home").id;
await t.invoke("move_tab", { id: home, zone: "favorite", folder: null, index: null });
const folder = await t.invoke("create_folder");
await t.invoke("rename_folder", { id: folder, name: "Docs" });
await t.invoke("move_tab", { id: find("Fixture Page 2").id, zone: "pinned", folder, index: null });
await t.until(() => find("Fixture Page 2")?.folder === folder && S().folders[0]?.name === "Docs", "tab pinned into a folder");

// Closing a favorite unloads it but keeps it
await t.invoke("activate_tab", { id: home });
await t.until(() => S().active === home, "favorite active");
await t.invoke("menu_action", { id: "close-tab" });
await t.until(() => S().active !== home && find("Fixture Home") && !find("Fixture Home").loaded, "cmd+w unloads a favorite");
await t.invoke("activate_tab", { id: home });
await t.until(() => find("Fixture Home")?.loaded && tab()?.title === "Fixture Home", "unloaded favorite reloads on click");

// Sidebar collapse
await t.invoke("menu_action", { id: "toggle-sidebar" });
await t.until(() => S().sidebarCollapsed, "cmd+s collapses sidebar");
await t.sleep(300);
await t.shot("session-collapsed");
await t.invoke("menu_action", { id: "toggle-sidebar" });
await t.until(() => !S().sidebarCollapsed, "cmd+s expands sidebar");
await t.assert((await t.invoke("set_sidebar_width", { width: 300 })) === 300, "sidebar width set");

// A second space with a terminal; end on the first space's spa tab
const work = await t.invoke("create_space");
await t.invoke("rename_space", { id: work, name: "Work" });
// Work has its own profile: sign in (a cookie) and keep the page as a favorite
await t.assert(S().spaces.find((s) => s.id === work)?.profile, "Work has its own profile");
const mail = await t.invoke("new_tab", { url: `${F}/cookie.html?set=work-login` });
await t.until(() => tab()?.id === mail && tab()?.title === "cookie:work-login", "Work signs in");
await t.invoke("navigate", { url: `${F}/cookie.html` }); // saved URL mustn't set the cookie itself
await t.until(() => tab()?.url === `${F}/cookie.html` && tab()?.title === "cookie:work-login", "signed in page");
await t.invoke("move_tab", { id: mail, zone: "favorite", folder: null, index: null });
await t.invoke("new_terminal");
await t.until(() => tab()?.kind === "terminal" && S().activeSpace === work, "terminal in Work");
await t.invoke("activate_tab", { id: find("SPA").id });
await t.until(() => tab()?.title === "SPA" && S().activeSpace !== work, "back in Home on spa");
await t.sleep(300);
await t.shot("session-1");
await t.quit();
