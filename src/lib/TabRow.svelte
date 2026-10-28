<script lang="ts">
  import TabIcon from "./TabIcon.svelte";
  import { browser, hostOf, type Tab, type Zone } from "./browser";

  let {
    tab,
    active,
    dragging = false,
    mark = null,
    indent = false,
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
    indent?: boolean;
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
  class:indent
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
