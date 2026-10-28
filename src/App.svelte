<script lang="ts">
  import { onMount } from "svelte";
  import Favicon from "./lib/Favicon.svelte";
  import Omnibox from "./lib/Omnibox.svelte";
  import {
    browser,
    hostOf,
    DEFAULT_SIDEBAR_WIDTH,
    MAX_SIDEBAR_WIDTH,
    MIN_SIDEBAR_WIDTH,
    type BrowserState,
  } from "./lib/browser";

  const WIDTH_KEY = "moove.sidebarWidth";

  let browserState = $state<BrowserState>({ tabs: [], active: null });
  let sidebarWidth = $state(DEFAULT_SIDEBAR_WIDTH);
  let resizing = $state(false);
  let omnibox = $state<Omnibox>();

  const activeTab = $derived(browserState.tabs.find((t) => t.id === browserState.active));

  onMount(() => {
    const unlisten = [
      browser.onState((s) => (browserState = s)),
      browser.onOmnibox((newTab) => omnibox?.begin(newTab)),
    ];
    (async () => {
      browserState = await browser.getState();
      sidebarWidth = await browser.setSidebarWidth(savedWidth());
      if (browserState.tabs.length === 0) omnibox?.begin(true);
    })();
    return () => unlisten.forEach((p) => p.then((off) => off()));
  });

  function savedWidth(): number {
    try {
      return Number(localStorage.getItem(WIDTH_KEY)) || DEFAULT_SIDEBAR_WIDTH;
    } catch {
      return DEFAULT_SIDEBAR_WIDTH;
    }
  }

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
      try {
        localStorage.setItem(WIDTH_KEY, String(sidebarWidth));
      } catch {}
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end, { once: true });
    handle.addEventListener("pointercancel", end, { once: true });
  }

  function onTabKey(event: KeyboardEvent, id: number) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      browser.activateTab(id);
    }
  }
</script>

<div class="app" class:resizing style:--sidebar="{sidebarWidth}px">
  <aside class="sidebar">
    <header class="titlebar" data-tauri-drag-region>
      <div class="nav">
        <button class="icon" aria-label="Back" onclick={browser.goBack} disabled={!activeTab}>
          <svg viewBox="0 0 16 16"><path d="M10 3 5 8l5 5" /></svg>
        </button>
        <button class="icon" aria-label="Forward" onclick={browser.goForward} disabled={!activeTab}>
          <svg viewBox="0 0 16 16"><path d="m6 3 5 5-5 5" /></svg>
        </button>
        <button class="icon" aria-label="Reload" onclick={browser.reload} disabled={!activeTab}>
          <svg viewBox="0 0 16 16"><path d="M13 8a5 5 0 1 1-1.5-3.5M13 2.5V5h-2.5" /></svg>
        </button>
      </div>
    </header>

    <Omnibox bind:this={omnibox} tab={activeTab} />

    <button class="row new-tab" onclick={() => omnibox?.begin(true)}>
      <svg viewBox="0 0 16 16"><path d="M8 3v10M3 8h10" /></svg>
      <span>New Tab</span>
    </button>

    <nav class="tabs">
      {#each browserState.tabs as tab (tab.id)}
        <div
          class="row tab"
          class:active={tab.id === browserState.active}
          role="button"
          tabindex="0"
          title={tab.url}
          onclick={() => browser.activateTab(tab.id)}
          onauxclick={(e) => e.button === 1 && browser.closeTab(tab.id)}
          onkeydown={(e) => onTabKey(e, tab.id)}
        >
          <Favicon url={tab.url} loading={tab.loading} />
          <span class="title">{tab.title || hostOf(tab.url)}</span>
          <button
            class="icon close"
            aria-label="Close tab"
            onclick={(e) => {
              e.stopPropagation();
              browser.closeTab(tab.id);
            }}
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

  <!-- Sits exactly under the active page webview: draws its shadow, and the
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
    justify-content: flex-end;
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
  .new-tab {
    color: var(--text-muted);
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

  /* Lives in the sidebar's right padding: the page webview covers anything
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
