<script lang="ts">
  import { onMount } from "svelte";
  import { flip } from "svelte/animate";
  import Omnibox from "./lib/Omnibox.svelte";
  import TabIcon from "./lib/TabIcon.svelte";
  import {
    browser,
    hostOf,
    DEFAULT_SIDEBAR_WIDTH,
    MAX_SIDEBAR_WIDTH,
    MIN_SIDEBAR_WIDTH,
    type BrowserState,
    type Tab,
  } from "./lib/browser";

  /** Row height + gap in the tab list; drag targets snap to this. */
  const ROW_PITCH = 36;

  let browserState = $state<BrowserState>({
    tabs: [],
    active: null,
    sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
    sidebarCollapsed: false,
  });
  let sidebarWidth = $state(DEFAULT_SIDEBAR_WIDTH);
  let resizing = $state(false);
  /** While a tab is being dragged: the optimistic order of the regular list. */
  let drag = $state<{ id: number; order: number[] } | null>(null);

  const activeTab = $derived(browserState.tabs.find((t) => t.id === browserState.active));
  const pinned = $derived(browserState.tabs.filter((t) => t.pinned));
  const regular = $derived.by(() => {
    const tabs = browserState.tabs.filter((t) => !t.pinned);
    if (!drag) return tabs;
    const byId = new Map(tabs.map((t) => [t.id, t]));
    return drag.order.map((id) => byId.get(id)).filter((t): t is Tab => t != null);
  });
  const collapsed = $derived(browserState.sidebarCollapsed);

  function applyState(s: BrowserState) {
    browserState = s;
    // Rust owns the width; don't fight an in-progress drag.
    if (!resizing) sidebarWidth = s.sidebarWidth;
  }

  onMount(() => {
    const unlisten = browser.onState(applyState);
    (async () => {
      // Subscribe before reading state, so no update can fall in between.
      await unlisten;
      applyState(await browser.getState());
      if (import.meta.env.DEV) {
        const { installTestHooks } = await import("./lib/debug");
        installTestHooks(() => browserState);
      }
    })();
    return () => unlisten.then((off) => off());
  });

  // Sidebar resize: the UI moves immediately; Rust is sent the latest width
  // with at most one call in flight, so a fast drag never queues up IPC.
  let pendingWidth: number | null = null;
  let inFlight = false;

  async function pushWidth(width: number) {
    pendingWidth = width;
    if (inFlight) return;
    inFlight = true;
    while (pendingWidth != null) {
      const next = pendingWidth;
      pendingWidth = null;
      await browser.setSidebarWidth(next);
    }
    inFlight = false;
  }

  function startResize(event: PointerEvent) {
    const handle = event.currentTarget as HTMLElement;
    const startX = event.clientX;
    const startWidth = sidebarWidth;
    handle.setPointerCapture(event.pointerId);
    resizing = true;

    const move = (e: PointerEvent) => {
      const width = Math.round(
        Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, startWidth + e.clientX - startX)),
      );
      if (width === sidebarWidth) return;
      sidebarWidth = width;
      pushWidth(width);
    };
    const end = () => {
      resizing = false;
      handle.removeEventListener("pointermove", move);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end, { once: true });
    handle.addEventListener("pointercancel", end, { once: true });
  }

  // Tab rows: click activates, drag reorders (after a few px of movement).
  function onTabPointerDown(event: PointerEvent, tab: Tab) {
    if (event.button !== 0) return;
    const row = event.currentTarget as HTMLElement;
    const startY = event.clientY;
    const ids = regular.map((t) => t.id);
    const from = ids.indexOf(tab.id);
    let dragging = false;
    try {
      row.setPointerCapture(event.pointerId);
    } catch {
      // Synthetic pointers (tests) can't be captured; moves still bubble here.
    }

    const move = (e: PointerEvent) => {
      const dy = e.clientY - startY;
      if (!dragging && Math.abs(dy) < 4) return;
      dragging = true;
      const to = Math.max(0, Math.min(ids.length - 1, from + Math.round(dy / ROW_PITCH)));
      const order = ids.filter((id) => id !== tab.id);
      order.splice(to, 0, tab.id);
      drag = { id: tab.id, order };
    };
    const up = async () => {
      row.removeEventListener("pointermove", move);
      if (!dragging || !drag) {
        drag = null;
        browser.activateTab(tab.id);
        return;
      }
      const to = drag.order.indexOf(tab.id);
      await browser.moveTab(tab.id, pinned.length + to);
      drag = null;
    };
    row.addEventListener("pointermove", move);
    row.addEventListener("pointerup", up, { once: true });
    row.addEventListener("pointercancel", () => (drag = null), { once: true });
  }

  function onTabKey(event: KeyboardEvent, id: number) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      browser.activateTab(id);
    }
  }

  function contextMenu(event: MouseEvent, id: number) {
    event.preventDefault();
    browser.tabContextMenu(id);
  }
