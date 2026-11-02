<script lang="ts">
  import { fly } from "svelte/transition";
  import { browser } from "./browser";
  import { updates, watchUpdates } from "./updates.svelte";

  // Pops up over the bottom of the sidebar when an update turns up, at launch
  // or while Ternt is in use, and stays through the download and restart
  // (src-tauri/src/updater.rs). Later puts it off for that version; Settings →
  // General can still install it. Also answers Check for Updates… (up to date,
  // or why it failed: only checks the user asked for report failures).
  watchUpdates();
  const status = $derived(updates.status);
  const version = $derived("version" in status ? status.version : null);
  const failed = $derived(status.state === "failed");
  const shown = $derived(
    status.state !== "idle" && !(status.state === "available" && updates.dismissed === status.version),
  );
  const progress = $derived(
    status.state === "downloading" ? (status.progress ?? 0) : status.state === "installing" ? 1 : 0,
  );

  function install() {
    // A failed install keeps its update to retry; a failed check checks again.
    if (failed && !updates.installing) browser.checkForUpdates();
    else browser.installUpdate();
  }
  function later() {
    updates.dismissed = version ?? updates.installing;
    updates.installing = null;
    if (failed) updates.status = { state: "idle" };
  }
</script>

{#if shown}
  <div class="update-card" class:failed role="status" transition:fly={{ y: 12, duration: 180 }}>
    {#if status.state === "upToDate"}
      <svg viewBox="0 0 16 16"><path d="m4 8.5 2.5 2.5L12 5.5" /></svg>
    {:else}
      <svg viewBox="0 0 16 16"><path d="M8 3v7.5M4.5 7 8 10.5 11.5 7M4 13h8" /></svg>
    {/if}
    <div class="text">
      {#if status.state === "available"}
        <strong>Ternt {status.version} is ready</strong>
        <span>Restart to update. Your tabs and spaces come back as they are.</span>
      {:else if status.state === "downloading"}
        <strong>Updating to {status.version}…</strong>
        <span>{status.progress == null ? "Downloading" : `Downloading ${Math.round(status.progress * 100)}%`}</span>
      {:else if status.state === "installing"}
        <strong>Restarting…</strong>
        <span>Ternt {status.version} opens in a moment.</span>
      {:else if status.state === "upToDate"}
        <strong>Ternt is up to date</strong>
        <span>You have the latest version.</span>
      {:else if status.state === "failed"}
        <strong>{updates.installing ? "The update failed" : "Couldn't check for updates"}</strong>
        <span title={status.message}>{status.message}</span>
      {/if}
    </div>
    {#if status.state === "downloading" || status.state === "installing"}
      <div class="progress" style:--progress={progress}></div>
    {:else if status.state !== "upToDate"}
      <div class="actions">
        <button class="later" onclick={later}>{failed ? "Close" : "Later"}</button>
        <button class="restart" onclick={install}>{failed ? "Try Again" : "Restart Now"}</button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .update-card {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 10px;
    margin: 8px 0 2px;
    padding: 12px;
    border-radius: 12px;
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--ring),
      0 6px 20px var(--shadow);
  }
  svg {
    width: 18px;
    height: 18px;
    padding: 4px;
    box-sizing: content-box;
    border-radius: 50%;
    background: var(--accent);
    color: white;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  strong {
    font-size: 12px;
    font-weight: 600;
  }
  span {
    font-size: 11px;
    color: var(--text-muted);
  }
  .actions {
    grid-column: 1 / -1;
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    margin-top: 6px;
  }
  button {
    height: 24px;
    padding: 0 10px;
    border-radius: 7px;
    font-size: 11px;
    font-weight: 600;
  }
  .later {
    background: var(--fill-strong);
  }
  .later:hover {
    background: var(--fill-hover);
  }
  .restart {
    background: var(--accent);
    color: white;
  }
  .restart:hover {
    filter: brightness(1.08);
  }
  .failed svg {
    background: #d9534f;
  }
  .failed span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Download progress. */
  .progress {
    grid-column: 1 / -1;
    height: 4px;
    margin-top: 6px;
    border-radius: 2px;
    background: linear-gradient(var(--accent), var(--accent)) no-repeat 0 0 / calc(var(--progress) * 100%) 100%,
      var(--fill-strong);
    transition: background-size 120ms linear;
  }
</style>
