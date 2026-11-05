// Tab groups: up to four tabs shown together (pages and terminals), made by
// "Open in Split View" or by dragging a tab onto the content area. Rust
// places the views; the sidebar shows each group as a row over its tabs.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const view = (label) => t.invoke("debug_view", { label });
const near = (a, b) => Math.abs(a - b) <= 1;
const INSET = 8;
const group = () => S().groups[0];
const layout = () => JSON.stringify(group()?.columns);
const frame = (id) => S().screen.find((f) => f.tab === id);
const sameAs = async (label, f) => {
  const v = await view(label);
  return !v.hidden && near(v.x, f.x) && near(v.width, f.width) && near(v.height, f.height);
};
const rowOf = (id) => document.querySelector(`.tab[data-tab="${id}"]`);
const groupRow = () => document.querySelector(".row.group");

const a = await t.invoke("new_tab", { url: `${F}/` });
await t.until(() => tab()?.title === "Fixture Home", "page A");
const b = await t.invoke("new_tab", { url: `${F}/page2.html` });
await t.until(() => tab()?.title === "Fixture Page 2", "page B");
const full = await view(`tab-${b}`);
const room = full.width - INSET;

// Open A beside B: a group of two, side by side
await t.invoke("split_view", { id: a });
await t.until(() => layout() === JSON.stringify([[b], [a]]), "a group: the active tab left, the other right");
await t.assert(S().active === b && S().shownGroup === group().id, "the active tab stays active, the group on screen");
await t.assert(group().name === "Group 1", "named Group 1");
await t.until(async () => (await sameAs(`tab-${b}`, frame(b))) && (await sameAs(`tab-${a}`, frame(a))), "both views in their frames");
await t.assert(near(frame(b).width, Math.round(room / 2)) && near(frame(a).x + frame(a).width, full.x + full.width), `half and half (${frame(b).width})`);
await t.until(() => groupRow()?.textContent.includes("Group 1"), "the sidebar shows the group's row");
const color = (id) => rowOf(id)?.style.getPropertyValue("--split");
await t.assert(color(a) && color(a) === color(b) && groupRow().style.getPropertyValue("--split") === color(a), `its tabs share its color (${color(a)})`);
await t.assert(groupRow().nextElementSibling === rowOf(b) && rowOf(b).nextElementSibling === rowOf(a), "with its tabs right under it");
await t.until(() => document.querySelector(".split-handle"), "a divider between the panes");
await t.sleep(300);
await t.shot("group-pages");

// Resize: clamped, both follow
await t.invoke("set_group_ratio", { column: null, ratio: 0.3 });
await t.until(async () => near((await view(`tab-${b}`)).width, Math.round(room * 0.3)), "left pane at 30%");
await t.assert((await t.invoke("set_group_ratio", { column: null, ratio: 0.01 })) === 0.2, "ratio clamped to 20%");
await t.invoke("set_group_ratio", { column: null, ratio: 0.5 });

// Clicking into the other pane makes it active; the group stays up
await t.invoke("debug_focus", { label: `tab-${a}` });
await t.until(() => S().active === a, "focusing the right pane activates its tab");
await t.assert(S().shownGroup === group().id, "the group stays on screen");

// Another tab shows alone; the group's row brings the group back
const c = await t.invoke("new_tab", { url: `${F}/spa/x` });
await t.until(() => tab()?.title === "SPA" && S().shownGroup === null, "a third tab, alone");
await t.until(async () => (await view(`tab-${a}`)).hidden && (await view(`tab-${b}`)).hidden, "the group is hidden");
groupRow().click();
await t.until(() => S().shownGroup === group().id && S().active === a, "clicking the group's row shows it, its last active tab focused");
await t.until(async () => !(await view(`tab-${a}`)).hidden && !(await view(`tab-${b}`)).hidden && (await view(`tab-${c}`)).hidden, "both panes back");
// A tab of the group, clicked in the sidebar, shows alone
const pointer = (el, type, x, y) => el.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y }));
const r = rowOf(b).getBoundingClientRect();
pointer(rowOf(b), "pointerdown", r.x + 20, r.y + 10);
pointer(rowOf(b), "pointerup", r.x + 20, r.y + 10);
await t.until(() => S().active === b && S().shownGroup === null, "clicking a grouped tab shows it alone");
await t.until(async () => near((await view(`tab-${b}`)).width, full.width) && (await view(`tab-${a}`)).hidden, "full screen");
groupRow().click();
await t.until(() => S().shownGroup === group().id, "the group again");

// Drag C from the sidebar onto the bottom of A's pane: 50/25/25
const fa = frame(a);
const rc = rowOf(c).getBoundingClientRect();
pointer(rowOf(c), "pointerdown", rc.x + 20, rc.y + 10);
pointer(rowOf(c), "pointermove", rc.x + 20, rc.y + 30);
pointer(rowOf(c), "pointermove", fa.x + fa.width / 2, fa.y + fa.height - 20);
await t.sleep(300);
await t.assert(await t.invoke("preview_group_drop", { tab: c, point: [fa.x + fa.width / 2, fa.y + fa.height - 20] }), "the drop would fit");
await t.shot("group-drop-hint");
pointer(rowOf(c), "pointerup", fa.x + fa.width / 2, fa.y + fa.height - 20);
await t.until(() => layout() === JSON.stringify([[b], [a, c]]), "dropped below A: 50/25/25");
await t.assert(S().active === c, "the dropped tab is active");
await t.until(async () => (await sameAs(`tab-${a}`, frame(a))) && (await sameAs(`tab-${c}`, frame(c))), "A and C stacked on the right");
await t.assert(near(frame(c).y, frame(a).y + frame(a).height + INSET) && near(frame(c).height + frame(a).height + INSET, fa.height), "a gap apart, sharing the height");
await t.assert(groupRow().nextElementSibling === rowOf(b) && rowOf(b).nextElementSibling === rowOf(a) && rowOf(a).nextElementSibling === rowOf(c), "C joins the group's rows");

