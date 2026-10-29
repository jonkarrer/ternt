// Command bar results: what to offer for a query, in what order. Pure data
// in, data out (no Tauri calls), so it's unit-tested in rank.test.ts; the
// palette performs each item's `action`.
import type { HistoryHit, PaletteMode, Space, Tab } from "../lib/browser";
import { hostOf, parseInput } from "../lib/url";

/** `top` holds the best matches, shown without a header. */
export type Section = "top" | "Suggestions" | "Tabs" | "History" | "Commands" | "Spaces";

export type Action =
  | { type: "open"; url: string; newTab: boolean }
  | { type: "tab"; id: number }
  | { type: "space"; id: number }
  /** A menu action id (see src-tauri/src/menu.rs). */
  | { type: "menu"; id: string }
  | { type: "terminal"; command: string };

export type Item = {
  key: string;
  kind: "url" | "search" | "tab" | "history" | "command" | "space" | "run";
  label: string;
  detail?: string;
  shortcut?: string;
  url?: string;
  tab?: Tab;
  space?: Space;
  section: Section;
  action: Action;
};

export type Command = {
  /** Menu action id (see src-tauri/src/menu.rs). */
  id: string;
  label: string;
  shortcut?: string;
  /** Extra words that find it. */
  keywords?: string;
  /** Offered before anything is typed. */
  suggest?: boolean;
  /** Only when the active tab is a web page. */
  web?: boolean;
  /** Needs an active tab. `ctx-` actions get its id: `<id>:<tab>`. */
  tab?: boolean;
  /** Hidden when the active tab is already in this zone. */
  unless?: Tab["zone"];
};

export const COMMANDS: Command[] = [
  { id: "new-terminal", label: "New Terminal", shortcut: "⌥⌘T", keywords: "shell console", suggest: true },
  { id: "explore", label: "Explore Tabs", shortcut: "⌘F", keywords: "switch browse find", suggest: true },
  { id: "new-space", label: "New Space", keywords: "profile", suggest: true },
  { id: "reopen-tab", label: "Reopen Closed Tab", shortcut: "⇧⌘T", keywords: "undo restore", suggest: true },
  { id: "copy-url", label: "Copy URL", keywords: "link clipboard", web: true },
  { id: "ctx-favorite", label: "Add to Favorites", keywords: "star", web: true, tab: true, unless: "favorite" },
  { id: "ctx-pin", label: "Pin Tab", web: true, tab: true, unless: "pinned" },
  { id: "ctx-duplicate", label: "Duplicate Tab", keywords: "copy", tab: true },
  { id: "clear-today", label: "Clear Today Tabs", keywords: "close all archive" },
  { id: "clear-history", label: "Clear History", keywords: "browsing forget" },
  { id: "next-space", label: "Next Space", shortcut: "⌥⌘→" },
  { id: "previous-space", label: "Previous Space", shortcut: "⌥⌘←" },
  { id: "new-folder", label: "New Folder" },
  { id: "toggle-sidebar", label: "Toggle Sidebar", shortcut: "⌘S", keywords: "hide show" },
  { id: "reload", label: "Reload Page", shortcut: "⌘R", keywords: "refresh", web: true },
  { id: "back", label: "Go Back", shortcut: "⌘[", web: true },
  { id: "forward", label: "Go Forward", shortcut: "⌘]", web: true },
  { id: "close-tab", label: "Close Tab", shortcut: "⌘W", tab: true },
];

/** Per section, below the top matches. */
export const SECTION_LIMIT = 5;

export interface Context {
  mode: PaletteMode;
  tabs: Tab[];
  spaces: Space[];
  activeId: number | null;
  activeSpace: number;
  /** Visited pages matching the query (from Rust). */
  history: HistoryHit[];
}

/** How well `text` matches the lowercase query `q`: 0 (no match) to 100. */
export function score(text: string, q: string): number {
  const t = text.toLowerCase();
  if (t.startsWith(q)) return 100;
  const words = t.split(/[\s\-_/.:]+/).filter(Boolean);
  const terms = q.split(/\s+/);
  if (terms.every((term) => words.some((w) => w.startsWith(term)))) return 80;
  // "nt" → New Terminal
  if (terms.length === 1 && words.map((w) => w[0]).join("").startsWith(q)) return 70;
  if (terms.every((term) => t.includes(term))) return 40;
  return 0;
}

/** `https://www.github.com/x` → `github.com/x`. */
const bare = (url: string) => url.replace(/^[a-z]+:\/\/(www\.)?/i, "").toLowerCase();

function ranked<T>(list: T[], rate: (x: T) => number) {
  return list
    .map((x) => ({ x, s: rate(x) }))
    .filter((r) => r.s > 0)
    .sort((a, b) => b.s - a.s);
}

