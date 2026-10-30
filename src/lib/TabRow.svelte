<script lang="ts">
  import TabIcon from "./TabIcon.svelte";
  import { browser, hostOf, type Tab, type Zone } from "./browser";

  let {
    tab,
    active,
    dragging = false,
    mark = null,
    cursor = false,
    indent = false,
    split = null,
    beside = false,
    zone,
    folder = null,
    index,
    onpointerdown,
  }: {
    tab: Tab;
    active: boolean;
    dragging?: boolean;
    /** Drop indicator while another tab is dragged over this row. */
    mark?: "before" | "after" | null;
    /** Explore mode's keyboard cursor is on this row. */
    cursor?: boolean;
    indent?: boolean;
    /** Its side, if the tab is in a split view. */
    split?: "left" | "right" | null;
    /** On screen beside the active tab (the other pane of its split). */
    beside?: boolean;
    /** This row's group and position, for drop targeting. */
    zone: Zone;
    folder?: number | null;
    index: number;
    onpointerdown: (event: PointerEvent) => void;
  } = $props();

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      browser.activateTab(tab.id);
    }
  }
</script>

<div
  class="row tab"
  class:active
  class:dragging
  class:cursor
  class:indent
  class:beside
  class:terminal={tab.kind === "terminal"}
  class:before={mark === "before"}
  class:after={mark === "after"}
  role="button"
  tabindex="0"
  title={tab.kind === "web" ? tab.url : tab.title}
  data-drop="row"
  data-tab={tab.id}
  data-zone={zone}
  data-folder={folder ?? ""}
  data-index={index}
  {onpointerdown}
  onauxclick={(e) => e.button === 1 && browser.closeTab(tab.id)}
  oncontextmenu={(e) => {
    e.preventDefault();
    browser.tabContextMenu(tab.id);
  }}
  {onkeydown}
>
  <TabIcon {tab} />
  <span class="title">{tab.title || hostOf(tab.url)}</span>
  {#if split}
    <svg class="split-glyph" viewBox="0 0 16 16" aria-label="Split view, {split} side">
      <rect x="2" y="3.5" width="12" height="9" rx="2" />
      <path d="M8 3.5v9" />
      <rect class="side" x={split === "left" ? 3.5 : 9.5} y="5" width="3" height="6" rx="0.75" />
    </svg>
  {/if}
  <button
    class="close"
    aria-label={zone === "today" ? "Close tab" : "Unload tab"}
    onpointerdown={(e) => e.stopPropagation()}
    onclick={() => browser.closeTab(tab.id)}
  >
    {#if zone === "today"}
      <svg viewBox="0 0 16 16"><path d="m4.5 4.5 7 7m0-7-7 7" /></svg>
    {:else}
      <svg viewBox="0 0 16 16"><path d="M4.5 8h7" /></svg>
    {/if}
  </button>
</div>

<style>
  .row {
    position: relative;
    height: 34px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0 6px 0 10px;
    border-radius: 9px;
    color: var(--text);
    cursor: default;
  }
  .row:hover {
    background: var(--fill-hover);
  }
  .indent {
    padding-left: 30px;
  }
  .active {
    background: var(--tab-active);
    box-shadow: 0 1px 2px var(--shadow);
  }
  .active:hover {
    background: var(--tab-active);
  }
  /* The other pane of the split on screen. */
  .beside {
    background: color-mix(in srgb, var(--tab-active) 50%, transparent);
  }
  .terminal {
    background: var(--term-row);
  }
  .terminal:hover {
    background: var(--term-row-hover);
  }
  .terminal.beside {
    background: var(--term-row-hover);
  }
  .terminal.active {
    background: var(--term-row-active);
    box-shadow:
      inset 0 0 0 1px var(--fill-strong),
      0 1px 2px var(--shadow);
  }
  .row.cursor {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .dragging {
    opacity: 0.5;
  }
  .before::before,
  .after::after {
    content: "";
    position: absolute;
    left: 8px;
    right: 8px;
    height: 2px;
    border-radius: 1px;
    background: var(--accent);
  }
  .before::before {
    top: -2px;
  }
  .after::after {
    bottom: -2px;
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
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--text-muted);
    visibility: hidden;
  }
  .close:hover {
    background: var(--fill-hover);
    color: var(--text);
  }
  .row:hover .close,
  .active .close {
    visibility: visible;
  }
  .split-glyph {
    flex: none;
    width: 14px;
    height: 14px;
    stroke-width: 1.3;
    color: var(--text-muted);
  }
  .split-glyph .side {
    fill: currentColor;
    stroke: none;
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
</style>