</script>

<div class="app" class:resizing style:--sidebar="{collapsed ? 0 : sidebarWidth}px">
  {#if !collapsed}
    <aside class="sidebar">
      <header class="titlebar" data-tauri-drag-region>
        <button class="icon" aria-label="Hide sidebar (⌘S)" onclick={() => browser.menuAction("toggle-sidebar")}>
          <svg viewBox="0 0 16 16"><rect x="2" y="3" width="12" height="10" rx="2" /><path d="M6.5 3v10" /></svg>
        </button>
        <div class="nav">
          <button class="icon" aria-label="Back" onclick={browser.goBack} disabled={!activeTab?.canGoBack}>
            <svg viewBox="0 0 16 16"><path d="M10 3 5 8l5 5" /></svg>
          </button>
          <button class="icon" aria-label="Forward" onclick={browser.goForward} disabled={!activeTab?.canGoForward}>
            <svg viewBox="0 0 16 16"><path d="m6 3 5 5-5 5" /></svg>
          </button>
          <button class="icon" aria-label="Reload" onclick={browser.reload} disabled={activeTab?.kind !== "web"}>
            <svg viewBox="0 0 16 16"><path d="M13 8a5 5 0 1 1-1.5-3.5M13 2.5V5h-2.5" /></svg>
          </button>
        </div>
      </header>

      <Omnibox tab={activeTab} />

      {#if pinned.length}
        <div class="pinned">
          {#each pinned as tab (tab.id)}
            <button
              class="pin"
              class:active={tab.id === browserState.active}
              title={tab.title || hostOf(tab.url)}
              onclick={() => browser.activateTab(tab.id)}
              oncontextmenu={(e) => contextMenu(e, tab.id)}
            >
              <TabIcon {tab} />
            </button>
          {/each}
        </div>
      {/if}

      <button class="row action first" onclick={() => browser.openPalette("newTab")}>
        <svg viewBox="0 0 16 16"><path d="M8 3v10M3 8h10" /></svg>
        <span>New Tab</span>
      </button>
      <button class="row action" onclick={() => browser.newTerminal()}>
        <svg viewBox="0 0 16 16"><path d="m4 5 3 3-3 3M9 11.5h3.5" /></svg>
        <span>New Terminal</span>
      </button>

      <nav class="tabs">
        {#each regular as tab (tab.id)}
          <div
            class="row tab"
            class:active={tab.id === browserState.active}
            class:dragging={drag?.id === tab.id}
            role="button"
            tabindex="0"
            title={tab.kind === "web" ? tab.url : tab.title}
            animate:flip={{ duration: 150 }}
            onpointerdown={(e) => onTabPointerDown(e, tab)}
            onauxclick={(e) => e.button === 1 && browser.closeTab(tab.id)}
            oncontextmenu={(e) => contextMenu(e, tab.id)}
            onkeydown={(e) => onTabKey(e, tab.id)}
          >
            <TabIcon {tab} />
            <span class="title">{tab.title || hostOf(tab.url)}</span>
            <button
              class="icon close"
              aria-label="Close tab"
              onpointerdown={(e) => e.stopPropagation()}
              onclick={() => browser.closeTab(tab.id)}
            >
              <svg viewBox="0 0 16 16"><path d="m4.5 4.5 7 7m0-7-7 7" /></svg>
            </button>
          </div>
        {/each}
      </nav>
      <div class="fill" data-tauri-drag-region></div>
    </aside>

    <div
      class="resize-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize sidebar"
      onpointerdown={startResize}
    ></div>
  {/if}

  <!-- Sits exactly under the active content view: draws its shadow, and the
       empty state when no tab is open. -->
  <main class="content">
    {#if browserState.tabs.length === 0}
      <p class="empty">Press <kbd>⌘T</kbd> to open a tab</p>
    {/if}
  </main>
</div>

<style>
  .app {
    position: fixed;
    inset: 0;
    background: var(--tint);
  }
  .app.resizing {
    cursor: col-resize;
  }

  .sidebar {
    position: absolute;
    inset: 0 auto 0 0;
    width: var(--sidebar);
    padding: 0 var(--inset) var(--inset);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* 28px matches the macOS overlay title bar so buttons line up with the
     traffic lights; the left padding clears them. */
  .titlebar {
    height: 28px;
    flex: none;
    margin: 0 0 4px;
    padding-left: 70px;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .nav {
    display: flex;
    gap: 2px;
  }

  .icon {
    width: 26px;
    height: 26px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--text-muted);
  }
  .icon:hover:not(:disabled) {
    background: var(--fill-hover);
    color: var(--text);
  }
  .icon:disabled {
    opacity: 0.4;
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

  /* Arc-style favorites: a grid of icon tiles. */
  .pinned {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(48px, 1fr));
    gap: 6px;
    margin-top: 2px;
  }
  .pin {
    height: 42px;
    display: grid;
    place-items: center;
    border-radius: 10px;
    background: var(--fill);
  }
  .pin:hover {
    background: var(--fill-hover);
  }
  .pin.active {
    background: var(--tab-active);
    box-shadow: 0 1px 2px var(--shadow);
  }

  .row {
    height: 34px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0 6px 0 10px;
    border-radius: 9px;
    color: var(--text);
    text-align: left;
  }
  .row:hover {
    background: var(--fill-hover);
  }
  .action {
    color: var(--text-muted);
  }
  .action.first {
    margin-top: 4px;
  }

  .tabs {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
    min-height: 0;
  }
  .tab {
    cursor: default;
  }
  .tab.active {
    background: var(--tab-active);
    box-shadow: 0 1px 2px var(--shadow);
  }
  .tab.dragging {
    background: var(--tab-active);
    box-shadow: 0 4px 12px var(--shadow);
    z-index: 1;
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .close {
    width: 22px;
    height: 22px;
    visibility: hidden;
  }
  .tab:hover .close,
  .tab.active .close {
    visibility: visible;
  }

  .fill {
    flex: 1;
  }

  /* Lives in the sidebar's right padding: the content view covers anything
     further right. */
  .resize-handle {
    position: absolute;
    top: 0;
    bottom: 0;
    left: calc(var(--sidebar) - var(--inset));
    width: var(--inset);
    cursor: col-resize;
  }

  .content {
    position: absolute;
    top: var(--inset);
    right: var(--inset);
    bottom: var(--inset);
    left: max(var(--sidebar), var(--inset));
    border-radius: var(--content-radius);
    background: var(--surface);
    box-shadow:
      0 0 0 0.5px var(--ring),
      0 2px 12px var(--shadow);
    display: grid;
    place-items: center;
  }
  .empty {
    color: var(--text-muted);
  }
  kbd {
    font: inherit;
    padding: 1px 6px;
    border-radius: 5px;
    background: var(--fill);
  }
</style>