export function buildItems(raw: string, ctx: Context): Item[] {
  const { tabs, spaces, activeId, activeSpace } = ctx;
  const q = raw.trim();
  const lq = q.toLowerCase();
  const newTab = ctx.mode === "newTab";
  const activeTab = tabs.find((t) => t.id === activeId);
  const spaceName = (id: number) => spaces.find((s) => s.id === id)?.name ?? "";

  // "> command" runs in a new terminal.
  if (q.startsWith(">")) {
    const command = q.slice(1).trim();
    return [
      {
        key: "run",
        kind: "run",
        label: command ? `Run “${command}”` : "Type a command to run in a new terminal",
        detail: "New terminal",
        section: "top",
        action: { type: "terminal", command },
      },
    ];
  }

  const commands = COMMANDS.filter(
    (c) =>
      (!c.web || activeTab?.kind === "web") && (!c.tab || activeTab) && (!c.unless || activeTab?.zone !== c.unless),
  );
  const commandItem = (c: Command, section: Section): Item => ({
    key: `cmd-${c.id}`,
    kind: "command",
    label: c.label,
    shortcut: c.shortcut,
    section,
    action: { type: "menu", id: c.id.startsWith("ctx-") ? `${c.id}:${activeId}` : c.id },
  });
  const tabItem = (t: Tab, section: Section): Item => ({
    key: `tab-${t.id}`,
    kind: "tab",
    label: t.title || hostOf(t.url),
    detail: t.zone === "favorite" ? "Favorite" : t.space === activeSpace ? "Switch to tab" : spaceName(t.space),
    url: t.url,
    tab: t,
    section,
    action: { type: "tab", id: t.id },
  });
  const spaceItem = (s: Space, section: Section): Item => ({
    key: `space-${s.id}`,
    kind: "space",
    label: `Go to ${s.name}`,
    detail: s.profile ? "Space · own profile" : "Space",
    space: s,
    section,
    action: { type: "space", id: s.id },
  });
  const historyItem = (h: HistoryHit, section: Section): Item => ({
    key: `history-${h.url}`,
    kind: "history",
    label: h.title || hostOf(h.url),
    detail: hostOf(h.url),
    url: h.url,
    section,
    action: { type: "open", url: h.url, newTab },
  });
  const otherTabs = tabs.filter((t) => t.id !== activeId);
  const otherSpaces = spaces.filter((s) => s.id !== activeSpace);

  // Nothing typed: what you can do, where you can go.
  if (!q) {
    return [
      ...commands.filter((c) => c.suggest).map((c) => commandItem(c, "Suggestions")),
      // This space's tabs first.
      ...[...otherTabs]
        .sort((a, b) => Number(b.space === activeSpace) - Number(a.space === activeSpace))
        .slice(0, SECTION_LIMIT)
        .map((t) => tabItem(t, "Tabs")),
      ...otherSpaces.map((s) => spaceItem(s, "Spaces")),
    ];
  }

  const tabHits = ranked(otherTabs, (t) => Math.max(score(t.title, lq), t.url.toLowerCase().includes(lq) ? 40 : 0));
  const commandHits = ranked(commands, (c) =>
    Math.max(score(c.label, lq), c.keywords && score(c.keywords, lq) >= 80 ? 50 : 0),
  );
  const spaceHits = ranked(otherSpaces, (s) => Math.max(score(s.name, lq), score("space", lq) >= 80 ? 50 : 0));
  // History: pages not already open, still matching every typed word (the
  // hits may be from the previous keystroke).
  const historyHits = ctx.history.filter(
    (h) =>
      !tabs.some((t) => t.url === h.url) &&
      lq.split(/\s+/).every((w) => `${h.url} ${h.title}`.toLowerCase().includes(w)),
  );

  // Top: a typed URL first; then a tab whose title matches (beats a web
  // search), a visited address starting with the query, and a clearly named
  // command or space; then the web search.
  const top: Item[] = [];
  const parsed = parseInput(q);
  const open: Item | null = parsed && {
    key: "primary",
    kind: parsed.kind,
    label: parsed.kind === "search" ? `Search Google for “${q}”` : parsed.url,
    detail: parsed.kind === "search" ? "Google" : newTab ? "Open in new tab" : "Open",
    url: parsed.url,
    section: "top",
    action: { type: "open", url: parsed.url, newTab },
  };
  if (open && parsed?.kind === "url") top.push(open);
  const bestTab = tabHits[0] && score(tabHits[0].x.title, lq) > 0 ? tabHits.shift()!.x : undefined;
  if (bestTab) top.push(tabItem(bestTab, "top"));
  if (parsed?.kind !== "url" && historyHits[0] && bare(historyHits[0].url).startsWith(lq)) {
    top.push(historyItem(historyHits.shift()!, "top"));
  }
  if (commandHits[0]?.s >= 70) top.push(commandItem(commandHits.shift()!.x, "top"));
  if (spaceHits[0]?.s >= 80) top.push(spaceItem(spaceHits.shift()!.x, "top"));
  if (open && parsed?.kind === "search") top.push(open);

  return [
    ...top,
    ...tabHits.slice(0, SECTION_LIMIT).map((r) => tabItem(r.x, "Tabs")),
    ...historyHits.slice(0, SECTION_LIMIT).map((h) => historyItem(h, "History")),
    ...commandHits.slice(0, SECTION_LIMIT).map((r) => commandItem(r.x, "Commands")),
    ...spaceHits.map((r) => spaceItem(r.x, "Spaces")),
  ];
}
