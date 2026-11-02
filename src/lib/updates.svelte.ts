import { browser, type UpdateStatus } from "./browser";

// The updater's status (src-tauri/src/updater.rs), shared by the sidebar's
// update button and the card that pops up when an update arrives.
export const updates = $state<{ status: UpdateStatus; dismissed: string | null }>({
  status: { state: "idle" },
  /** The version whose card was put off with Later. */
  dismissed: null,
});

let watching = false;

/** Follow the status for the life of the page (idempotent). */
export function watchUpdates() {
  if (watching) return;
  watching = true;
  // Subscribe first, then read, so no change slips in between.
  browser
    .onUpdate((next) => (updates.status = next))
    .then(() => browser.updateStatus())
    .then((status) => (updates.status = status));
}
