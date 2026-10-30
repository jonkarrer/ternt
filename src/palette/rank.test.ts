import { describe, expect, test } from "bun:test";
import type { ClosedTab, HistoryHit, Space, Tab } from "../lib/browser";
import { SEARCH_URL } from "../lib/url";
import { buildItems, CLOSED_SUGGESTIONS, COMMANDS, score, SECTION_LIMIT, type Context, type Item } from "./rank";

let nextId = 1;
function tab(title: string, url: string, extra: Partial<Tab> = {}): Tab {
  return {
    id: nextId++,
    kind: "web",
    url,
    title,
    zone: "today",
    space: 1,
    folder: null,
    loaded: true,
    loading: false,
    canGoBack: false,
    canGoForward: false,
    ...extra,
  };
}

const home: Space = { id: 1, name: "Home", color: "blue", profile: null, icon: null };
const work: Space = { id: 2, name: "Work", color: "green", profile: "abc", icon: null };

function ctx(over: Partial<Context> = {}): Context {
  return { mode: "newTab", tabs: [], spaces: [home, work], activeId: null, activeSpace: 1, history: [], closed: [], splits: [], ...over };
}

const labels = (items: Item[]) => items.map((i) => i.label);
const top = (items: Item[]) => items.filter((i) => i.section === "top");

describe("score", () => {
  test("prefix > word starts > initials > substring > nothing", () => {
    expect(score("New Terminal", "new")).toBe(100);
    expect(score("New Terminal", "term")).toBe(80);
    expect(score("Reopen Closed Tab", "closed tab")).toBe(80);
    expect(score("New Terminal", "nt")).toBe(70);
    expect(score("New Terminal", "rmin")).toBe(40);
    expect(score("New Terminal", "space")).toBe(0);
  });

  test("words split on punctuation too", () => {
    expect(score("docs.rs/tauri", "tauri")).toBe(80);
  });
});

describe("empty query", () => {
  test("suggests commands, then other tabs (this space first), then other spaces", () => {
    const a = tab("A", "https://a.com");
    const b = tab("B", "https://b.com", { space: 2 });
    const c = tab("C", "https://c.com");
    const items = buildItems("", ctx({ tabs: [b, c, a], activeId: a.id }));
    const sections = [...new Set(items.map((i) => i.section))];
    expect(sections).toEqual(["Suggestions", "Tabs", "Spaces"]);
    expect(items.filter((i) => i.section === "Tabs").map((i) => i.label)).toEqual(["C", "B"]);
    expect(items.some((i) => i.tab?.id === a.id)).toBe(false);
    expect(items.filter((i) => i.section === "Spaces").map((i) => i.label)).toEqual(["Go to Work"]);
  });

  test("caps the tab list", () => {
    const tabs = Array.from({ length: 12 }, (_, i) => tab(`T${i}`, `https://t${i}.com`));
    const items = buildItems("", ctx({ tabs }));
    expect(items.filter((i) => i.section === "Tabs")).toHaveLength(SECTION_LIMIT);
  });
});

