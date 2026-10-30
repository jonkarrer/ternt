<script lang="ts">
  import { onMount, tick } from "svelte";
  import Favicon from "../lib/Favicon.svelte";
  import TabIcon from "../lib/TabIcon.svelte";
  import { browser, type ClosedTab, type HistoryHit, type PaletteRequest, type Space, type Tab } from "../lib/browser";
  import { buildItems, type Action, type Item } from "./rank";

  let request = $state<PaletteRequest | null>(null);
  let query = $state("");
  let selected = $state(0);
  let tabs = $state<Tab[]>([]);
  let spaces = $state<Space[]>([]);
  let activeId = $state<number | null>(null);
  let activeSpace = $state(0);
  let closed = $state<ClosedTab[]>([]);
  let input = $state<HTMLInputElement>();
  let list = $state<HTMLElement>();

  /** Visited pages for the current query, from Rust (kept until the next
      answer arrives, so the list doesn't flicker per keystroke). */
  let history = $state<HistoryHit[]>([]);
  let historySeq = 0;

  const items = $derived(
    request ? buildItems(query, { mode: request.mode, tabs, spaces, activeId, activeSpace, history, closed }) : [],
  );

  $effect(() => {
    const q = query.trim();
    const seq = ++historySeq;
    if (!request || !q || q.startsWith(">")) {
      history = [];
      return;
    }
    browser.searchHistory(q, 8).then((hits) => {
      if (seq === historySeq) history = hits;
    });
  });

  onMount(() => {
    const unlisten = browser.onPaletteOpen(show);
    // A request made before this page loaded is waiting in Rust; subscribe
    // first so none can slip in between.
    unlisten.then(() => browser.paletteRequest()).then((pending) => pending && show(pending));
    return () => unlisten.then((off) => off());
  });

  async function show(next: PaletteRequest) {
    request = next;
    lastPointer = "";
    query = next.url ?? "";
    selected = 0;
    await tick();
    input?.focus();
    input?.select();
    const state = await browser.getState();
    tabs = state.tabs;
    spaces = state.spaces;
    activeId = state.active;
    activeSpace = state.activeSpace;
    closed = state.closed;
  }

  function close() {
    request = null;
    return browser.closePalette();
  }

  // WebKit sends pointermove when the list appears under a resting cursor;
  // only a real move should take the selection from the keyboard.
  let lastPointer = "";
  function hover(event: PointerEvent, index: number) {
    const at = `${event.screenX},${event.screenY}`;
    if (at === lastPointer) return;
    const first = lastPointer === "";
    lastPointer = at;
    if (!first) selected = index;
  }

  function perform(action: Action) {
    switch (action.type) {
      case "open":
        return action.newTab ? browser.newTab(action.url) : browser.navigate(action.url);
      case "tab":
        return browser.activateTab(action.id);
      case "closed":
        return browser.reopenClosed(action.id);
      case "space":
        return browser.activateSpace(action.id);
      case "menu":
        return browser.menuAction(action.id);
      case "terminal":
        return browser.newTerminal(action.command);
    }
  }

  async function run(item: Item | undefined) {
    if (!item) return;
    await close();
    await perform(item.action);
  }

  function onkeydown(event: KeyboardEvent) {
    const down = event.key === "ArrowDown" || (event.ctrlKey && event.key === "n");
    const up = event.key === "ArrowUp" || (event.ctrlKey && event.key === "p");
    if (down || up) {
      event.preventDefault();
      if (items.length) selected = (selected + (down ? 1 : -1) + items.length) % items.length;
    } else if (event.key === "Enter") {
      event.preventDefault();
      run(items[selected]);
    } else if (event.key === "Escape") {
      event.preventDefault();
      close();
    }
  }

  $effect(() => {
    // Reset the highlight whenever the result set changes.
    void query;
    selected = 0;
  });

  $effect(() => {
    list?.querySelector(`[data-index="${selected}"]`)?.scrollIntoView({ block: "nearest" });
  });
</script>

