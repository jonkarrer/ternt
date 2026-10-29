<script lang="ts">
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import { browser, type UpdateStatus } from "./browser";

  // Bottom-left of the sidebar: appears when an update is available; a click
  // downloads it, swaps the app and relaunches (src-tauri/src/updater.rs).
  let status = $state<UpdateStatus>({ state: "idle" });

  onMount(() => {
    const unlisten = browser.onUpdate((next) => (status = next));
    // Subscribe first, then read, so no change slips in between.
    unlisten.then(() => browser.updateStatus()).then((s) => (status = s));
    return () => unlisten.then((off) => off());
  });

  const busy = $derived(status.state === "downloading" || status.state === "installing");
  const label = $derived.by(() => {
    switch (status.state) {
      case "available":
        return "Update";
      case "downloading":
        return status.progress == null ? "Updating…" : `Updating ${Math.round(status.progress * 100)}%`;
      case "installing":
        return "Restarting…";
      case "upToDate":
        return "Up to date";
      case "failed":
        return "Retry update";
      default:
        return "";
    }
  });
  const title = $derived.by(() => {
    switch (status.state) {
      case "available":
        return `Ternt ${status.version} is available. Click to install and restart.`;
      case "downloading":
      case "installing":
        return `Installing Ternt ${status.version}…`;
      case "failed":
        return `The update failed: ${status.message}`;
      default:
        return "";
    }
  });

  function onclick() {
    if (status.state === "available" || status.state === "failed") browser.installUpdate();
  }
</script>

{#if status.state !== "idle"}
  <button
    class="update"
    class:busy
    class:quiet={status.state === "upToDate"}
    class:failed={status.state === "failed"}
    style:--progress={status.state === "downloading" ? (status.progress ?? 0) : status.state === "installing" ? 1 : 0}
    {title}
    disabled={busy || status.state === "upToDate"}
    {onclick}
    transition:fly={{ x: -8, duration: 160 }}
  >
    {#if status.state === "upToDate"}
      <svg viewBox="0 0 16 16"><path d="m4 8.5 2.5 2.5L12 5.5" /></svg>
    {:else}
      <svg viewBox="0 0 16 16"><path d="M8 3v7.5M4.5 7 8 10.5 11.5 7M4 13h8" /></svg>
    {/if}
    <span>{label}</span>
  </button>
{/if}

<style>
  .update {
    position: relative;
    height: 22px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px 0 6px;
    border-radius: 11px;
    overflow: hidden;
    font-size: 11px;
    font-weight: 600;
    color: white;
    background: var(--accent);
    box-shadow: 0 1px 2px var(--shadow);
    white-space: nowrap;
  }
  .update:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  /* Download progress fills the pill from the left. */
  .update.busy {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .update.busy::before {
    content: "";
    position: absolute;
    inset: 0;
    width: calc(var(--progress) * 100%);
    background: var(--accent);
    transition: width 120ms linear;
  }
  .update > * {
    position: relative;
  }
  .update.quiet {
    background: var(--fill-strong);
    color: var(--text-muted);
    box-shadow: none;
  }
  .update.failed {
    background: #d9534f;
  }
  svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