describe("typed queries", () => {
  test("a URL comes first and opens in a new tab (⌘T) or in place (⌘L)", () => {
    const [first] = buildItems("github.com", ctx());
    expect(first.action).toEqual({ type: "open", url: "https://github.com", newTab: true });
    const [edit] = buildItems("github.com", ctx({ mode: "edit" }));
    expect(edit.action).toEqual({ type: "open", url: "https://github.com", newTab: false });
  });

  test("plain text offers a Google search", () => {
    const items = buildItems("rust borrow checker", ctx());
    expect(items.at(-1)?.label).toBe("Search Google for “rust borrow checker”");
    expect(items.at(-1)?.action).toEqual({
      type: "open",
      url: SEARCH_URL + encodeURIComponent("rust borrow checker"),
      newTab: true,
    });
  });

  test("a tab whose title matches beats the search", () => {
    const t = tab("Fixture Home", "http://localhost/");
    const items = buildItems("fixture", ctx({ tabs: [t] }));
    expect(labels(top(items))).toEqual(["Fixture Home", "Search Google for “fixture”"]);
    expect(items[0].action).toEqual({ type: "tab", id: t.id });
  });

  test("a URL-only tab match stays below the search", () => {
    const t = tab("Something", "https://example.com/fixture");
    const items = buildItems("fixture", ctx({ tabs: [t] }));
    expect(items[0].kind).toBe("search");
    expect(items[1]).toMatchObject({ section: "Tabs", label: "Something" });
  });

  test("the active tab isn't offered", () => {
    const t = tab("Fixture", "http://x/");
    expect(buildItems("fixture", ctx({ tabs: [t], activeId: t.id })).some((i) => i.kind === "tab")).toBe(false);
  });

  test("named commands beat the search; initials work", () => {
    expect(buildItems("new term", ctx())[0]).toMatchObject({ label: "New Terminal", action: { type: "menu", id: "new-terminal" } });
    expect(buildItems("nt", ctx())[0].label).toBe("New Terminal");
    expect(buildItems("terminal", ctx())[0].label).toBe("New Terminal");
  });

  test("keywords find commands without beating the search", () => {
    const items = buildItems("shell", ctx());
    expect(items[0].kind).toBe("search");
    expect(items.find((i) => i.section === "Commands")?.label).toBe("New Terminal");
  });

  test("spaces by name", () => {
    expect(buildItems("work", ctx())[0]).toMatchObject({ label: "Go to Work", action: { type: "space", id: 2 } });
    expect(buildItems("home", ctx()).some((i) => i.kind === "space"), "not the active space").toBe(false);
  });

  test("tabs in other spaces name their space", () => {
    const t = tab("Mail", "https://mail.example", { space: 2 });
    expect(buildItems("mail", ctx({ tabs: [t] }))[0].detail).toBe("Work");
  });
});

describe("commands that need a tab", () => {
  const pinned = tab("Docs", "https://docs.example", { zone: "pinned" });
  const term = tab("zsh", "", { kind: "terminal" });
  const ids = (c: Partial<Context>) =>
    buildItems("tab", ctx(c))
      .filter((i) => i.kind === "command")
      .map((i) => (i.action.type === "menu" ? i.action.id : ""));

  test("hidden without an active tab", () => {
    expect(ids({})).not.toContain("close-tab");
    expect(ids({}).some((id) => id.startsWith("ctx-"))).toBe(false);
  });

  test("ctx- actions target the active tab; already-pinned hides Pin", () => {
    const got = ids({ tabs: [pinned], activeId: pinned.id });
    expect(got).toContain("close-tab");
    expect(got).toContain(`ctx-duplicate:${pinned.id}`);
    expect(buildItems("pin", ctx({ tabs: [pinned], activeId: pinned.id })).some((i) => i.label === "Pin Tab")).toBe(false);
  });

  test("web-only commands hide on terminals", () => {
    const items = buildItems("reload", ctx({ tabs: [term], activeId: term.id }));
    expect(items.some((i) => i.label === "Reload Page")).toBe(false);
  });

  test("every command has a unique id", () => {
    expect(new Set(COMMANDS.map((c) => c.id)).size).toBe(COMMANDS.length);
  });
});

describe("history", () => {
  const hits: HistoryHit[] = [
    { url: "https://www.github.com/", title: "GitHub" },
    { url: "https://news.site/gh", title: "GitHub trending" },
  ];

  test("an address prefix match goes above the search", () => {
    const items = buildItems("gith", ctx({ history: hits }));
    expect(items[0]).toMatchObject({ kind: "history", label: "GitHub", section: "top" });
    expect(items[0].action).toEqual({ type: "open", url: "https://www.github.com/", newTab: true });
    expect(items.find((i) => i.section === "History")?.label).toBe("GitHub trending");
  });

  test("a title-only match stays in the History section", () => {
    const items = buildItems("trending", ctx({ history: hits }));
    expect(items[0].kind).toBe("search");
    expect(items.find((i) => i.kind === "history")).toMatchObject({ section: "History", label: "GitHub trending" });
  });

  test("pages already open as tabs are skipped", () => {
    const open = tab("GitHub", "https://www.github.com/");
    const items = buildItems("git", ctx({ tabs: [open], history: hits }));
    expect(items.filter((i) => i.url === "https://www.github.com/").map((i) => i.kind)).toEqual(["tab"]);
  });

  test("stale hits that no longer match are dropped", () => {
    expect(buildItems("gitlab", ctx({ history: hits })).some((i) => i.kind === "history")).toBe(false);
  });

  test("not offered for a typed URL", () => {
    const items = buildItems("github.com", ctx({ history: hits }));
    expect(items[0].kind).toBe("url");
    expect(top(items)).toHaveLength(1);
  });
});

