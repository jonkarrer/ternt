// Tab icons: the one the page names (<link rel="icon">), not only the
// site's /favicon.ico, which many sites don't have; saved with the session
// (favicons-2).
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = t.F;
const row = (id) => document.querySelector(`[data-tab="${id}"]`);
const img = (id) => row(id)?.querySelector(".favicon img");

const id = await t.invoke("new_tab", { url: `${F}/named-icon.html` });
await t.until(() => tab()?.favicon === `${F}/app-icon.svg`, "the tab gets the icon its page names");
await t.until(() => img(id)?.src === `${F}/app-icon.svg` && img(id).complete && img(id).naturalWidth > 0, "and the sidebar shows it");

await t.invoke("navigate", { url: `${F}/links.html` });
await t.until(() => tab()?.title === "Fixture Links" && !tab().loading, "a page that names none");
await t.until(() => tab().favicon === null, "drops it");
await t.until(() => !img(id) && row(id).querySelector(".favicon .letter"), "and with no /favicon.ico either, shows the letter");

await t.invoke("go_back");
await t.until(() => tab()?.favicon === `${F}/app-icon.svg`, "back on the page, its icon returns");
// Another tab on top, so favicons-2 restores this one unloaded
await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2", "another tab on top");
