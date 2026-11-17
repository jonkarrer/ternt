// The Downloads box (src/lib/DownloadsBox.svelte): a download shows up with
// its progress, can be canceled, opens with a click and is kept across
// launches (downloads-2). Files go to e2e/.out/downloads in tests.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = t.F;
const SLOW = "http://localhost:8766/slow";
const list = () => t.invoke("downloads");
const saved = () => t.invoke("debug_downloads");
const box = () => document.querySelector("section.downloads");
const eval_ = (id, js) => t.invoke("debug_eval", { label: `tab-${id}`, js });

const page = await t.invoke("new_tab", { url: `${F}/links.html` });
await t.until(() => tab()?.title === "Fixture Links" && !tab().loading, "a page of links");
await t.assert(!document.querySelector(".downloads-button"), "no Downloads button before any download");

// A text file sent as an attachment downloads instead of showing in the tab
await eval_(page, `document.getElementById("text-attachment").click()`);
await t.until(async () => (await saved()).some(([p, ok, text]) => p.endsWith("notes.txt") && ok && text === "plain notes"), "an attached text file is saved");
await t.assert(tab().url.endsWith("/links.html"), "not shown in the tab");
await t.until(() => box()?.textContent.includes("notes.txt"), "the Downloads box opens with it");
await t.assert(document.querySelector(".downloads-button"), "and the Downloads button shows");

// Progress while it downloads (1 MB over ~3 s)
await eval_(page, `location.href = "${SLOW}?name=big.bin"`);
await t.until(async () => (await list()).some((d) => d.name === "big.bin" && d.state === "active" && d.received > 0 && d.received < d.total && d.total === 1_000_000), "a slow download shows its progress", 8000);
await t.until(() => document.querySelector(".downloads-button .ring") && box()?.querySelector("li.active .bar"), "a progress bar and a ring around the button");
await t.until(() => /of 1\.0 MB/.test(box().querySelector("li.active .detail")?.textContent ?? ""), "with how much of the size");
await t.shot("downloads-progress");
await t.until(async () => (await list()).some((d) => d.name === "big.bin" && d.state === "done" && d.received === 1_000_000), "then it's done", 10000);
await t.until(() => !document.querySelector(".downloads-button .ring"), "and the ring goes");

// Two downloads of the same file at once are told apart
await eval_(page, `[0, 1].forEach(() => { const f = document.createElement("iframe"); f.src = "${SLOW}?name=twin.bin&chunks=6&ms=100"; document.body.append(f); })`);
await t.until(async () => (await list()).filter((d) => d.name.startsWith("twin") && d.state === "done").length === 2, "two downloads of one URL both finish", 10000);
const twins = (await list()).filter((d) => d.name.startsWith("twin"));
await t.assert(twins[0].path !== twins[1].path && twins.every((d) => d.received === 300_000), "each to its own file, whole");

// Cancel
await eval_(page, `location.href = "${SLOW}?name=cancel.bin&chunks=100&ms=100&unknown"`);
await t.until(async () => (await list()).some((d) => d.name === "cancel.bin" && d.state === "active" && d.received > 0 && d.total == null), "a download of unknown size", 8000);
await t.until(() => document.querySelector(".ring.unknown") && box().querySelector("li.active .bar.unknown"), "spins rather than fills");
box().querySelector("li.active .act").click();
await t.until(async () => (await list()).find((d) => d.name === "cancel.bin")?.state === "canceled", "Cancel stops it");
await t.sleep(800);
await t.assert((await list()).find((d) => d.name === "cancel.bin")?.state === "canceled", "and it stays canceled");
await t.assert(!(await saved()).some(([p]) => p.endsWith("cancel.bin")), "without finishing");
await t.until(() => box().querySelector("li.canceled")?.textContent.includes("Canceled"), "the box says so");

// A link opened in a new tab that's only a file: the empty tab closes
const tabs = S().tabs.length;
await eval_(page, `(() => { const r = document.getElementById("blank-download").getBoundingClientRect(); document.title = "at " + (r.x + 5) + "," + (r.y + r.height / 2); })()`);
await t.until(() => tab()?.title.startsWith("at "), "find the new-tab download link");
const [x, y] = tab().title.slice(3).split(",").map(Number);
const before = (await saved()).length;
await t.invoke("debug_click", { label: `tab-${page}`, x, y });
await t.until(async () => (await saved()).length > before && /report( \(\d+\))?\.csv$/.test((await saved()).at(-1)[0]), "a new-tab link to a file downloads it");
await t.until(() => S().tabs.length === tabs && S().active === page, "and its empty tab closes, back to the page");

// Opening and showing in Finder (recorded in tests)
const opened = () => t.invoke("debug_opened");
const notes = box().querySelector("li:last-child");
notes.querySelector(".file").click();
await t.until(async () => (await opened()).some((o) => o.startsWith("open:") && o.endsWith("notes.txt")), "a click opens the file");
notes.querySelector(".act").click();
await t.until(async () => (await opened()).some((o) => o.startsWith("reveal:") && o.endsWith("notes.txt")), "the magnifier shows it in Finder");

// Remove the canceled one from the list; the box toggles from the button
box().querySelector("li.canceled .act").click();
await t.until(async () => !(await list()).some((d) => d.name === "cancel.bin"), "a canceled download comes off the list");
document.querySelector(".downloads-button").click();
await t.until(() => !box(), "the button hides the box");
document.querySelector(".downloads-button").click();
await t.until(() => box(), "and shows it again");
await t.sleep(5500);
await t.assert(box(), "opened by hand, it stays open");
await t.shot("downloads");
