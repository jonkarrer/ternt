<script lang="ts">
  // Mirrors the menu accelerators (src-tauri/src/menu.rs) and the README.
  const GROUPS: { title: string; items: [keys: string, action: string][] }[] = [
    {
      title: "Tabs",
      items: [
        ["⌘T", "Command bar: open a URL, search, switch tabs or spaces, run commands"],
        ["⌥⌘T", "New terminal tab"],
        ["⌘L", "Edit the current URL"],
        ["⌘W", "Close tab (unloads favorites and pinned tabs)"],
        ["⇧⌘T", "Reopen closed tab"],
        ["⌃Tab  ⌃⇧Tab", "Next / previous tab"],
        ["⌘1 – ⌘9", "Select tab (⌘9: the last one)"],
      ],
    },
    {
      title: "Pages",
      items: [
        ["⌘[  ⌘]", "Back / forward"],
        ["⌘R", "Reload"],
      ],
    },
    {
      title: "Sidebar and spaces",
      items: [
        ["⌘S", "Toggle sidebar"],
        ["⌘F", "Explore tabs: j/k move, h/l switch space, Space peeks, Enter opens"],
        ["⌃1 – ⌃9", "Select space"],
        ["⌥⌘←  ⌥⌘→", "Previous / next space (or swipe across the sidebar)"],
      ],
    },
    {
      title: "App",
      items: [
        ["⌘,", "Settings"],
        ["⌘H  ⌥⌘H", "Hide Ternt / hide others"],
        ["⌘Q", "Quit"],
      ],
    },
  ];
</script>

{#each GROUPS as group (group.title)}
  <h2>{group.title}</h2>
  <div class="card">
    {#each group.items as [keys, action] (keys)}
      <div class="row">
        <span class="label">{action}</span>
        <kbd>{keys}</kbd>
      </div>
    {/each}
  </div>
{/each}

<style>
  .row {
    min-height: 36px;
  }
  kbd {
    flex: none;
    padding: 2px 7px;
    border-radius: 5px;
    background: var(--fill);
    box-shadow: inset 0 0 0 0.5px var(--ring);
    font: inherit;
    font-size: 12px;
    color: var(--text-muted);
    white-space: pre;
  }
</style>
