// alert(), confirm() and prompt() show sheets on the window (wry leaves them
// out, so WebKit answered at once: prompt() gave null without asking).
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const page = await t.invoke("new_tab", { url: `${F}/dialogs.html` });
await t.until(() => tab()?.title === "Fixture Dialogs", "a page that asks things");
const run = (js) => t.invoke("debug_eval", { label: `tab-${page}`, js: `setTimeout(() => { ${js} }, 0)` });
const answer = (ok, text) => t.invoke("debug_answer_dialog", { ok, text: text ?? null });
const dialog = async (ok, text) => {
  let texts = null;
  await t.until(async () => (texts = await answer(ok, text)) !== null, "a dialog shows");
  return texts;
};

await run(`alert("Saved"); show("alert", "closed")`);
await t.sleep(300);
await t.assert(tab().title === "Fixture Dialogs", "alert() waits for an answer");
await t.shot("dialogs-alert");
const texts = await dialog(true);
await t.assert(texts.includes("localhost says") && texts.includes("Saved"), `the dialog names the site and shows the message (${texts})`);
await t.until(() => tab().title === 'alert: "closed"', "OK closes the alert");

await run(`show("confirm", confirm("Delete?"))`);
await dialog(true);
await t.until(() => tab().title === "confirm: true", "confirm() OK gives true");
await run(`show("confirm", confirm("Delete?"))`);
await dialog(false);
await t.until(() => tab().title === "confirm: false", "Cancel gives false");

await run(`show("prompt", prompt("Name?", "Ada"))`);
await dialog(true);
await t.until(() => tab().title === 'prompt: "Ada"', "prompt() offers the default text");
await run(`show("prompt", prompt("Name?"))`);
await dialog(false);
await t.until(() => tab().title === "prompt: null", "Cancel gives null");

// A link that asks for a password first (a real click)
await t.invoke("debug_eval", { label: `tab-${page}`, js: `(() => { const r = document.getElementById("locked").getBoundingClientRect(); document.title = "at " + (r.x + 5) + "," + (r.y + r.height / 2); })()` });
await t.until(() => tab()?.title.startsWith("at "), "find the locked link");
const [x, y] = tab().title.slice(3).split(",").map(Number);
await t.invoke("debug_click", { label: `tab-${page}`, x, y });
await t.shot("dialogs-prompt");
await dialog(true, "hunter2");
await t.until(() => tab()?.url.endsWith("/page2.html?password=hunter2"), "a link asking for a password follows it with the answer");
await t.quit();
