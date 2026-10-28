<script lang="ts">
  import { browser, hostOf, type Tab } from "./browser";

  let { tab }: { tab: Tab | undefined } = $props();
</script>

<!-- Arc-style: the sidebar shows the host; clicking opens the command bar
     prefilled with the full URL. -->
<button class="omnibox" onclick={() => browser.openPalette("edit")}>
  {#if tab}
    <span class="host">{hostOf(tab.url)}</span>
  {:else}
    <span class="placeholder">Search or enter URL…</span>
  {/if}
</button>

<style>
  .omnibox {
    display: flex;
    align-items: center;
    width: 100%;
    height: 34px;
    flex: none;
    padding: 0 10px;
    border-radius: 10px;
    background: var(--fill);
    color: var(--text);
    text-align: left;
    transition: background-color 120ms ease;
  }
  .omnibox:hover {
    background: var(--fill-hover);
  }
  .host,
  .placeholder {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .placeholder {
    color: var(--text-muted);
  }
</style>
