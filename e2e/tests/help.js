// ⌘H: the shortcuts and how-tos sheet, in the command bar overlay.
const { request, type, key } = t.palette;
const F = "http://localhost:8765";
const view = (label) => t.invoke("debug_view", { label });
const sheet = async () =>
  JSON.parse(
    await t.read(
      "palette",
      `JSON.stringify({
        groups: [...document.querySelectorAll(".sheet .group h3")].map((h) => h.textContent.trim()),
        keys: [...document.querySelectorAll(".sheet kbd")].map((k) => k.textContent.trim()),
        howTos: [...document.querySelectorAll(".sheet .how-to h3")].map((h) => h.textContent.trim()),
        none: document.querySelector(".sheet .none")?.textContent ?? "",
        placeholder: document.querySelector("input")?.placeholder ?? "",
      })`,
    ),
  );

const page = await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => t.tab()?.title === "Fixture Home", "a page");

await t.invoke("menu_action", { id: "help" });
await t.until(async () => (await request())?.mode === "help", "⌘H opens the help sheet");
await t.until(async () => !(await view("palette")).hidden, "over the page");
await t.until(async () => (await sheet()).groups.length > 0, "it renders");
let s = await sheet();
await t.assert(s.placeholder.includes("shortcuts"), "its field searches the sheet");
await t.assert(["Tabs", "Terminals", "Sidebar and spaces"].every((g) => s.groups.includes(g)), `shortcut groups (${s.groups})`);
await t.assert(["⌘T", "⌘D", "⇧⌘D", "⌘H"].every((k) => s.keys.includes(k)), "including the split shortcuts and ⌘H itself");
await t.assert(s.howTos.includes("Rename a tab") && s.howTos.length >= 8, "and how-tos");
await t.sleep(300);
await t.shot("help");

// Typing filters it
await type("split");
await t.until(async () => (await sheet()).keys.includes("⇧⌘D") && !(await sheet()).keys.includes("⌘T"), "typing filters the shortcuts");
s = await sheet();
await t.assert(s.howTos.includes("Split a terminal") && !s.howTos.includes("Rename a tab"), "and the how-tos");
await type("zzzz");
await t.until(async () => (await sheet()).none.includes("zzzz"), "says when nothing matches");

// ⌘H again closes it; so does Escape
await t.invoke("menu_action", { id: "help" });
await t.until(async () => !(await request()) && (await view("palette")).hidden, "⌘H again closes it");
await t.invoke("menu_action", { id: "help" });
await t.until(async () => (await request())?.mode === "help", "reopened");
await t.until(async () => (await sheet()).keys.includes("⌘T"), "with the filter cleared");
await key("Escape");
await t.until(async () => !(await request()), "Escape closes it");

// From the command bar too
await t.palette.open("new-tab");
await type("shortcuts");
await t.until(async () => (await t.palette.rows()).includes("Shortcuts and How-Tos"), "the command bar offers it");
await key("Enter");
await t.until(async () => (await request())?.mode === "help", "and opens it");
await key("Escape");
await t.until(async () => !(await request()), "closed");

// The command bar still works normally after
await t.palette.open("open-location");
await t.assert((await request()).mode === "edit", "⌘L is still the URL editor");
await key("Escape");
await t.quit();
