// A real mouse drag (NSEvents) from a sidebar row onto the page area: the
// sidebar keeps getting the drag over the page's native view, and the drop
// makes a group.
const S = () => t.state();
const F = "http://localhost:8765";
const a = await t.invoke("new_tab", { url: `${F}/` });
const b = await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => S().tabs.every((x) => x.title), "two tabs");
await t.sleep(500);
const row = document.querySelector(`.tab[data-tab="${a}"]`).getBoundingClientRect();
await t.invoke("debug_click", { label: "ui", x: row.x + 30, y: row.y + row.height / 2, to: [1100, 400] });
await t.until(() => JSON.stringify(S().groups[0]?.columns) === JSON.stringify([[b], [a]]), "a real mouse drag onto the right half groups them");
await t.quit();
