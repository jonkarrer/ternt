// The icon came back with the session, before the tab loads.
const tab = t.state().tabs.find((x) => x.url.endsWith("/named-icon.html"));
await t.assert(tab && !tab.loaded, "the tab is restored, not loaded");
await t.assert(tab.favicon === `${t.F}/app-icon.svg`, "with its icon");
const img = document.querySelector(`[data-tab="${tab.id}"] .favicon img`);
await t.until(() => img?.src === tab.favicon && img.naturalWidth > 0, "shown in the sidebar");
