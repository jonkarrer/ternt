import { expect, test } from "bun:test";
import { formatBytes, formatLeft, formatProgress } from "./bytes";

test("sizes count in 1000s, like Finder", () => {
  expect(formatBytes(512)).toBe("512 bytes");
  expect(formatBytes(4_200)).toBe("4.2 KB");
  expect(formatBytes(12_000_000)).toBe("12 MB");
  expect(formatBytes(1_500_000_000)).toBe("1.5 GB");
});

test("progress shares the unit when it can", () => {
  expect(formatProgress(4_200_000, 12_000_000)).toBe("4.2 of 12 MB");
  expect(formatProgress(800_000, 12_000_000)).toBe("800 KB of 12 MB");
  expect(formatProgress(800_000, null)).toBe("800 KB");
});

test("time left", () => {
  expect(formatLeft(0.4)).toBe("1 s left");
  expect(formatLeft(150)).toBe("3 min left");
  expect(formatLeft(Infinity)).toBe("");
});
