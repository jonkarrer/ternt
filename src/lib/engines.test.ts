import { expect, test } from "bun:test";
import { engineFor, engineUrl } from "./engines";

test("the start of a name, or an alias, picks an engine", () => {
  expect(engineFor("you")?.name).toBe("YouTube");
  expect(engineFor("goo")?.name).toBe("Google");
  expect(engineFor("perp")?.name).toBe("Perplexity");
  expect(engineFor("PERP ")?.name).toBe("Perplexity");
  expect(engineFor("yt")?.name).toBe("YouTube");
  expect(engineFor("gh")?.name).toBe("GitHub");
  expect(engineFor("gi")?.name).toBe("GitHub");
});

test("not one letter (but for an alias), a phrase, or nothing", () => {
  expect(engineFor("y")).toBeNull();
  expect(engineFor("g")?.name).toBe("Google");
  expect(engineFor("you tube")).toBeNull();
  expect(engineFor("youtubez")).toBeNull();
  expect(engineFor("")).toBeNull();
});

test("the query goes in the search URL; nothing opens the home page", () => {
  const youtube = engineFor("yt")!;
  expect(engineUrl(youtube, " lofi & chill ")).toBe("https://www.youtube.com/results?search_query=lofi%20%26%20chill");
  expect(engineUrl(youtube, "  ")).toBe("https://www.youtube.com/");
});

test("tok picks Tokidoki, only while the agent is on", () => {
  expect(engineFor("tok")).toBeNull();
  expect(engineFor("tok", true)?.id).toBe("tokidoki");
  expect(engineFor("Toki", true)?.ask).toBe(true);
  expect(engineFor("t", true)).toBeNull();
  expect(engineFor("you", true)?.name).toBe("YouTube");
});
