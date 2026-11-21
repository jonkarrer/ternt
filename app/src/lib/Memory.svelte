<script lang="ts">
  import { core, type Memory } from "./core";
  import { app, openThread } from "./state.svelte";

  // What Tokidoki knows: every memory it keeps, newest first. The user can tell it something
  // outright, correct a fact, or have it forget one. The list itself follows the core's
  // memory events (state.svelte.ts), so what Tokidoki learns elsewhere shows up here too.
  let telling = $state("");
  let filter = $state("");
  let editing = $state<{ id: number; text: string } | null>(null);
  let forgetting = $state<number | null>(null);
  let error = $state<string | null>(null);

  const shown = $derived.by(() => {
    const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
    return app.memories.filter((m) => words.every((word) => m.text.toLowerCase().includes(word)));
  });

  /** Runs a change; a failure is shown, and the list is read again in case it was stale. */
  async function attempt(change: () => Promise<unknown>) {
    error = null;
    try {
      await change();
      return true;
    } catch (problem) {
      error = String(problem);
      app.memories = await core.memories();
      return false;
    }
  }

  async function tell(event: SubmitEvent) {
    event.preventDefault();
    const text = telling.trim();
    if (text && (await attempt(() => core.remember(text)))) telling = "";
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!editing) return;
    const { id, text } = editing;
    const before = app.memories.find((m) => m.id === id)?.text;
    if (text.trim() === before || (await attempt(() => core.reviseMemory(id, text)))) editing = null;
  }

  async function forget(id: number) {
    forgetting = null;
    await attempt(() => core.forgetMemory(id));
  }

  function when(ms: number) {
    const days = Math.floor((Date.now() - ms) / 86_400_000);
    if (days < 1) return "today";
    if (days < 2) return "yesterday";
    if (days < 7) return `${days} days ago`;
    return new Date(ms).toLocaleDateString(undefined, { month: "long", day: "numeric", year: "numeric" });
  }

  function from(memory: Memory) {
    return memory.source && app.threads.find((thread) => thread.id === memory.source!.thread);
  }
</script>

