// Keep what didn't change between two states as the same objects, so the
// UI only re-renders the rows that did: every update from Rust is a whole
// new state, even when one tab's title is all that changed.

/** Deep equality for JSON values. */
export function same(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a)) {
    const bb = b as unknown[];
    return a.length === bb.length && a.every((x, i) => same(x, bb[i]));
  }
  const ka = Object.keys(a);
  if (ka.length !== Object.keys(b).length) return false;
  return ka.every((k) => same((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
}

/** `next`, with each item equal to one of `prev` (matched by id) replaced by
 *  that one, and `prev` itself if nothing changed. */
export function shareList<T extends { id: number | string | null }>(prev: T[], next: T[]): T[] {
  const byId = new Map(prev.map((x) => [x.id, x]));
  let changed = prev.length !== next.length;
  const out = next.map((x, i) => {
    const old = byId.get(x.id);
    const kept = old !== undefined && same(old, x) ? old : x;
    if (kept !== prev[i]) changed = true;
    return kept;
  });
  return changed ? out : prev;
}

/** `next` sharing what's unchanged with `prev`: each top-level field, and
 *  the items of its lists (by `id`; plain `same` for lists without ids). */
export function shareState<S extends object>(prev: S, next: S): S {
  const out: Record<string, unknown> = { ...(next as Record<string, unknown>) };
  const was = prev as Record<string, unknown>;
  for (const [key, b] of Object.entries(next)) {
    const a = was[key];
    if (same(a, b)) out[key] = a;
    else if (Array.isArray(a) && Array.isArray(b) && b.every((x) => x && typeof x === "object" && "id" in x)) {
      out[key] = shareList(a as { id: number }[], b as { id: number }[]);
    }
  }
  return out as S;
}
