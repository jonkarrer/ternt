// Terminals sit at the bottom of the sidebar, below Today's pages; a long
// Today list pushes them down and they scroll with it.
const S = () => t.state();
const F = "http://localhost:8765";
const rect = (sel) => document.querySelector(sel).getBoundingClientRect();
const rows = (sel) => [...document.querySelectorAll(`${sel} [data-tab]`)].map((el) => Number(el.dataset.tab));
const scroll = () => document.querySelector(".space-scroll");

await t.invoke("new_tab", { url: `${F}/` });
const term = await t.invoke("new_terminal");
await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => rows(".terminals").includes(term), "the terminal shows in the bottom section");
await t.assert(!rows(".space-scroll > .tabs").includes(term) && rows(".space-scroll > .tabs").length === 2, "not among Today's pages");
const bottom = rect(".space-scroll").bottom - rect(".terminals").bottom;
await t.assert(bottom >= 0 && bottom < 16, `pinned to the bottom (${bottom}px above it)`);
await t.assert(rect(".terminals").top - rect(".space-scroll > .tabs [data-tab]:last-child").bottom > 100, "with the room above it");
await t.sleep(250);
await t.shot("sidebar-terminals");

// Next Tab walks the sidebar as shown: pages, then terminals
await t.invoke("activate_tab", { id: rows(".space-scroll > .tabs")[1] });
await t.invoke("menu_action", { id: "next-tab" });
await t.until(() => S().active === term, "Next Tab goes from the last page to the terminal");

// A long Today list pushes the terminals down; it all scrolls
for (let i = 0; i < 30; i++) await t.invoke("new_tab", { url: `${F}/?n=${i}` });
await t.until(() => rows(".space-scroll > .tabs").length === 32, "lots of pages");
await t.assert(scroll().scrollHeight > scroll().clientHeight, "the sidebar scrolls");
await t.assert(rect(".terminals").top >= rect(".space-scroll > .tabs [data-tab]:last-child").bottom, "terminals stay below the pages");
await t.assert(rect(".terminals").top > rect(".space-scroll").bottom, "pushed out of view");
scroll().scrollTop = scroll().scrollHeight;
await t.sleep(100);
await t.assert(rect(".terminals").bottom <= rect(".space-scroll").bottom + 1, "and scroll into view at the end");
await t.shot("sidebar-terminals-scrolled");
await t.quit();
