import { describe, expect, test } from "bun:test";
import { hostOf, parseInput, SEARCH_URL } from "./url";

describe("parseInput", () => {
  test("empty input is nothing", () => {
    expect(parseInput("")).toBeNull();
    expect(parseInput("   ")).toBeNull();
  });

  test.each([
    ["https://example.com/a?b=c", "https://example.com/a?b=c"],
    ["HTTP://EXAMPLE.COM", "HTTP://EXAMPLE.COM"],
    ["about:blank", "about:blank"],
    ["file:///tmp/x.html", "file:///tmp/x.html"],
    ["  github.com  ", "https://github.com"],
    ["docs.rs/tauri/latest", "https://docs.rs/tauri/latest"],
    ["example.co.uk:8080/x", "https://example.co.uk:8080/x"],
    ["localhost", "http://localhost"],
    ["localhost:1420/index.html", "http://localhost:1420/index.html"],
    ["127.0.0.1:8765", "http://127.0.0.1:8765"],
  ])("%p is a URL", (input, url) => {
    expect(parseInput(input)).toEqual({ kind: "url", url });
  });

  test.each(["tauri multiwebview", "rust", "what is 2.5", "node.js tutorial", "a.b c"])(
    "%p is a search",
    (input) => {
      expect(parseInput(input)).toEqual({ kind: "search", url: SEARCH_URL + encodeURIComponent(input.trim()) });
    },
  );

  test("search terms are encoded", () => {
    expect(parseInput("c++ & rust?")?.url).toBe(`${SEARCH_URL}c%2B%2B%20%26%20rust%3F`);
  });
});

describe("hostOf", () => {
  test("drops www and keeps the port", () => {
    expect(hostOf("https://www.github.com/x")).toBe("github.com");
    expect(hostOf("http://localhost:8765/a")).toBe("localhost:8765");
  });

  test("returns non-URLs unchanged", () => {
    expect(hostOf("not a url")).toBe("not a url");
    expect(hostOf("")).toBe("");
  });
});
