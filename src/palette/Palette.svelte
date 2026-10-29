<script lang="ts">
  import { onMount, tick } from "svelte";
  import Favicon from "../lib/Favicon.svelte";
  import TabIcon from "../lib/TabIcon.svelte";
  import { browser, hostOf, parseInput, type PaletteRequest, type Space, type Tab } from "../lib/browser";

  /** `top` holds the best matches, shown without a header. */
  type Section = "top" | "Suggestions" | "Tabs" | "Commands" | "Spaces";
  type Item = {
    key: string;
    kind: "url" | "search" | "tab" | "command" | "space" | "run";
    label: string;
    detail?: string;
    shortcut?: string;
    url?: string;
    tab?: Tab;
    space?: Space;
    section: Section;
    run: () => Promise<unknown>;
  };

  type Command = {
    /** Menu action id (see src-tauri/src/menu.rs). */
    id: string;
    label: string;
    shortcut?: string;
    /** Extra words that find it. */
    keywords?: string;
    /** Offered before anything is typed. */
    suggest?: boolean;
    /** Only when the active tab is a web page. */
    web?: boolean;
    /** Needs an active tab. `ctx-` actions get its id: `<id>:<tab>`. */
    tab?: boolean;
    /** Hidden when the active tab is already in this zone. */
    unless?: Tab["zone"];
  };

  const COMMANDS: Command[] = [
    { id: "new-terminal", label: "New Terminal", shortcut: "⌥⌘T", keywords: "shell console", suggest: true },
    { id: "explore", label: "Explore Tabs", shortcut: "⌘F", keywords: "switch browse find", suggest: true },
    { id: "new-space", label: "New Space", keywords: "profile", suggest: true },
    { id: "reopen-tab", label: "Reopen Closed Tab", shortcut: "⇧⌘T", keywords: "undo restore", suggest: true },
    { id: "copy-url", label: "Copy URL", keywords: "link clipboard", web: true },
    { id: "ctx-favorite", label: "Add to Favorites", keywords: "star", web: true, tab: true, unless: "favorite" },
    { id: "ctx-pin", label: "Pin Tab", web: true, tab: true, unless: "pinned" },
    { id: "ctx-duplicate", label: "Duplicate Tab", keywords: "copy", tab: true },
    { id: "clear-today", label: "Clear Today Tabs", keywords: "close all archive" },
    { id: "next-space", label: "Next Space", shortcut: "⌥⌘→" },
    { id: "previous-space", label: "Previous Space", shortcut: "⌥⌘←" },
    { id: "new-folder", label: "New Folder" },
    { id: "toggle-sidebar", label: "Toggle Sidebar", shortcut: "⌘S", keywords: "hide show" },
    { id: "reload", label: "Reload Page", shortcut: "⌘R", keywords: "refresh", web: true },
    { id: "back", label: "Go Back", shortcut: "⌘[", web: true },
    { id: "forward", label: "Go Forward", shortcut: "⌘]", web: true },
    { id: "close-tab", label: "Close Tab", shortcut: "⌘W", tab: true },
  ];
  /** Per section, below the top matches. */
  const SECTION_LIMIT = 5;

  let request = $state<PaletteRequest | null>(null);
  let query = $state("");
  let selected = $state(0);
  let tabs = $state<Tab[]>([]);
  let spaces = $state<Space[]>([]);
  let activeId = $state<number | null>(null);
  let activeSpace = $state(0);
  let input = $state<HTMLInputElement>();
  let list = $state<HTMLElement>();

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
  }

  function close() {
    request = null;
    return browser.closePalette();
  }

  /** How well `text` matches the lowercase query: 0 (no match) to 100. */
  function score(text: string, q: string): number {
    const t = text.toLowerCase();
    if (t.startsWith(q)) return 100;
    const words = t.split(/[\s\-_/.:]+/).filter(Boolean);
    const terms = q.split(/\s+/);
    if (terms.every((term) => words.some((w) => w.startsWith(term)))) return 80;
    // "nt" → New Terminal
    if (terms.length === 1 && words.map((w) => w[0]).join("").startsWith(q)) return 70;
    if (terms.every((term) => t.includes(term))) return 40;
    return 0;
  }

  function buildItems(raw: string): Item[] {
    if (!request) return [];
    const q = raw.trim();
    const lq = q.toLowerCase();
    const activeTab = tabs.find((t) => t.id === activeId);
    const spaceName = (id: number) => spaces.find((s) => s.id === id)?.name ?? "";

    // "> command" runs in a new terminal.
    if (q.startsWith(">")) {
      const command = q.slice(1).trim();
      return [
        {
          key: "run",
          kind: "run",
          label: command ? `Run “${command}”` : "Type a command to run in a new terminal",
          detail: "New terminal",
          section: "top",
          run: () => browser.newTerminal(command),
        },
      ];
    }

    const commands = COMMANDS.filter(
      (c) =>
        (!c.web || activeTab?.kind === "web") &&
        (!c.tab || activeTab) &&
        (!c.unless || activeTab?.zone !== c.unless),
    );
    const commandItem = (c: Command, section: Section): Item => ({
      key: `cmd-${c.id}`,
      kind: "command",
      label: c.label,
      shortcut: c.shortcut,
      section,
      run: () => browser.menuAction(c.id.startsWith("ctx-") ? `${c.id}:${activeId}` : c.id),
    });
    const tabItem = (t: Tab, section: Section): Item => ({
      key: `tab-${t.id}`,
      kind: "tab",
      label: t.title || hostOf(t.url),
      detail: t.zone === "favorite" ? "Favorite" : t.space === activeSpace ? "Switch to tab" : spaceName(t.space),
      url: t.url,
      tab: t,
      section,
      run: () => browser.activateTab(t.id),
    });
    const spaceItem = (s: Space, section: Section): Item => ({
      key: `space-${s.id}`,
      kind: "space",
      label: `Go to ${s.name}`,
      detail: s.profile ? "Space · own profile" : "Space",
      space: s,
      section,
      run: () => browser.activateSpace(s.id),
    });
    const otherTabs = tabs.filter((t) => t.id !== activeId);
    const otherSpaces = spaces.filter((s) => s.id !== activeSpace);

    // Nothing typed: what you can do, where you can go.
    if (!q) {
      return [
        ...commands.filter((c) => c.suggest).map((c) => commandItem(c, "Suggestions")),
        // This space's tabs first.
        ...[...otherTabs]
          .sort((a, b) => Number(b.space === activeSpace) - Number(a.space === activeSpace))
          .slice(0, SECTION_LIMIT)
          .map((t) => tabItem(t, "Tabs")),
        ...otherSpaces.map((s) => spaceItem(s, "Spaces")),
      ];
    }

    const ranked = <T,>(list: T[], rate: (x: T) => number) =>
      list
        .map((x) => ({ x, s: rate(x) }))
        .filter((r) => r.s > 0)
        .sort((a, b) => b.s - a.s);
    const tabHits = ranked(otherTabs, (t) =>
      Math.max(score(t.title, lq), t.url.toLowerCase().includes(lq) ? 40 : 0),
    );
    const commandHits = ranked(commands, (c) =>
      Math.max(score(c.label, lq), c.keywords && score(c.keywords, lq) >= 80 ? 50 : 0),
    );
    const spaceHits = ranked(otherSpaces, (s) => Math.max(score(s.name, lq), score("space", lq) >= 80 ? 50 : 0));

    // Top: a typed URL first; then a tab whose title matches (beats a web
    // search) and a clearly named command; then the URL/search itself.
    const top: Item[] = [];
    const parsed = parseInput(q);
    const newTab = request.mode === "newTab";
    const open = parsed && {
      key: "primary",
      kind: parsed.kind,
      label: parsed.kind === "search" ? `Search Google for “${q}”` : parsed.url,
      detail: parsed.kind === "search" ? "Google" : newTab ? "Open in new tab" : "Open",
      url: parsed.url,
      section: "top" as const,
      run: () => (newTab ? browser.newTab(parsed.url) : browser.navigate(parsed.url)),
    };
    if (open && parsed.kind === "url") top.push(open);
    const bestTab = tabHits[0]?.x && score(tabHits[0].x.title, lq) > 0 ? tabHits.shift()!.x : undefined;
    const bestCommand = commandHits[0]?.s >= 70 ? commandHits.shift()!.x : undefined;
    const bestSpace = spaceHits[0]?.s >= 80 ? spaceHits.shift()!.x : undefined;
    if (bestTab) top.push(tabItem(bestTab, "top"));
    if (bestCommand) top.push(commandItem(bestCommand, "top"));
    if (bestSpace) top.push(spaceItem(bestSpace, "top"));
    if (open && parsed.kind === "search") top.push(open);

    return [
      ...top,
      ...tabHits.slice(0, SECTION_LIMIT).map((r) => tabItem(r.x, "Tabs")),
      ...commandHits.slice(0, SECTION_LIMIT).map((r) => commandItem(r.x, "Commands")),
      ...spaceHits.map((r) => spaceItem(r.x, "Spaces")),
    ];
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
