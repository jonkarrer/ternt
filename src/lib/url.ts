// Address-bar input handling. Pure (no Tauri imports) so it's unit-testable.

export const SEARCH_URL = "https://www.google.com/search?q=";

export type ParsedInput = { kind: "url" | "search"; url: string };

/** Interpret address-bar input: explicit URL, bare host, or a search. */
export function parseInput(raw: string): ParsedInput | null {
  const input = raw.trim();
  if (!input) return null;
  if (/^(https?|file|about|data):/i.test(input)) return { kind: "url", url: input };
  if (/^(localhost|\d{1,3}(\.\d{1,3}){3})(:\d+)?(\/\S*)?$/i.test(input)) {
    return { kind: "url", url: `http://${input}` };
  }
  if (/^[^\s/]+\.[a-z]{2,}(:\d+)?(\/\S*)?$/i.test(input)) {
    return { kind: "url", url: `https://${input}` };
  }
  return { kind: "search", url: SEARCH_URL + encodeURIComponent(input) };
}

/** What the sidebar shows for a URL: the host for web pages, the whole URL
    (minus a trailing slash) for app pages like `ternt://welcome`. */
export function hostOf(url: string): string {
  try {
    const u = new URL(url);
    if (!u.protocol.startsWith("http")) return url.replace(/\/$/, "");
    return u.host.replace(/^www\./, "");
  } catch {
    return url;
  }
}

/** Pages Ternt serves itself (the welcome page). */
export function isAppPage(url: string): boolean {
  return url.startsWith("ternt://");
}