{#if request}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="backdrop"
    style:--content-left="{request.contentLeft}px"
    onclick={(e) => e.target === e.currentTarget && close()}
  >
    <div class="panel" role="dialog" aria-label="Command bar">
      <input
        bind:this={input}
        bind:value={query}
        {onkeydown}
        placeholder={request.mode === "newTab" ? "Search, open a URL, or type a command…" : "Enter URL…"}
        spellcheck="false"
        autocomplete="off"
        role="combobox"
        aria-expanded={items.length > 0}
        aria-controls="palette-results"
      />
      {#if items.length}
        <ul id="palette-results" role="listbox" bind:this={list}>
          {#each items as item, i (item.key)}
            {#if item.section !== "top" && item.section !== items[i - 1]?.section}
              <li class="section" role="presentation">{item.section}</li>
            {/if}
            <li
              role="option"
              data-index={i}
              aria-selected={i === selected}
              class:selected={i === selected}
              onpointermove={(e) => hover(e, i)}
              onpointerdown={(e) => {
                e.preventDefault();
                run(item);
              }}
            >
              <span class="icon">
                {#if item.kind === "search"}
                  <svg viewBox="0 0 16 16"><circle cx="7" cy="7" r="4.5" /><path d="m10.5 10.5 3 3" /></svg>
                {:else if item.kind === "run"}
                  <svg viewBox="0 0 16 16"><path d="m3.5 5 3 3-3 3M8.5 11.5h4" /></svg>
                {:else if item.space}
                  <svg viewBox="0 0 16 16"><rect x="2.5" y="2.5" width="11" height="11" rx="3" /><path d="M6 2.5v11" /></svg>
                {:else if item.tab}
                  <TabIcon tab={item.tab} />
                {:else if item.kind === "command"}
                  <svg viewBox="0 0 16 16"><path d="m6 4 4 4-4 4" /></svg>
                {:else}
                  <Favicon url={item.url ?? ""} />
                {/if}
              </span>
              <span class="label">{item.label}</span>
              {#if item.detail}<span class="detail">{item.detail}</span>{/if}
              {#if item.shortcut}<kbd>{item.shortcut}</kbd>{/if}
            </li>
          {/each}
        </ul>
      {/if}
      {#if request.mode === "newTab"}
        <footer>
          <span><kbd>↑</kbd><kbd>↓</kbd> select</span>
          <span><kbd>↵</kbd> run</span>
          <span><kbd>&gt;</kbd> run in terminal</span>
          <span><kbd>esc</kbd> close</span>
        </footer>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.2);
    animation: fade 120ms ease-out;
  }
  .panel {
    position: absolute;
    top: 16vh;
    /* Centered over the content pane (right of the sidebar), not the window. */
    left: calc(var(--content-left) + (100vw - var(--content-left) - var(--inset)) / 2);
    width: min(680px, calc(100vw - var(--content-left) - var(--inset) - 48px));
    translate: -50% 0;
    border-radius: 14px;
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--ring),
      0 24px 64px rgb(0 0 0 / 0.3);
    overflow: hidden;
    animation: rise 140ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  input {
    display: block;
    width: 100%;
    height: 56px;
    padding: 0 20px;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: 18px;
  }
  input::placeholder {
    color: var(--text-muted);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 6px;
    border-top: 0.5px solid var(--ring);
  }
  li {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 40px;
    padding: 0 12px;
    border-radius: 8px;
    font-size: 14px;
  }
  li.selected {
    background: var(--accent);
    color: white;
  }
  li.selected .detail,
  li.selected .icon {
    color: rgb(255 255 255 / 0.8);
  }
  .icon {
    display: grid;
    place-items: center;
    width: 16px;
    color: var(--text-muted);
  }
  svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .detail {
    color: var(--text-muted);
    font-size: 12px;
  }
  ul {
    max-height: min(440px, 56vh);
    overflow-y: auto;
  }
  li.section {
    height: 28px;
    padding: 6px 12px 0;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
  }
  kbd {
    min-width: 18px;
    padding: 1px 5px;
    border-radius: 5px;
    background: var(--fill-strong);
    font: inherit;
    font-size: 11px;
    text-align: center;
    color: var(--text-muted);
  }
  li.selected kbd {
    background: rgb(255 255 255 / 0.2);
    color: white;
  }
  footer {
    display: flex;
    gap: 14px;
    padding: 8px 14px;
    border-top: 0.5px solid var(--ring);
    font-size: 11px;
    color: var(--text-muted);
  }
  footer span {
    display: flex;
    align-items: center;
    gap: 3px;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      scale: 0.98;
    }
  }
</style>
