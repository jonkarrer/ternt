<script lang="ts">
  import { onMount } from "svelte";
  import { core, type UpdateStatus } from "./core";

  // Shown at the foot of the sidebar once an update has downloaded: it installs when
  // Tokidoki quits, or now with Restart now (src-tauri/src/updater.rs). Also answers
  // Check for updates.
  let status = $state<UpdateStatus>({ state: "idle" });
  let later = $state<string | null>(null);

  onMount(() => {
    // Subscribe first, then read, so no change slips in between.
    const stop = core.onUpdate((next) => (status = next));
    core.updateStatus().then((now) => (status = now));
    return () => void stop.then((off) => off());
  });

  const shown = $derived(status.state !== "idle" && !(status.state === "ready" && later === status.version));
</script>

{#if shown}
  <div class="card" role="status">
    {#if status.state === "ready"}
      <strong>Tokidoki {status.version} is ready</strong>
      <span>It installs when you quit. Anything under way picks up where it stopped.</span>
      <div class="actions">
        <button onclick={() => (later = status.state === "ready" ? status.version : null)}>Later</button>
        <button class="primary" onclick={() => core.installUpdate()}>Restart now</button>
      </div>
    {:else if status.state === "installing"}
      <strong>Restarting…</strong>
      <span>Tokidoki {status.version} opens in a moment.</span>
    {:else if status.state === "up_to_date"}
      <strong>Tokidoki is up to date</strong>
      <div class="actions"><button onclick={() => (status = { state: "idle" })}>Close</button></div>
    {:else if status.state === "failed"}
      <strong>The update didn't go through</strong>
      <span class="selectable">{status.message}</span>
      <div class="actions">
        <button onclick={() => (status = { state: "idle" })}>Close</button>
        <button class="primary" onclick={() => core.checkForUpdates()}>Check again</button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 11px 12px;
    border-radius: 10px;
    background: var(--raised);
    border: 1px solid var(--line);
    font-size: 12px;
  }
  span {
    color: var(--muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    margin-top: 7px;
  }
  button {
    height: 24px;
    padding: 0 10px;
    border-radius: 6px;
    background: var(--side);
    font-size: 12px;
    font-weight: 600;
  }
  .primary {
    background: var(--indigo);
    color: var(--paper);
  }
</style>