describe("> runs in a terminal", () => {
  test("offers only the command", () => {
    const items = buildItems(">  npm test ", ctx({ history: [{ url: "https://npm.com", title: "npm" }] }));
    expect(items).toEqual([expect.objectContaining({ kind: "run", label: "Run “npm test”", action: { type: "terminal", command: "npm test" } })]);
  });

  test("bare > opens a plain terminal", () => {
    expect(buildItems(">", ctx())[0].action).toEqual({ type: "terminal", command: "" });
  });
});

let nextClosed = 1;
function closedTab(title: string, url: string, extra: Partial<ClosedTab> = {}): ClosedTab {
  return { id: nextClosed++, kind: "web", url, title, space: 1, ...extra };
}

describe("recently closed", () => {
  test("offered newest first before anything is typed, a few of them", () => {
    const closed = ["a", "b", "c", "d"].map((n) => closedTab(n.toUpperCase(), `https://${n}.com`));
    const items = buildItems("", ctx({ closed })).filter((i) => i.section === "Recently Closed");
    expect(labels(items)).toEqual(["D", "C", "B"].slice(0, CLOSED_SUGGESTIONS));
    expect(items[0].action).toEqual({ type: "closed", id: closed[3].id });
  });

  test("matches by title or address, skipping terminals, open pages and repeats", () => {
    const closed = [
      closedTab("Rust docs", "https://doc.rust-lang.org"),
      closedTab("Rust docs", "https://doc.rust-lang.org"),
      closedTab("Terminal", "", { kind: "terminal" }),
      closedTab("Rust blog", "https://blog.rust-lang.org"),
    ];
    const tabs = [tab("Blog", "https://blog.rust-lang.org")];
    const items = buildItems("rust", ctx({ closed, tabs }));
    const hits = items.filter((i) => i.kind === "closed");
    expect(labels(hits)).toEqual(["Rust docs"]);
    expect(hits[0].action).toEqual({ type: "closed", id: closed[1].id });
  });

  test("a closed tab named like the query is a top match; history doesn't repeat it", () => {
    const closed = [closedTab("Linear", "https://linear.app")];
    const history: HistoryHit[] = [
      { url: "https://linear.app", title: "Linear" },
      { url: "https://lineage.com", title: "Lineage" },
    ];
    const items = buildItems("linear", ctx({ closed, history }));
    expect(items[0].kind).toBe("closed");
    expect(items.filter((i) => i.url === "https://linear.app").length).toBe(1);
  });

  test("typing the start of a closed page's address still completes to it, once", () => {
    const closed = [closedTab("Fixture Page 2", "http://localhost:8765/page2.html")];
    const history: HistoryHit[] = [{ url: "http://localhost:8765/page2.html", title: "Fixture Page 2" }];
    const items = buildItems("localh", ctx({ closed, history }));
    expect(items[0].url).toBe("http://localhost:8765/page2.html");
    expect(items.filter((i) => i.url === "http://localhost:8765/page2.html").length).toBe(1);
  });

  test("other spaces' closed tabs say where they were", () => {
    const closed = [closedTab("Jira", "https://jira.com", { space: 2 })];
    expect(buildItems("", ctx({ closed })).find((i) => i.kind === "closed")?.detail).toBe("Closed · Work");
  });
});

describe("split view commands", () => {
  test("Close Split View only when the active tab is in a split", () => {
    const a = tab("A", "https://a.com");
    const b = tab("B", "https://b.com");
    const has = (over: Partial<Context>) => labels(buildItems("split", ctx({ tabs: [a, b], activeId: a.id, ...over })));
    expect(has({})).not.toContain("Close Split View");
    expect(has({ splits: [{ left: a.id, right: b.id, ratio: 0.5 }] })).toContain("Close Split View");
    expect(has({})).toContain("Split View with New Terminal");
  });
});
