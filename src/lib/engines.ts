// Search engines for the command bar: type the start of one's name (or an
// alias) and press Tab, and the query goes to that site (`you` ⇥ YouTube).
// Pure (no Tauri imports) so it's unit-testable.

export type Engine = {
  id: string;
  name: string;
  /** Short names that pick it too. */
  aliases: string[];
  /** The query is appended, URL-encoded. */
  search: string;
  /** Opened when there's nothing to search for. */
  home: string;
  /** The pill's color. */
  color: string;
};

export const ENGINES: Engine[] = [
  {
    id: "google",
    name: "Google",
    aliases: ["g"],
    search: "https://www.google.com/search?q=",
    home: "https://www.google.com/",
    color: "#4285f4",
  },
  {
    id: "youtube",
    name: "YouTube",
    aliases: ["yt"],
    search: "https://www.youtube.com/results?search_query=",
    home: "https://www.youtube.com/",
    color: "#ff0033",
  },
  {
    id: "perplexity",
    name: "Perplexity",
    aliases: ["pplx"],
    search: "https://www.perplexity.ai/search?q=",
    home: "https://www.perplexity.ai/",
    color: "#20808d",
  },
  {
    id: "chatgpt",
    name: "ChatGPT",
    aliases: ["gpt"],
    search: "https://chatgpt.com/?q=",
    home: "https://chatgpt.com/",
    color: "#10a37f",
  },
  {
    id: "claude",
    name: "Claude",
    aliases: [],
    search: "https://claude.ai/new?q=",
    home: "https://claude.ai/new",
    color: "#d97757",
  },
  {
    id: "github",
    name: "GitHub",
    aliases: ["gh"],
    search: "https://github.com/search?q=",
    home: "https://github.com/",
    color: "#6e5494",
  },
  {
    id: "wikipedia",
    name: "Wikipedia",
    aliases: ["wiki"],
    search: "https://en.wikipedia.org/w/index.php?search=",
    home: "https://en.wikipedia.org/",
    color: "#636466",
  },
  {
    id: "maps",
    name: "Maps",
    aliases: ["gmaps"],
    search: "https://www.google.com/maps/search/",
    home: "https://www.google.com/maps",
    color: "#34a853",
  },
  {
    id: "amazon",
    name: "Amazon",
    aliases: ["amz"],
    search: "https://www.amazon.com/s?k=",
    home: "https://www.amazon.com/",
    color: "#ff9900",
  },
  {
    id: "reddit",
    name: "Reddit",
    aliases: [],
    search: "https://www.reddit.com/search/?q=",
    home: "https://www.reddit.com/",
    color: "#ff4500",
  },
];

/** The engine a typed word picks: an alias exactly, else the start of a
    name (two letters at least, so a one-letter search isn't taken over). */
export function engineFor(typed: string): Engine | null {
  const word = typed.trim().toLowerCase();
  if (!word || /\s/.test(word)) return null;
  const byAlias = ENGINES.find((e) => e.aliases.includes(word));
  if (byAlias) return byAlias;
  if (word.length < 2) return null;
  return ENGINES.find((e) => e.name.toLowerCase().startsWith(word)) ?? null;
}

/** Where a search for `query` goes (its home page with nothing to search). */
export function engineUrl(engine: Engine, query: string): string {
  const q = query.trim();
  return q ? engine.search + encodeURIComponent(q) : engine.home;
}
