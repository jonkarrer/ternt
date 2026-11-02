<script lang="ts">
  // The same list ⌘H shows (src/lib/help.ts).
  import { SHORTCUTS } from "../lib/help";
</script>

{#each SHORTCUTS as group (group.title)}
  <h2>{group.title}</h2>
  <div class="card">
    {#each group.items as { keys, action } (keys + action)}
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
