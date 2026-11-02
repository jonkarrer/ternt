import { browser, type UpdateStatus } from "./browser";

// The updater's status (src-tauri/src/updater.rs), for the card that pops up
// in the sidebar when an update arrives.
export const updates = $state<{ status: UpdateStatus; dismissed: string | null; installing: string | null }>({
  status: { state: "idle" },
  /** The version whose card was put off with Later. */
  dismissed: null,
  /** The version being installed, kept after a failure (to retry it). */
  installing: null,
});

let watching = false;

/** Follow the status for the life of the page (idempotent). */
export function watchUpdates() {
  if (watching) return;
  watching = true;
  // Subscribe first, then read, so no change slips in between.
  browser
    .onUpdate(set)
    .then(() => browser.updateStatus())
    .then(set);
}

function set(status: UpdateStatus) {
  updates.status = status;
  if (status.state === "downloading" || status.state === "installing") updates.installing = status.version;
  else if (status.state !== "failed") updates.installing = null;
}
