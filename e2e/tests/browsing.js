const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";

// Navigation state via KVO
await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home" && !tab().loading, "home loaded");
await t.assert(!tab().canGoBack && !tab().canGoForward, "fresh tab has no history");
await t.invoke("navigate", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2" && !tab().loading, "page2 loaded");
await t.assert(tab().canGoBack && !tab().canGoForward, "canGoBack after navigating");
await t.invoke("go_back");
await t.until(() => tab()?.title === "Fixture Home", "native goBack");
await t.assert(tab().canGoForward, "canGoForward after back");
await t.invoke("go_forward");
await t.until(() => tab()?.title === "Fixture Page 2", "native goForward");

// SPA route change with unchanged title
await t.invoke("new_tab", { url: `${F}/spa/start` });
await t.until(() => tab()?.title === "SPA" && !tab().loading, "spa loaded");
await t.eval(`tab-${S().active}`, `go("/spa/route-2")`);
await t.until(() => tab()?.url.endsWith("/spa/route-2"), "pushState URL tracked");
await t.assert(tab().canGoBack, "pushState adds history entry");

// Tab switching shortcuts
await t.invoke("debug_menu", { id: "next-tab" });
await t.until(() => S().active === S().tabs[0].id, "ctrl+tab wraps to first tab");
await t.invoke("debug_menu", { id: "previous-tab" });
await t.until(() => S().active === S().tabs.at(-1).id, "ctrl+shift+tab wraps to last");
await t.invoke("debug_menu", { id: "select-tab-1" });
await t.until(() => S().active === S().tabs[0].id, "cmd+1");
await t.invoke("debug_menu", { id: "select-tab-9" });
await t.until(() => S().active === S().tabs.at(-1).id, "cmd+9 selects last");

// Close + reopen
const before = S().tabs.length;
await t.invoke("debug_menu", { id: "close-tab" });
await t.until(() => S().tabs.length === before - 1, "cmd+w closes tab");
await t.invoke("debug_menu", { id: "reopen-tab" });
await t.until(() => S().tabs.length === before && tab()?.url.includes("/spa/route-2"), "cmd+shift+t reopens");

// Popups: sized window.open -> real window with working opener; _blank -> tab
await t.invoke("new_tab", { url: `${F}/popup.html` });
await t.until(() => tab()?.title === "Opener" && !tab().loading, "opener loaded");
const opener = `tab-${S().active}`;
await t.invoke("debug_allow_popups", { label: opener });
await t.eval(opener, "openPopup()");
await t.until(() => tab()?.title === "got:hello-from-popup", "popup posted back to window.opener");
const windows = await t.invoke("debug_windows");
await t.assert(windows.some((w) => w.startsWith("popup-")), `popup window exists (${windows})`);
const count = S().tabs.length;
await t.eval(opener, "openTab()");
await t.until(() => S().tabs.length === count + 1, "unsized window.open opens a tab");

// Layout after resize
await t.invoke("debug_menu", { id: "select-tab-1" });
await t.resize(1000, 700);
await t.sleep(400);
await t.shot("browsing-resized");
await t.log(`tabs: ${S().tabs.map((x) => x.title).join(" | ")}`);
await t.quit();
