// Favorites' app icons come from the live page when it's loaded: many
// sites add their icon links with scripts, or show a signed-out visitor
// (Ternt fetching on its own) a sign-in page instead.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
// Another origin than other suites' localhost, so nothing is remembered.
const page = await t.invoke("new_tab", { url: "http://127.0.0.1:8765/script-icon.html" });
await t.until(() => tab()?.title === "Fixture Script Icon" && !tab().loading, "a page whose icon link comes from a script");
await t.invoke("move_tab", { id: page, zone: "favorite", folder: null, index: null });
await t.until(
  () => document.querySelector(`.favorites .fav[data-tab="${page}"] img.site-icon`)?.src.startsWith("data:image/svg+xml"),
  "its favorite tile shows that icon",
);
await t.shot("favorite-icons");
await t.quit();
