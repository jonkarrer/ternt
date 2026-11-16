// Only the view on top takes a mouse move: the UI webview spans the window
// under the pages, and a hidden terminal view can keep the keyboard, so two
// webviews used to handle each move over a link and fight over the cursor.
// Checked for both ways moves arrive: the first responder and WebKit's
// tracking areas (`null`: the two disagree, or no tracking areas).
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const takes = (label, x, y) => t.invoke("debug_takes_mouse", { label, x, y });
const term = await t.invoke("new_terminal");
await t.until(() => tab()?.kind === "terminal", "a terminal");
const page = await t.invoke("new_tab", { url: "http://localhost:8765/links.html" });
await t.until(() => tab()?.title === "Fixture Links" && !tab().loading, "then a page over it");
const { x, y, width, height } = S().screen[0];
const [px, py] = [x + width / 2, y + height / 2];
await t.assert((await takes(`tab-${page}`, px, py)) === true, "the page takes moves over itself");
await t.assert((await takes("ui", px, py)) === false, "the sidebar's webview, under it, doesn't");
await t.assert((await takes("term", px, py)) === false, "nor the hidden terminal view");
await t.assert((await takes("ui", 100, py)) === true, "the sidebar's webview takes moves over the sidebar");
await t.assert((await takes(`tab-${page}`, 100, py)) === false, "and the page doesn't");
await t.quit();
