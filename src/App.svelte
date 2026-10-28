<script lang="ts">
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import Omnibox from "./lib/Omnibox.svelte";
  import TabIcon from "./lib/TabIcon.svelte";
  import TabRow from "./lib/TabRow.svelte";
  import {
    browser,
    hostOf,
    DEFAULT_SIDEBAR_WIDTH,
    MAX_SIDEBAR_WIDTH,
    MIN_SIDEBAR_WIDTH,
    type BrowserState,
    type Tab,
    type Zone,
  } from "./lib/browser";
  import { spaceStyle, spaceSwatch } from "./lib/spaces";

  /** Appends when dropped on a section rather than a row. */
  const END = 1_000_000;
  /** Horizontal trackpad travel that switches spaces. */
  const SWIPE_DISTANCE = 120;

  type Mark =
    | { kind: "row"; tab: number; where: "before" | "after" }
    | { kind: "folder"; id: number }
    | { kind: "zone"; zone: Zone };
  type DropTarget = { zone: Zone; folder: number | null; index: number; mark: Mark };

  let browserState = $state<BrowserState>({
    tabs: [],
    spaces: [],
    folders: [],
    active: null,
    activeSpace: 0,
    sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
    sidebarCollapsed: false,
  });
  let sidebarWidth = $state(DEFAULT_SIDEBAR_WIDTH);
  let resizing = $state(false);
  let drag = $state<{ id: number; target: DropTarget | null } | null>(null);
  let renaming = $state<{ kind: "space" | "folder"; id: number } | null>(null);
  /** Direction of the last space switch, for the slide-in. */
  let slide = $state(1);

  const activeSpace = $derived(browserState.activeSpace);
  const space = $derived(browserState.spaces.find((s) => s.id === activeSpace));
  const activeTab = $derived(browserState.tabs.find((t) => t.id === browserState.active));
  const favorites = $derived(browserState.tabs.filter((t) => t.zone === "favorite"));
  const folders = $derived(browserState.folders.filter((f) => f.space === activeSpace));
  const loosePinned = $derived(pinnedIn(null));
  const today = $derived(browserState.tabs.filter((t) => t.zone === "today" && t.space === activeSpace));
  const collapsed = $derived(browserState.sidebarCollapsed);

  function pinnedIn(folder: number | null) {
    return browserState.tabs.filter((t) => t.zone === "pinned" && t.space === activeSpace && t.folder === folder);
  }

  function applyState(s: BrowserState) {
    const from = browserState.spaces.findIndex((x) => x.id === browserState.activeSpace);
    const to = s.spaces.findIndex((x) => x.id === s.activeSpace);
    if (from !== to && from >= 0) slide = to > from ? 1 : -1;
    browserState = s;
    // Rust owns the width; don't fight an in-progress drag.
    if (!resizing) sidebarWidth = s.sidebarWidth;
  }

  onMount(() => {
    const unlisten = [browser.onState(applyState), browser.onRename((target) => (renaming = target))];
    (async () => {
      // Subscribe before reading state, so no update can fall in between.
      await Promise.all(unlisten);
      applyState(await browser.getState());
      if (import.meta.env.DEV) {
        const { installTestHooks } = await import("./lib/debug");
        installTestHooks(() => browserState);
      }
    })();
    return () => unlisten.forEach((p) => p.then((off) => off()));
  });

  // --- Sidebar resize ------------------------------------------------------
  // The UI moves immediately; Rust is sent the latest width with at most one
  // call in flight, so a fast drag never queues up IPC.
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

  // --- Drag and drop -------------------------------------------------------
  // Click activates; dragging (after a few px) shows a drop marker and moves
  // the tab on release: between rows, into a folder, or into a section.

  function sameGroup(a: Tab, zone: Zone, folder: number | null) {
    return a.zone === zone && a.folder === folder && (zone === "favorite" || a.space === activeSpace);
  }

  function hitTest(x: number, y: number, tab: Tab): DropTarget | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop]");
    if (!el) return null;
    const kind = el.dataset.drop;
    let target: DropTarget | null = null;

    if (kind === "row") {
      const id = Number(el.dataset.tab);
      if (id === tab.id) return null;
      const zone = el.dataset.zone as Zone;
      const folder = el.dataset.folder ? Number(el.dataset.folder) : null;
      const rect = el.getBoundingClientRect();
      // Favorites are a grid: before/after by x. Rows: by y.
      const after = zone === "favorite" ? x > rect.left + rect.width / 2 : y > rect.top + rect.height / 2;
      let index = Number(el.dataset.index) + (after ? 1 : 0);
      // Indexes count the group *without* the dragged tab.
      if (sameGroup(tab, zone, folder)) {
        const from = browserState.tabs.filter((t) => sameGroup(t, zone, folder)).indexOf(tab);
        if (from < index) index -= 1;
      }
      target = { zone, folder, index, mark: { kind: "row", tab: id, where: after ? "after" : "before" } };
    } else if (kind === "folder") {
      const id = Number(el.dataset.folder);
      target = { zone: "pinned", folder: id, index: END, mark: { kind: "folder", id } };
    } else if (kind === "zone") {
      const zone = el.dataset.zone as Zone;
      target = { zone, folder: null, index: END, mark: { kind: "zone", zone } };
    }
    // Terminals only live in Today.
    if (target && tab.kind === "terminal" && target.zone !== "today") return null;
    return target;
  }

  function startDrag(event: PointerEvent, tab: Tab) {
    if (event.button !== 0) return;
    const el = event.currentTarget as HTMLElement;
    const startX = event.clientX;
    const startY = event.clientY;
    let dragging = false;
    try {
      el.setPointerCapture(event.pointerId);
    } catch {
      // Synthetic pointers (tests) can't be captured; moves still bubble here.
    }

    const move = (e: PointerEvent) => {
      if (!dragging && Math.hypot(e.clientX - startX, e.clientY - startY) < 4) return;
      dragging = true;
      drag = { id: tab.id, target: hitTest(e.clientX, e.clientY, tab) };
    };
    const up = async () => {
      el.removeEventListener("pointermove", move);
      const target = drag?.target;
      drag = null;
      if (!dragging) {
        browser.activateTab(tab.id);
      } else if (target) {
        await browser.moveTab(tab.id, target.zone, target.folder, target.index);
      }
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up, { once: true });
    el.addEventListener("pointercancel", () => (drag = null), { once: true });
  }

  function markFor(tab: Tab): "before" | "after" | null {
    const mark = drag?.target?.mark;
    return mark?.kind === "row" && mark.tab === tab.id ? mark.where : null;
  }

  function isDropInto(kind: "folder" | "zone", key: number | Zone) {
    const mark = drag?.target?.mark;
    if (!mark) return false;
    return kind === "folder" ? mark.kind === "folder" && mark.id === key : mark.kind === "zone" && mark.zone === key;
  }

  // --- Spaces --------------------------------------------------------------

  // Two-finger horizontal swipe over the sidebar switches spaces.
  let swipe = 0;
  let swipeLocked = false;
  let swipeTimer: ReturnType<typeof setTimeout> | undefined;

  function onwheel(event: WheelEvent) {
    if (Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
    clearTimeout(swipeTimer);
    // One switch per gesture: unlock once the wheel events stop.
    swipeTimer = setTimeout(() => {
      swipe = 0;
      swipeLocked = false;
    }, 200);
    if (swipeLocked) return;
    swipe += event.deltaX;
    if (Math.abs(swipe) > SWIPE_DISTANCE) {
      browser.menuAction(swipe > 0 ? "next-space" : "previous-space");
      swipeLocked = true;
    }
  }

  function focusSelect(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  function commitRename(event: Event) {
    const name = (event.currentTarget as HTMLInputElement).value;
    const target = renaming;
    renaming = null;
    if (!target) return;
    if (target.kind === "space") browser.renameSpace(target.id, name);
    else browser.renameFolder(target.id, name);
  }

  function renameKeys(event: KeyboardEvent) {
    if (event.key === "Enter") commitRename(event);
    if (event.key === "Escape") renaming = null;
  }
</script>

<div
  class="app"
  class:resizing
  class:dragging={drag != null}
  style:--sidebar="{collapsed ? 0 : sidebarWidth}px"
  style={spaceStyle(space?.color ?? "blue")}
>
  {#if !collapsed}
    <aside class="sidebar" {onwheel}>
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

      <!-- Favorites: shared by every space. While dragging, an empty
           favorites area appears as a drop target. -->
      {#if favorites.length || drag}
        <div class="favorites" class:drop-into={isDropInto("zone", "favorite")} data-drop="zone" data-zone="favorite">
          {#each favorites as tab, i (tab.id)}
            <button
              class="fav"
              class:active={tab.id === browserState.active}
              class:dragging={drag?.id === tab.id}
              class:before={markFor(tab) === "before"}
              class:after={markFor(tab) === "after"}
              title={tab.title || hostOf(tab.url)}
              data-drop="row"
              data-tab={tab.id}
              data-zone="favorite"
              data-folder=""
              data-index={i}
              onpointerdown={(e) => startDrag(e, tab)}
              oncontextmenu={(e) => {
                e.preventDefault();
                browser.tabContextMenu(tab.id);
              }}
            >
              <TabIcon {tab} />
            </button>
          {:else}
            <span class="hint">Drop to add to Favorites</span>
          {/each}
        </div>
      {/if}

      {#key activeSpace}
        <div class="space-body" in:fly={{ x: 24 * slide, duration: 160 }}>
          <div class="space-header">
            {#if renaming?.kind === "space" && renaming.id === space?.id}
              <input class="rename" value={space?.name} use:focusSelect onblur={commitRename} onkeydown={renameKeys} />
            {:else}
              <span
                class="space-name"
                role="heading"
                aria-level="2"
                ondblclick={() => space && (renaming = { kind: "space", id: space.id })}
              >
                {space?.name}
              </span>
              {#if space?.profile}
                <span class="profile" title="This space has its own cookies and logins">Profile</span>
              {/if}
            {/if}
            <button class="icon small" aria-label="New folder" title="New folder" onclick={() => browser.menuAction("new-folder")}>
              <svg viewBox="0 0 16 16"><path d="M2.5 4.5h4l1.5 1.5h5.5v6.5h-11zM8 8v3M6.5 9.5h3" /></svg>
            </button>
          </div>

          <!-- Pinned: this space's folders, then loose pinned tabs. -->
          <div class="pinned-list" class:drop-into={isDropInto("zone", "pinned")} data-drop="zone" data-zone="pinned">
            {#each folders as folder (folder.id)}
              <div
                class="row folder"
                class:drop-into={isDropInto("folder", folder.id)}
                role="button"
                tabindex="0"
                data-drop="folder"
                data-folder={folder.id}
                onclick={(e) => !(e.target as Element).closest(".rename") && browser.setFolderOpen(folder.id, !folder.open)}
                onkeydown={(e) => e.target === e.currentTarget && e.key === "Enter" && browser.setFolderOpen(folder.id, !folder.open)}
                oncontextmenu={(e) => {
                  e.preventDefault();
                  browser.folderContextMenu(folder.id);
                }}
              >
                <svg class="chevron" class:open={folder.open} viewBox="0 0 16 16"><path d="m6 4 4 4-4 4" /></svg>
                <svg viewBox="0 0 16 16"><path d="M2.5 4.5h4l1.5 1.5h5.5v6.5h-11z" /></svg>
                {#if renaming?.kind === "folder" && renaming.id === folder.id}
                  <input class="rename" value={folder.name} use:focusSelect onblur={commitRename} onkeydown={renameKeys} />
                {:else}
                  <span class="title" role="none" ondblclick={() => (renaming = { kind: "folder", id: folder.id })}>
                    {folder.name}
                  </span>
                {/if}
              </div>
              {#if folder.open}
                {#each pinnedIn(folder.id) as tab, i (tab.id)}
                  <TabRow
                    {tab}
                    indent
                    zone="pinned"
                    folder={folder.id}
                    index={i}
                    active={tab.id === browserState.active}
                    dragging={drag?.id === tab.id}
                    mark={markFor(tab)}
                    onpointerdown={(e) => startDrag(e, tab)}
                  />
                {/each}
              {/if}
            {/each}
            {#each loosePinned as tab, i (tab.id)}
              <TabRow
                {tab}
                zone="pinned"
                index={i}
                active={tab.id === browserState.active}
                dragging={drag?.id === tab.id}
                mark={markFor(tab)}
                onpointerdown={(e) => startDrag(e, tab)}
              />
            {/each}
            {#if drag && !folders.length && !loosePinned.length}
              <span class="hint">Drop to pin to {space?.name}</span>
            {/if}
          </div>

          <!-- Arc's separator between Pinned and Today, with "Clear" on hover. -->
          <div class="divider">
            <span class="line"></span>
            {#if today.some((t) => t.kind === "web")}
              <button class="clear" title="Close this space's Today tabs" onclick={() => browser.menuAction("clear-today")}>
                <svg viewBox="0 0 16 16"><path d="M8 3.5v8m-3.5-3.5L8 11.5 11.5 8" /></svg>
                Clear
              </button>
            {/if}
          </div>

          <button class="row action" onclick={() => browser.openPalette("newTab")}>
            <svg viewBox="0 0 16 16"><path d="M8 3v10M3 8h10" /></svg>
            <span>New Tab</span>
          </button>
          <button class="row action" onclick={() => browser.newTerminal()}>
            <svg viewBox="0 0 16 16"><path d="m4 5 3 3-3 3M9 11.5h3.5" /></svg>
            <span>New Terminal</span>
          </button>

          <nav class="tabs" class:drop-into={isDropInto("zone", "today")} data-drop="zone" data-zone="today">
            {#each today as tab, i (tab.id)}
              <TabRow
                {tab}
                zone="today"
                index={i}
                active={tab.id === browserState.active}
                dragging={drag?.id === tab.id}
                mark={markFor(tab)}
                onpointerdown={(e) => startDrag(e, tab)}
              />
            {/each}
          </nav>
        </div>
      {/key}

      <footer class="spaces">
        {#each browserState.spaces as s (s.id)}
          <button
            class="space-dot"
            class:active={s.id === activeSpace}
            style:--swatch={spaceSwatch(s.color)}
            title="{s.name}{s.profile ? ' (separate profile)' : ''}"
            aria-label="Switch to {s.name}"
            onclick={() => browser.activateSpace(s.id)}
            oncontextmenu={(e) => {
              e.preventDefault();
              browser.spaceContextMenu(s.id);
            }}
          >
            {#if s.profile}<span class="ring"></span>{/if}
          </button>
        {/each}
        <button class="icon small" aria-label="New space" title="New space" onclick={() => browser.createSpace()}>
          <svg viewBox="0 0 16 16"><path d="M8 3v10M3 8h10" /></svg>
        </button>
      </footer>
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
       empty state when nothing is open. -->
  <main class="content">
    {#if browserState.active == null}
      <p class="empty">Press <kbd>⌘T</kbd> to open a tab</p>
    {/if}
  </main>
</div>

<style>
  .app {
    position: fixed;
    inset: 0;
    background: linear-gradient(
      165deg,
      hsl(var(--hue) var(--sat) var(--tint-l1) / var(--tint-a1)),
      hsl(calc(var(--hue) + 25) var(--sat) var(--tint-l2) / var(--tint-a2))
    );
    transition: background 200ms ease;
  }
  .app.resizing {
    cursor: col-resize;
  }
  .app.dragging {
    cursor: grabbing;
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
  .icon.small {
    width: 22px;
    height: 22px;
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
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* Drop targets highlight while a tab is dragged over them. */
  .drop-into {
    background: var(--fill-hover);
    box-shadow: inset 0 0 0 1.5px var(--accent);
    border-radius: 10px;
  }
  .hint {
    padding: 8px 10px;
    color: var(--text-muted);
    font-size: 12px;
  }

  /* Favorites: a grid of icon tiles, shared by all spaces. */
  .favorites {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(48px, 1fr));
    gap: 6px;
    margin-top: 2px;
    min-height: 42px;
  }
  .favorites .hint {
    grid-column: 1 / -1;
    text-align: center;
  }
  .fav {
    position: relative;
    height: 42px;
    display: grid;
    place-items: center;
    border-radius: 10px;
    background: var(--fill);
  }
  .fav:hover {
    background: var(--fill-hover);
  }
  .fav.active {
    background: var(--tab-active);
    box-shadow: 0 1px 2px var(--shadow);
  }
  .fav.dragging {
    opacity: 0.5;
  }
  .fav.before::before,
  .fav.after::after {
    content: "";
    position: absolute;
    top: 6px;
    bottom: 6px;
    width: 2px;
    border-radius: 1px;
    background: var(--accent);
  }
  .fav.before::before {
    left: -4px;
  }
  .fav.after::after {
    right: -4px;
  }

  .space-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .space-header {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 4px 0 10px;
    margin-top: 6px;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
  }
  .space-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .profile {
    padding: 1px 6px;
    border-radius: 5px;
    background: var(--fill-strong);
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .space-header .icon {
    visibility: hidden;
  }
  .space-header:hover .icon {
    visibility: visible;
  }
  .rename {
    flex: 1;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    border: 0;
    border-radius: 6px;
    outline: 0;
    background: var(--surface);
    box-shadow: 0 0 0 1.5px var(--accent);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    font-weight: 400;
  }

  .pinned-list,
  .tabs {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .pinned-list {
    flex: none;
  }
  .tabs {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
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
  .folder {
    gap: 6px;
    padding-left: 4px;
  }
  .folder .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .chevron {
    width: 12px;
    height: 12px;
    color: var(--text-muted);
    transition: rotate 120ms ease;
  }
  .chevron.open {
    rotate: 90deg;
  }

  .divider {
    height: 20px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 4px 10px;
  }
  .divider .line {
    flex: 1;
    height: 1px;
    background: var(--separator);
  }
  .divider .clear {
    display: flex;
    align-items: center;
    gap: 3px;
    padding: 2px 4px;
    border-radius: 5px;
    font-size: 11px;
    color: var(--text-muted);
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .divider .clear svg {
    width: 11px;
    height: 11px;
  }
  .divider:hover .clear,
  .divider .clear:focus-visible {
    opacity: 1;
  }
  .divider .clear:hover {
    background: var(--fill-hover);
    color: var(--text);
  }

  .spaces {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding-top: 6px;
  }
  .space-dot {
    position: relative;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--swatch);
    opacity: 0.5;
    transition:
      opacity 120ms ease,
      scale 120ms ease;
  }
  .space-dot:hover {
    opacity: 0.8;
  }
  .space-dot.active {
    opacity: 1;
    scale: 1.3;
  }
  /* A ring marks spaces with their own profile. */
  .ring {
    position: absolute;
    inset: -3px;
    border-radius: 50%;
    border: 1.5px solid var(--swatch);
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
