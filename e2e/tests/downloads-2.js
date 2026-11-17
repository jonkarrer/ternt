// Finished downloads are kept across launches (downloads-1 made them).
const list = () => t.invoke("downloads");
const box = () => document.querySelector("section.downloads");

const names = (await list()).map((d) => d.name);
await t.assert(names[0] === "report.csv" && names.includes("notes.txt") && names.includes("big.bin"), "the list comes back, newest first");
await t.assert(!names.includes("cancel.bin"), "without the one taken off it");
await t.assert((await list()).every((d) => d.state === "done"), "all done");
await t.assert(!box(), "the box stays closed at launch");

document.querySelector(".downloads-button").click();
await t.until(() => box()?.querySelectorAll("li").length === names.length, "the button shows them");
box().querySelector(".clear").click();
await t.until(async () => (await list()).length === 0, "Clear empties the list");
await t.until(() => !box() && !document.querySelector(".downloads-button"), "and the box and button go");
