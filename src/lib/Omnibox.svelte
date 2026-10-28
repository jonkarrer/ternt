<script lang="ts">
  import { tick } from "svelte";
  import { browser, hostOf, toUrl, type Tab } from "./browser";

  let { tab }: { tab: Tab | undefined } = $props();

  let input = $state<HTMLInputElement>();
  let editing = $state(false);
  let draft = $state("");
  let newTab = false;

  /** Focus the field: empty for a new tab, or prefilled with the current URL. */
  export async function begin(asNewTab: boolean) {
    newTab = asNewTab || !tab;
    draft = newTab ? "" : (tab?.url ?? "");
    editing = true;
    await tick();
    input?.focus();
    input?.select();
  }

  function submit(event: SubmitEvent) {
    event.preventDefault();
    const url = toUrl(draft);
    if (!url) return;
    editing = false;
    if (newTab) browser.newTab(url);
    else browser.navigate(url);
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") input?.blur();
  }
</script>

{#if editing}
  <form class="omnibox editing" onsubmit={submit}>
    <input
      bind:this={input}
      bind:value={draft}
      {onkeydown}
      onblur={() => (editing = false)}
      placeholder="Search or enter URL…"
      spellcheck="false"
      autocomplete="off"
    />
  </form>
{:else}
  <button class="omnibox" onclick={() => begin(false)}>
    {#if tab}
      <span class="host">{hostOf(tab.url)}</span>
    {:else}
      <span class="placeholder">Search or enter URL…</span>
    {/if}
  </button>
{/if}

<style>
  .omnibox {
    display: flex;
    align-items: center;
    width: 100%;
    height: 34px;
    padding: 0 10px;
    border-radius: 10px;
    background: var(--fill);
    color: var(--text);
    text-align: left;
    transition: background-color 120ms ease;
  }
  button.omnibox:hover {
    background: var(--fill-hover);
  }
  .editing {
    background: var(--surface);
    box-shadow:
      0 0 0 1px var(--ring),
      0 2px 8px var(--shadow);
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
  input {
    width: 100%;
    border: 0;
    outline: 0;
    background: none;
    color: inherit;
    font: inherit;
  }
  input::placeholder {
    color: var(--text-muted);
  }
</style>
