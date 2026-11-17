import { describe, expect, test } from "bun:test";
import { same, shareList, shareState } from "./share";

describe("sharing unchanged state", () => {
  test("deep equality", () => {
    expect(same({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] })).toBe(true);
    expect(same({ a: [1, { b: 2 }] }, { a: [1, { b: 3 }] })).toBe(false);
    expect(same({ a: 1 }, { a: 1, b: undefined })).toBe(false);
    expect(same([], {})).toBe(false);
    expect(same(null, null)).toBe(true);
  });

  test("unchanged items keep their identity, changed ones don't", () => {
    const prev = [{ id: 1, t: "a" }, { id: 2, t: "b" }];
    const next = shareList(prev, [{ id: 1, t: "a" }, { id: 2, t: "B" }]);
    expect(next[0]).toBe(prev[0]);
    expect(next[1]).not.toBe(prev[1]);
    expect(next[1].t).toBe("B");
  });

  test("an unchanged list is the same list; a reorder is a new one", () => {
    const prev = [{ id: 1 }, { id: 2 }];
    expect(shareList(prev, [{ id: 1 }, { id: 2 }])).toBe(prev);
    const moved = shareList(prev, [{ id: 2 }, { id: 1 }]);
    expect(moved).not.toBe(prev);
    expect(moved[0]).toBe(prev[1]);
  });

  test("whole states", () => {
    const prev = { tabs: [{ id: 1, t: "a" }, { id: 2, t: "b" }], active: 1, screen: [{ tab: 1, x: 0 }] };
    const next = shareState(prev, { tabs: [{ id: 1, t: "a" }, { id: 2, t: "c" }], active: 2, screen: [{ tab: 1, x: 0 }] });
    expect(next.tabs[0]).toBe(prev.tabs[0]);
    expect(next.tabs[1].t).toBe("c");
    expect(next.screen).toBe(prev.screen);
    expect(next.active).toBe(2);
  });
});
