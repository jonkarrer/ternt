<script lang="ts">
  import Favicon from "./Favicon.svelte";
  import type { Tab } from "./browser";

  let { tab }: { tab: Pick<Tab, "kind" | "url" | "loading"> } = $props();
</script>

{#if tab.kind === "terminal"}
  <span class="terminal" aria-hidden="true">
    <svg viewBox="0 0 16 16"><path d="m3.5 5 3 3-3 3M8.5 11.5h4" /></svg>
  </span>
{:else}
  <Favicon url={tab.url} loading={tab.loading} />
{/if}

<style>
  .terminal {
    flex: none;
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
    border-radius: 4px;
    background: var(--text);
    color: var(--surface);
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
