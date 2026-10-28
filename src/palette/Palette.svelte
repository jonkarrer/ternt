<script lang="ts">
  import { onMount, tick } from "svelte";
  import Favicon from "../lib/Favicon.svelte";
  import TabIcon from "../lib/TabIcon.svelte";
  import { browser, hostOf, parseInput, type PaletteRequest, type Tab } from "../lib/browser";

  type Item = {
    key: string;
    kind: "url" | "search" | "tab" | "command";
    label: string;
    detail?: string;
    url?: string;
    tab?: Tab;
    run: () => Promise<unknown>;
  };

  /** Actions offered by name. Ids are menu action ids (see src-tauri/src/menu.rs). */
  const COMMANDS = [
    { id: "new-terminal", label: "New Terminal" },
    { id: "reload", label: "Reload Page" },
    { id: "back", label: "Go Back" },
    { id: "forward", label: "Go Forward" },
    { id: "close-tab", label: "Close Tab" },
    { id: "reopen-tab", label: "Reopen Closed Tab" },
  ];
  const MAX_ITEMS = 8;

  let request = $state<PaletteRequest | null>(null);
  let query = $state("");
  let selected = $state(0);
  let tabs = $state<Tab[]>([]);
  let activeId = $state<number | null>(null);
  let input = $state<HTMLInputElement>();

  const items = $derived(buildItems(query));

  onMount(() => {
    const unlisten = browser.onPaletteOpen(show);
    // A request made before this page loaded is waiting in Rust; subscribe
    // first so none can slip in between.
    unlisten.then(() => browser.paletteRequest()).then((pending) => pending && show(pending));
    return () => unlisten.then((off) => off());
  });

  async function show(next: PaletteRequest) {
    request = next;
    query = next.url ?? "";
    selected = 0;
    await tick();
    input?.focus();
    input?.select();
    const state = await browser.getState();
    tabs = state.tabs;
    activeId = state.active;
  }

  function close() {
    request = null;
    return browser.closePalette();
  }

  function matches(text: string, words: string[]) {
    const haystack = text.toLowerCase();
    return words.every((w) => haystack.includes(w));
  }

  function buildItems(raw: string): Item[] {
    if (!request) return [];
    const q = raw.trim();
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    const newTab = request.mode === "newTab";
    const out: Item[] = [];

    const tabMatches = tabs.filter(
      (t) => t.id !== activeId && (words.length === 0 || matches(`${t.title} ${t.url}`, words)),
    );
    const tabItem = (t: Tab): Item => ({
      key: `tab-${t.id}`,
      kind: "tab",
      label: t.title || hostOf(t.url),
      detail: "Switch to tab",
      url: t.url,
      tab: t,
      run: () => browser.activateTab(t.id),
    });

    const parsed = parseInput(q);
    // A tab whose title contains every typed word beats a web search.
    const best = parsed?.kind === "search" ? tabMatches.find((t) => matches(t.title, words)) : undefined;
    if (best) out.push(tabItem(best));

    if (parsed) {
      const open = (url: string) => (newTab ? browser.newTab(url) : browser.navigate(url));
      const searchQuery = parsed.kind === "search" ? q : null;
      out.push({
        key: "primary",
        kind: parsed.kind,
        label: searchQuery ? `Search for “${searchQuery}”` : parsed.url,
        detail: searchQuery ? "Google" : newTab ? "Open in new tab" : "Open",
        url: parsed.url,
        run: () => open(parsed.url),
      });
    }

    for (const t of tabMatches) {
      if (t !== best) out.push(tabItem(t));
    }

    if (words.length > 0) {
      for (const c of COMMANDS.filter((c) => matches(c.label, words))) {
        out.push({
          key: `cmd-${c.id}`,
          kind: "command",
          label: c.label,
          detail: "Command",
          run: () => browser.menuAction(c.id),
        });
      }
    }
    return out.slice(0, MAX_ITEMS);
  }

  async function run(item: Item | undefined) {
    if (!item) return;
    await close();
    await item.run();
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
</script>

{#if request}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={(e) => e.target === e.currentTarget && close()}>
    <div class="panel" role="dialog" aria-label="Command bar">
      <input
        bind:this={input}
        bind:value={query}
        {onkeydown}
        placeholder={request.mode === "newTab" ? "Search or enter URL…" : "Enter URL…"}
        spellcheck="false"
        autocomplete="off"
        role="combobox"
        aria-expanded={items.length > 0}
        aria-controls="palette-results"
      />
      {#if items.length}
        <ul id="palette-results" role="listbox">
          {#each items as item, i (item.key)}
            <li
              role="option"
              aria-selected={i === selected}
              class:selected={i === selected}
              onpointermove={() => (selected = i)}
              onpointerdown={(e) => {
                e.preventDefault();
                run(item);
              }}
            >
              <span class="icon">
                {#if item.kind === "search"}
                  <svg viewBox="0 0 16 16"><circle cx="7" cy="7" r="4.5" /><path d="m10.5 10.5 3 3" /></svg>
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
            </li>
          {/each}
        </ul>
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
    left: 50%;
    width: min(680px, calc(100vw - 48px));
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
