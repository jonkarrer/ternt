import { browser, type Download } from "./browser";

// Downloads (src-tauri/src/downloads.rs), for the sidebar's Downloads box and
// its button. The box opens when a download starts and closes a little after
// the last one finishes, unless it was opened by hand.
export const downloads = $state<{ list: Download[]; open: boolean; byHand: boolean }>({
  list: [],
  open: false,
  byHand: false,
});

/** Bytes per second, smoothed, by download id. */
const rates = new Map<number, { at: number; received: number; rate: number }>();
let closing: ReturnType<typeof setTimeout> | undefined;
let watching = false;

/** How long the box stays after the last download ends. */
const CLOSE_AFTER = 5000;

/** Follow the list for the life of the page (idempotent). */
export function watchDownloads() {
  if (watching) return;
  watching = true;
  browser
    .onDownloads((list) => set(list, true))
    .then(() => browser.downloads())
    .then((list) => set(list, false));
}

function set(list: Download[], live: boolean) {
  const known = new Set(downloads.list.map((d) => d.id));
  const now = performance.now();
  for (const d of list) {
    if (d.state !== "active") {
      rates.delete(d.id);
      continue;
    }
    const last = rates.get(d.id);
    if (!last) rates.set(d.id, { at: now, received: d.received, rate: 0 });
    else if (d.received !== last.received && now > last.at) {
      const sample = ((d.received - last.received) * 1000) / (now - last.at);
      rates.set(d.id, { at: now, received: d.received, rate: last.rate ? last.rate * 0.7 + sample * 0.3 : sample });
    }
  }
  downloads.list = list;
  const active = list.some((d) => d.state === "active");
  if (live && list.some((d) => d.state === "active" && !known.has(d.id))) {
    downloads.open = true;
  }
  clearTimeout(closing);
  if (downloads.open && !active && !downloads.byHand) {
    closing = setTimeout(() => (downloads.open = false), CLOSE_AFTER);
  }
  if (!list.length) downloads.open = false;
}

/** The footer button: show or hide the box. */
export function toggleDownloads() {
  clearTimeout(closing);
  downloads.open = !downloads.open;
  downloads.byHand = downloads.open;
}

/** The pointer is over the box: keep it open while it's read. */
export function holdDownloads() {
  clearTimeout(closing);
  downloads.byHand = true;
}

export function rateOf(id: number): number {
  return rates.get(id)?.rate ?? 0;
}
