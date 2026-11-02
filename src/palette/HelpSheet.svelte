<script lang="ts">
  // ⌘H: every shortcut and a few how-tos, filtered by what's typed in the
  // command bar's field (see src/lib/help.ts).
  import { filterHelp } from "../lib/help";

  let { query }: { query: string } = $props();
  const found = $derived(filterHelp(query));
</script>

<div class="sheet">
  {#if found.shortcuts.length}
    <section class="shortcuts">
      {#each found.shortcuts as group (group.title)}
        <div class="group">
          <h3>{group.title}</h3>
          {#each group.items as item (item.keys + item.action)}
            <div class="shortcut">
              <span>{item.action}</span>
              <kbd>{item.keys}</kbd>
            </div>
          {/each}
        </div>
      {/each}
    </section>
  {/if}
  {#if found.howTos.length}
    <section>
      <h2>How to</h2>
      <div class="how-tos">
        {#each found.howTos as howTo (howTo.title)}
          <div class="how-to">
            <h3>{howTo.title}</h3>
            <p>{howTo.text}</p>
          </div>
        {/each}
      </div>
    </section>
  {/if}
  {#if !found.shortcuts.length && !found.howTos.length}
    <p class="none">Nothing matches “{query.trim()}”.</p>
  {/if}
</div>

<style>
  .sheet {
    max-height: min(560px, 68vh);
    overflow-y: auto;
    padding: 6px 18px 18px;
    border-top: 0.5px solid var(--ring);
  }
  .shortcuts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 4px 24px;
  }
  h2 {
    margin: 18px 0 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  h3 {
    margin: 12px 0 4px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
  }
  .shortcut {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 28px;
    font-size: 13px;
  }
  .shortcut span {
    flex: 1;
  }
  kbd {
    flex: none;
    padding: 1px 6px;
    border-radius: 5px;
    background: var(--fill-strong);
    font: inherit;
    font-size: 12px;
    color: var(--text-muted);
    white-space: pre;
  }
  .how-tos {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 8px;
  }
  .how-to {
    padding: 2px 12px 10px;
    border-radius: 10px;
    background: var(--fill);
  }
  .how-to h3 {
    margin-top: 10px;
    color: var(--text);
    font-size: 13px;
  }
  .how-to p {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .none {
    margin: 18px 0 6px;
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
