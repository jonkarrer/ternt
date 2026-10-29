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

export function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return url;
  }
}