// A terminal below B: quarters, the terminal view in its quarter
const t1 = await t.invoke("new_terminal");
await t.until(() => S().active === t1, "a terminal (alone)");
groupRow().click();
await t.until(() => S().shownGroup === group().id, "back to the group");
const fb = frame(b);
await t.assert(await t.invoke("drop_on_content", { tab: t1, x: fb.x + 30, y: fb.y + fb.height - 10 }), "dropping the terminal below B");
await t.until(() => layout() === JSON.stringify([[b, t1], [a, c]]), "quarters");
await t.until(async () => sameAs("term", frame(t1)), "the terminal view fills its quarter");
await t.until(async () => (await t.read("term", `window.__termText(${t1})`).catch(() => "")).length > 0, "its shell runs", 15000);
const d = await t.invoke("new_tab", { url: `${F}/page2.html?d` });
groupRow().click();
await t.until(() => S().shownGroup === group().id, "the group");
await t.assert(!(await t.invoke("drop_on_content", { tab: d, x: fb.x + 30, y: fb.y + 30 })), "a fifth tab doesn't fit");
await t.sleep(300);
await t.shot("group-quarters");

// Closing a tab: the others fill its space
await t.invoke("close_tab", { id: c });
await t.until(() => layout() === JSON.stringify([[b, t1], [a]]), "C closed: A takes the right column");
await t.until(async () => sameAs(`tab-${a}`, frame(a)) && near(frame(a).height, fa.height), "A full height again");

// A second terminal below A: both terminals on screen, each in its pane
const t2 = await t.invoke("new_terminal");
groupRow().click();
await t.until(() => S().shownGroup === group().id, "the group");
const fa2 = frame(a);
await t.invoke("drop_on_content", { tab: t2, x: fa2.x + 30, y: fa2.y + fa2.height - 10 });
await t.until(() => layout() === JSON.stringify([[b, t1], [a, t2]]), "two terminals in the group");
await t.until(async () => {
  const v = await view("term");
  return near(v.x, frame(t1).x) && near(v.x + v.width, frame(t2).x + frame(t2).width) && near(v.height, frame(t1).height);
}, "the terminal view spans both terminals' panes");
await t.until(async () => (await t.read("term", `window.__termText(${t2})`).catch(() => "")).length > 0, "the second shell runs", 15000);
await t.assert((await t.read("term", `document.querySelectorAll(".box").length`)) === "2", "each terminal in its own box");
await t.invoke("pty_write", { id: t2, pane: 0, data: "echo second-terminal\r" });
await t.until(async () => /^second-terminal$/m.test(await t.read("term", `window.__termText(${t2})`)), "typing reaches the second terminal");
await t.invoke("focus_pane", { id: t1, pane: 0 });
await t.until(() => S().active === t1 && S().shownGroup === group().id, "clicking into a terminal's pane activates it, the group stays");
await t.sleep(400);
await t.shot("group-terminals");

// Rename the group from its row
groupRow().querySelector(".title").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
await t.until(() => groupRow().querySelector(".rename"), "double-click renames");
const input = groupRow().querySelector(".rename");
input.value = "Workbench";
input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
await t.until(() => group().name === "Workbench" && groupRow().textContent.includes("Workbench"), "renamed");

// Resizing the window re-lays out the panes
await t.resize(1000, 700);
await t.until(async () => (await sameAs(`tab-${a}`, frame(a))) && frame(a).x + frame(a).width < 1000, "panes re-laid out after a window resize");
await t.resize(1280, 800);

// Ungroup: the tabs stay, each on its own
groupRow().querySelector(".ungroup").click();
await t.until(() => S().groups.length === 0 && S().shownGroup === null, "ungrouped");
await t.until(async () => near((await view("term")).width, full.width), "the active terminal fills the area");
await t.assert(S().tabs.some((x) => x.id === a) && S().tabs.some((x) => x.id === t2), "its tabs are still open");

// ⌥⌘\ opens a terminal beside the page
await t.invoke("activate_tab", { id: b });
await t.invoke("menu_action", { id: "split-terminal" });
await t.until(() => tab()?.kind === "terminal" && layout() === JSON.stringify([[b], [S().active]]), "a terminal opens beside the page");
await t.until(async () => sameAs("term", frame(S().active)), "in the right pane");
// Making a grouped tab a favorite takes it out
await t.invoke("move_tab", { id: b, zone: "favorite", folder: null, index: null });
await t.until(() => S().groups.length === 0, "a favorite leaves the group (a group of one ends)");

// Dropping on a tab alone makes a group of the two
await t.invoke("activate_tab", { id: a });
await t.until(async () => near((await view(`tab-${a}`)).width, full.width), "A alone");
await t.assert(await t.invoke("drop_on_content", { tab: t2, x: full.x + full.width - 20, y: 300 }), "drop a terminal on A's right edge");
await t.until(() => layout() === JSON.stringify([[a], [t2]]) && S().groups[0].name === "Group 1", "a new group: A and the terminal side by side");

// The command bar offers the group commands
await t.palette.open();
await t.palette.type("split");
await t.until(async () => (await t.palette.rows()).includes("Split View with New Terminal"), "command bar: Split View with New Terminal");
await t.palette.key("Escape");

await t.quit();
