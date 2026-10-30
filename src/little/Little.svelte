<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";
  import Favicon from "../lib/Favicon.svelte";
  import { hostOf } from "../lib/browser";

  // Little Ternt's bar (src-tauri/src/little.rs): the page's address and
  // title over it, and "Open in Ternt" to move it into a tab. 40px tall,
  // mirrored by BAR_HEIGHT in little.rs.
  type LittleState = { url: string; title: string; loading: boolean; space: string };

  let state = $state<LittleState | null>(null);

  onMount(() => {
    const unlisten = getCurrentWebview().listen<LittleState>("little:state", (e) => (state = e.payload));
    unlisten.then(() => invoke<LittleState | null>("little_state")).then((s) => (state ??= s));
    return () => unlisten.then((off) => off());
  });

  const openInTernt = () => invoke("little_open_in_ternt");
</script>

<header data-tauri-drag-region>
  {#if state}
    <span class="site" data-tauri-drag-region>
      <span class="favicon"><Favicon url={state.url} /></span>
      <span class="host" data-tauri-drag-region>{hostOf(state.url)}</span>
      {#if state.title}<span class="title" data-tauri-drag-region>{state.title}</span>{/if}
      {#if state.loading}<span class="loading" aria-label="Loading"></span>{/if}
    </span>
    <button class="open" title="Move this page into a tab in {state.space || 'Ternt'}" onclick={openInTernt}>
      Open in Ternt
      <svg viewBox="0 0 16 16"><path d="M6 3.5h6.5V10M12.5 3.5 4 12" /></svg>
    </button>
  {/if}
</header>

<style>
  /* theme.css makes pages transparent (for the sidebar's vibrancy); this
     window has none. */
  :global(html body) {
    background: var(--surface-raised);
  }
  header {
    height: 40px;
    background: var(--surface-raised);
    display: flex;
    align-items: center;
    gap: 10px;
    /* The traffic lights sit at the left. */
    padding: 0 8px 0 78px;
    border-bottom: 0.5px solid var(--separator);
  }
  .site {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .favicon {
    flex: none;
    display: grid;
    width: 16px;
    height: 16px;
  }
  .host {
    flex: none;
    font-weight: 600;
  }
  .title {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--text-muted);
  }
  .loading {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid var(--text-muted);
    border-right-color: transparent;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .open {
    flex: none;
    display: flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px;
    border-radius: 7px;
    background: var(--fill-strong);
    font-size: 12px;
    font-weight: 500;
  }
  .open:hover {
    background: var(--accent);
    color: white;
  }
  svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
