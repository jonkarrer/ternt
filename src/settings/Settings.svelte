<script lang="ts">
  import { onMount } from "svelte";
  import { browser, type BrowserState } from "../lib/browser";
  import Advanced from "./Advanced.svelte";
  import General from "./General.svelte";
  import Links from "./Links.svelte";
  import Profiles from "./Profiles.svelte";
  import Shortcuts from "./Shortcuts.svelte";
  import Ternt from "./Ternt.svelte";

  // Arc-style: a row of icon tabs in the title bar, one page below.
  const PAGES = [
    { id: "ternt", label: "Ternt", icon: "M8 1.75a6.25 6.25 0 1 0 0 12.5 6.25 6.25 0 0 0 0-12.5ZM5 5.5h6M8 5.5v5.5" },
    { id: "general", label: "General", icon: "M8 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5ZM8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" },
    { id: "profiles", label: "Profiles", icon: "M8 2.5a2.75 2.75 0 1 0 0 5.5 2.75 2.75 0 0 0 0-5.5ZM2.75 14c.4-2.9 2.6-4.5 5.25-4.5s4.85 1.6 5.25 4.5" },
    { id: "links", label: "Links", icon: "M6.75 9.25a2.6 2.6 0 0 0 3.7 0l2.3-2.3a2.6 2.6 0 0 0-3.7-3.7l-.8.8M9.25 6.75a2.6 2.6 0 0 0-3.7 0l-2.3 2.3a2.6 2.6 0 0 0 3.7 3.7l.8-.8" },
    { id: "shortcuts", label: "Shortcuts", icon: "M6 6h4v4H6zM6 6V4.5A1.5 1.5 0 1 0 4.5 6H6M10 6V4.5A1.5 1.5 0 1 1 11.5 6H10M6 10v1.5A1.5 1.5 0 1 1 4.5 10H6M10 10v1.5a1.5 1.5 0 1 0 1.5-1.5H10" },
    { id: "advanced", label: "Advanced", icon: "M2.5 4.5h6M11.5 4.5h2M2.5 11.5h2M7.5 11.5h6M10 3v3M6 10v3" },
  ] as const;
  type Page = (typeof PAGES)[number]["id"];

  let page = $state<Page>("ternt");
  let browserState = $state<BrowserState | null>(null);

  onMount(() => {
    const unlisten = browser.onState((s) => (browserState = s));
    // Subscribe first, then read, so no change slips in between.
    unlisten.then(() => browser.getState()).then((s) => (browserState ??= s));
    return () => unlisten.then((off) => off());
  });

  function keydown(event: KeyboardEvent) {
    // ⌘1–⌘6 switch pages, as in most macOS settings windows.
    const index = Number(event.key) - 1;
    if (event.metaKey && index >= 0 && index < PAGES.length) {
      page = PAGES[index].id;
      event.preventDefault();
    }
  }
</script>

<svelte:window onkeydown={keydown} />

<div class="window">
  <nav class="toolbar" data-tauri-drag-region>
    {#each PAGES as item (item.id)}
      <button
        class="page-tab"
        class:selected={page === item.id}
        aria-pressed={page === item.id}
        onclick={() => (page = item.id)}
      >
        <svg viewBox="0 0 16 16"><path d={item.icon} /></svg>
        <span>{item.label}</span>
      </button>
    {/each}
  </nav>

  <main class:wide={page === "profiles"}>
    {#if page === "ternt"}
      {#if browserState}
        <Ternt {browserState} />
      {/if}
    {:else if page === "general"}
      <General />
    {:else if page === "profiles"}
      {#if browserState}
        <Profiles {browserState} />
      {/if}
    {:else if page === "links"}
      <Links />
    {:else if page === "shortcuts"}
      <Shortcuts />
    {:else}
      <Advanced />
    {/if}
  </main>
</div>

<style>
  .window {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  /* The title bar: traffic lights sit at the left, tabs centered. */
  .toolbar {
    flex: none;
    display: flex;
    justify-content: center;
    gap: 2px;
    padding: 10px 80px 8px;
    border-bottom: 0.5px solid var(--separator);
  }
  .page-tab {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    width: 72px;
    padding: 6px 0 5px;
    border-radius: 8px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .page-tab:hover {
    background: var(--fill);
  }
  .page-tab.selected {
    background: var(--fill-strong);
    color: var(--text);
  }
  .page-tab.selected svg {
    stroke: var(--accent);
  }
  svg {
    width: 22px;
    height: 22px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.3;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  main {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 40px;
  }
  /* Profiles brings its own two-column layout. */
  main.wide {
    padding: 0;
    overflow: hidden;
  }
</style>
