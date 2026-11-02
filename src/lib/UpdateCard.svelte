<script lang="ts">
  import { fly } from "svelte/transition";
  import { browser } from "./browser";
  import { updates, watchUpdates } from "./updates.svelte";

  // Pops up over the bottom of the sidebar when an update turns up, at launch
  // or while Ternt is in use. Later puts it off for that version (the
  // button at the bottom left stays).
  watchUpdates();
  const version = $derived(updates.status.state === "available" ? updates.status.version : null);
</script>

{#if version && updates.dismissed !== version}
  <div class="update-card" role="status" transition:fly={{ y: 12, duration: 180 }}>
    <svg viewBox="0 0 16 16"><path d="M8 3v7.5M4.5 7 8 10.5 11.5 7M4 13h8" /></svg>
    <div class="text">
      <strong>Ternt {version} is ready</strong>
      <span>Restart to update. Your tabs and spaces come back as they are.</span>
    </div>
    <div class="actions">
      <button class="later" onclick={() => (updates.dismissed = version)}>Later</button>
      <button class="restart" onclick={() => browser.installUpdate()}>Restart Now</button>
    </div>
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
</style>