<section>
  <header data-tauri-drag-region></header>
  <div class="scroll">
    <div class="column">
      <h1>What Tokidoki knows</h1>
      <p class="lede">
        It keeps what matters as you talk and tidies it while idle. Correct anything that's wrong, or tell it
        something outright.
      </p>

      <form class="tell" onsubmit={tell}>
        <input bind:value={telling} placeholder="Something to remember, like “I'm vegetarian”" aria-label="Something to remember" />
        <button type="submit" disabled={!telling.trim()}>Remember</button>
      </form>
      {#if error}<p class="error selectable" role="alert">{error}</p>{/if}

      {#if app.memories.length === 0}
        <p class="empty">
          Nothing yet. Talk for a while and what's worth keeping shows up here: preferences, facts about you, how
          you like things done.
        </p>
      {:else}
        {#if app.memories.length > 8}
          <input class="filter" type="search" bind:value={filter} placeholder="Search {app.memories.length} memories" aria-label="Search memories" />
        {/if}
        <ul>
          {#each shown as memory (memory.id)}
            {@const thread = from(memory)}
            <li>
              {#if editing?.id === memory.id}
                <form class="edit" onsubmit={save}>
                  <!-- svelte-ignore a11y_autofocus -->
                  <textarea bind:value={editing.text} rows="2" autofocus aria-label="Memory"></textarea>
                  <div class="actions shown">
                    <button type="button" onclick={() => (editing = null)}>Cancel</button>
                    <button type="submit" class="primary" disabled={!editing.text.trim()}>Save</button>
                  </div>
                </form>
              {:else}
                <p class="text selectable">{memory.text}</p>
                <div class="meta">
                  <span>
                    {memory.kind === "procedure" ? "A way of doing things, " : ""}{memory.version > 1
                      ? "revised"
                      : "learned"}
                    {when(memory.at)}{#if thread}, in
                      <button class="link" onclick={() => openThread(thread.id)}>{thread.title}</button>{/if}
                  </span>
                  <div class="actions" class:shown={forgetting === memory.id}>
                    {#if forgetting === memory.id}
                      <button onclick={() => (forgetting = null)}>Keep</button>
                      <button class="danger" onclick={() => forget(memory.id)}>Forget for good</button>
                    {:else}
                      {#if memory.kind === "fact"}
                        <button onclick={() => (editing = { id: memory.id, text: memory.text })}>Edit</button>
                      {/if}
                      <button onclick={() => (forgetting = memory.id)}>Forget</button>
                    {/if}
                  </div>
                </div>
              {/if}
            </li>
          {:else}
            <li class="empty">No memory matches “{filter}”.</li>
          {/each}
        </ul>
      {/if}
    </div>
  </div>
</section>

<style>
  section {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  header {
    height: 38px;
    flex: none;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .column {
    width: min(680px, 100%);
    margin: 0 auto;
    padding: 8px 28px 40px;
  }
  h1 {
    margin: 4vh 0 8px;
    font: 500 30px/1.15 var(--voice);
    letter-spacing: -0.02em;
  }
  .lede {
    margin: 0 0 22px;
    max-width: 52ch;
    color: var(--muted);
    font-size: 15px;
  }
  input,
  textarea {
    border: 1px solid var(--line);
    border-radius: 9px;
    background: var(--raised);
    padding: 0 12px;
    font-size: 14.5px;
  }
  input {
    height: 36px;
  }
  input:focus,
  textarea:focus {
    outline: 2px solid var(--indigo);
    outline-offset: -1px;
  }
  .tell {
    display: flex;
    gap: 8px;
  }
  .tell input {
    flex: 1;
    min-width: 0;
  }
  .tell button,
  .primary {
    height: 36px;
    padding: 0 14px;
    border-radius: 9px;
    background: var(--plum);
    color: var(--paper);
    font-weight: 600;
  }
  .tell button:disabled,
  .primary:disabled {
    opacity: 0.4;
  }
  .filter {
    width: 100%;
    margin-top: 26px;
  }
  .error {
    margin: 10px 0 0;
    color: var(--danger);
    font-size: 13px;
  }
  .empty {
    margin: 28px 0 0;
    max-width: 52ch;
    color: var(--muted);
  }
  ul {
    margin: 14px 0 0;
    padding: 0;
    list-style: none;
  }
  li:not(.empty) {
    padding: 14px 0 12px 18px;
    border-bottom: 1px solid var(--line);
    position: relative;
  }
  /* The plum mote: the mark of something remembered, here and in conversations. */
  li:not(.empty)::before {
    content: "";
    position: absolute;
    left: 2px;
    top: 24px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--plum);
  }
  .text {
    margin: 0;
    font: 16.5px/1.5 var(--voice);
    overflow-wrap: anywhere;
  }
  .meta {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-top: 4px;
    min-height: 22px;
    color: var(--muted);
    font-size: 12.5px;
  }
  .link {
    color: var(--indigo);
    max-width: 28ch;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    vertical-align: bottom;
  }
  .link:hover {
    text-decoration: underline;
  }
  .actions {
    display: flex;
    gap: 4px;
    flex: none;
    opacity: 0;
  }
  li:hover .actions,
  li:focus-within .actions,
  .actions.shown {
    opacity: 1;
  }
  .actions button:not(.primary) {
    height: 22px;
    padding: 0 8px;
    border-radius: 6px;
    background: var(--side);
    color: var(--ink);
    font-size: 12px;
    font-weight: 600;
  }
  .actions .danger {
    background: var(--danger);
    color: var(--paper);
  }
  .edit textarea {
    width: 100%;
    padding: 8px 12px;
    resize: vertical;
    font: 16.5px/1.5 var(--voice);
  }
  .edit .actions {
    justify-content: flex-end;
    margin-top: 6px;
  }
  .edit .primary {
    height: 28px;
    border-radius: 7px;
  }
  .edit .actions button:not(.primary) {
    height: 28px;
    padding: 0 12px;
    border-radius: 7px;
  }
</style>
