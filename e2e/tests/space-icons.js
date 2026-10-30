// Space icons: space menu → Change Icon… opens a picker under the space
// header; the icon shows there and replaces the space's dot in the switcher.
// The header's "…" button opens that same space menu (native menus can't be
// driven from tests; their actions are run through menu_action instead).
const S = () => t.state();
const space = (id) => S().spaces.find((s) => s.id === id);
const picker = () => document.querySelector(".picker");
const home = S().activeSpace;
const work = await t.invoke("create_space");
await t.invoke("rename_space", { id: work, name: "Work" });
await t.until(() => S().activeSpace === work, "in Work");

await t.assert(document.querySelector(".space-header .space-more")?.getAttribute("aria-label") === "Space options", "the header has a … button");

// Change Icon… for another space switches to it and opens the picker
await t.invoke("menu_action", { id: `ctx-space-icon:${home}` });
await t.until(() => S().activeSpace === home && picker(), "Change Icon… switches to Home and opens the picker");
await t.sleep(200);
await t.shot("space-icons-picker");
[...document.querySelectorAll(".picker .emoji")].find((b) => b.textContent === "🏠").click();
await t.until(() => space(home).icon === "🏠" && !picker(), "picking an emoji sets the icon and closes the picker");
await t.until(() => document.querySelector(".space-header .space-icon")?.textContent === "🏠", "the header shows the icon");
await t.until(
  () => document.querySelector(".spaces .space-dot.active .dot-icon")?.textContent === "🏠",
  "the switcher shows the icon instead of the dot",
);

// Any emoji can be typed; only the first one counts
await t.invoke("menu_action", { id: `ctx-space-icon:${work}` });
await t.until(() => S().activeSpace === work && picker(), "picker for Work");
const input = document.querySelector(".picker .custom");
input.value = "🦊 fox";
input.dispatchEvent(new Event("input", { bubbles: true }));
input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
await t.until(() => space(work).icon === "🦊", "a typed emoji becomes the icon");
await t.sleep(200);
await t.shot("space-icons-set");

// Escape closes without changing; Remove clears it
await t.invoke("menu_action", { id: `ctx-space-icon:${work}` });
await t.until(() => picker(), "picker again");
window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
await t.until(() => !picker() && space(work).icon === "🦊", "Escape closes the picker, icon kept");
await t.invoke("menu_action", { id: `ctx-space-icon:${work}` });
await t.until(() => picker(), "picker once more");
document.querySelector(".picker .remove").click();
await t.until(() => space(work).icon === null && !document.querySelector(".space-header .space-icon"), "Remove clears the icon");
await t.assert(!document.querySelector(".spaces .space-dot.active .dot-icon"), "Work is a dot again");

// Icons are one emoji, not text
await t.invoke("space_icon", { id: work, icon: "x".repeat(40) });
await t.sleep(150);
await t.assert(space(work).icon === null, "long text is refused");

await t.quit();
