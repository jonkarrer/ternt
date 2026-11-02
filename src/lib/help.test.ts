import { describe, expect, test } from "bun:test";
import { filterHelp, HOW_TOS, SHORTCUTS } from "./help";

describe("help sheet", () => {
  test("everything with no query", () => {
    const all = filterHelp("  ");
    expect(all.shortcuts).toEqual(SHORTCUTS);
    expect(all.howTos).toEqual(HOW_TOS);
  });

  test("every word must match, in any field", () => {
    const { shortcuts, howTos } = filterHelp("split down");
    expect(shortcuts.flatMap((g) => g.items.map((s) => s.keys))).toEqual(["⇧⌘D"]);
    expect(howTos.map((h) => h.title)).toEqual(["Split a terminal"]);
  });

  test("a group title matches its shortcuts", () => {
    const { shortcuts } = filterHelp("terminals");
    expect(shortcuts.map((g) => g.title)).toEqual(["Terminals"]);
    expect(shortcuts[0].items.length).toBe(SHORTCUTS.find((g) => g.title === "Terminals")!.items.length);
  });

  test("by key", () => {
    expect(filterHelp("⌘F").shortcuts[0].items[0].action).toMatch(/Explore/);
  });

  test("nothing", () => {
    const none = filterHelp("zzzz");
    expect(none.shortcuts).toEqual([]);
    expect(none.howTos).toEqual([]);
  });
});
