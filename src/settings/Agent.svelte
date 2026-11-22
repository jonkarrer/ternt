<script lang="ts">
  import { onMount } from "svelte";
  import { browser, type AgentKeys, type AgentStatus, type Prefs, type Space } from "../lib/browser";
  import type { Memory } from "../agent/protocol";

  let { spaces, activeSpace }: { spaces: Space[]; activeSpace: number | null } = $props();

  // The agent in the ⌘J panel: who answers, and your key for it. Bring your own
  // key: it lives in the Keychain and goes to the agent process, nowhere else.
  type Provider = Prefs["agentProvider"];
  const PROVIDERS: { id: Provider; name: string; keyHint: string; where: string }[] = [
    { id: "fireworks", name: "Fireworks AI", keyHint: "fw_…", where: "fireworks.ai → API keys" },
    { id: "anthropic", name: "Anthropic", keyHint: "sk-ant-…", where: "console.anthropic.com → API keys" },
  ];

  let provider = $state<Provider>("fireworks");
  let keys = $state<AgentKeys | null>(null);
  let status = $state<AgentStatus | null>(null);
  let draft = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);

  const chosen = $derived(PROVIDERS.find((p) => p.id === provider) ?? PROVIDERS[0]);
  /** Where the chosen provider's key is: the Keychain, Ternt's environment, or nowhere. */
  const source = $derived(keys?.[provider] ?? "none");
  const saved = $derived(source !== "none");

  onMount(() => {
    browser.getPrefs().then((p) => (provider = p.agentProvider));
    browser.agentKeys().then((k) => (keys = k));
    browser.agentStatus().then((s) => (status = s));
    const statuses = browser.onAgentStatus((s) => {
      // The process came (back) up: what it remembers may differ from what is listed.
      if (s.state === "running" && status?.state !== "running") reload();
      status = s;
    });
    const prefs = browser.onPrefsChanged((p) => (provider = p.agentProvider));
    // Coming back from a conversation: the agent may have learned something.
    window.addEventListener("focus", reload);
    return () => {
      statuses.then((off) => off());
      prefs.then((off) => off());
      window.removeEventListener("focus", reload);
    };
  });

  // --- Memory ------------------------------------------------------------------
  // What one space's agent remembers: its facts, to correct or forget one by one, or
  // wipe all at once. Procedures and episodes are kept too but not listed.

  /** The space whose memories are shown. */
  let memorySpace = $state<number | null>(null);
  /** `null` while loading. */
  let memories = $state<Memory[] | null>(null);
  let memoryError = $state<string | null>(null);
  let editing = $state<number | null>(null);
  let editDraft = $state("");
  let showAll = $state(false);
  let confirmWipe = $state(false);
  /** How many facts are listed before "Show all". */
  const SOME = 25;

  const facts = $derived((memories ?? []).filter((m) => m.kind === "fact"));
  const shown = $derived(showAll ? facts : facts.slice(0, SOME));
  const memorySpaceName = $derived(spaces.find((s) => s.id === memorySpace)?.name ?? "");

  $effect(() => {
    // Follow the active space until the user picks one; survive a space being deleted.
    if (memorySpace === null || !spaces.some((s) => s.id === memorySpace)) {
      memorySpace = spaces.some((s) => s.id === activeSpace) ? activeSpace : (spaces[0]?.id ?? null);
    }
  });

  let loading = 0;
  async function load(space: number) {
    const turn = ++loading;
    memoryError = null;
    try {
      const list = await browser.agentMemories(space);
      if (turn === loading) memories = list;
    } catch (e) {
      if (turn === loading) {
        memories = memories ?? [];
        memoryError = String(e);
      }
    }
  }

  function reload() {
    if (memorySpace !== null) load(memorySpace);
  }

  $effect(() => {
    const space = memorySpace;
    memories = null;
    editing = null;
    showAll = false;
    confirmWipe = false;
    if (space !== null) load(space);
  });

  function pickSpace(event: Event) {
    memorySpace = Number((event.currentTarget as HTMLSelectElement).value);
  }

  function edit(memory: Memory) {
    editing = memory.id;
    editDraft = memory.text;
    confirmWipe = false;
  }

  async function saveEdit(event: Event) {
    event.preventDefault();
    const id = editing;
    const text = editDraft.trim();
    if (id === null || memorySpace === null || !text) return;
    const space = memorySpace;
    memoryError = null;
    try {
      const revised = await browser.agentRevise(space, id, text);
      memories = (memories ?? []).map((m) => (m.id === id ? revised : m));
      editing = null;
    } catch (e) {
      memoryError = String(e);
      load(space);
    }
  }

  function editKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      editing = null;
      event.preventDefault();
    }
  }

  async function forget(memory: Memory) {
    if (memorySpace === null) return;
    const space = memorySpace;
    memoryError = null;
    try {
      await browser.agentForget(space, memory.id);
      memories = (memories ?? []).filter((m) => m.id !== memory.id);
    } catch (e) {
      memoryError = String(e);
      load(space);
    }
  }

  async function wipe() {
    if (memorySpace === null) return;
    const space = memorySpace;
    confirmWipe = false;
    memoryError = null;
    try {
      await browser.agentWipe(space);
      memories = [];
      editing = null;
    } catch (e) {
      memoryError = String(e);
      load(space);
    }
  }

  function when(memory: Memory): string {
    const date = new Date(memory.at).toLocaleDateString(undefined, { month: "short", day: "numeric" });
    return memory.version > 1 ? `Edited ${date}` : `Learned ${date}`;
  }

  const statusText = $derived.by(() => {
    if (!status) return "";
    switch (status.state) {
      case "starting":
        return "Starting the agent…";
      case "restarting":
        return status.asked ? "Restarting the agent…" : "The agent stopped; starting it again…";
      case "down":
        return status.reason;
      default:
        if (status.backend === "stub") return "A stand-in agent is running (this build has no tokidoki); no key is needed.";
        if (!status.hasKey) return `Running, waiting for a ${chosen.name} key.`;
        const model = status.model?.replace(/^accounts\/fireworks\/models\//, "");
        return model ? `Running with your ${chosen.name} key, answering with ${model}.` : `Running with your ${chosen.name} key.`;
    }
  });

  async function setProvider(event: Event) {
    const next = (event.currentTarget as HTMLSelectElement).value as Provider;
    if (next === provider) return;
    provider = next;
    draft = "";
    error = null;
    // Changing the provider restarts the agent with that provider's key.
    await browser.updatePrefs({ agentProvider: next });
  }

  async function change(update: Partial<Record<keyof AgentKeys, string>>) {
    saving = true;
    error = null;
    try {
      await browser.agentSetKeys(update);
      keys = await browser.agentKeys();
      draft = "";
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }

  function saveKey(event: Event) {
    event.preventDefault();
    const key = draft.trim();
    if (key) change({ [provider]: key });
  }
</script>

<h2>Agent</h2>
<div class="card">
  <div class="row">
    <div class="label">
      Tokidoki, your space's agent
      <div class="hint">
        Press ⌘J to talk to it. Each space has its own agent with its own memory. Ternt runs it as a separate process;
        your messages go from it to the provider below and nowhere else.
      </div>
      {#if status}<div class="hint status" data-state={status.state}>{statusText}</div>{/if}
    </div>
    {#if status?.state === "down"}
      <button class="button" onclick={() => browser.agentRestart()}>Restart</button>
    {/if}
  </div>
  <div class="row">
    <div class="label">
      Provider
      <div class="hint">Who answers. Bring your own key: Ternt has no account of its own.</div>
    </div>
    <select class="field provider" aria-label="Provider" value={provider} onchange={setProvider}>
      {#each PROVIDERS as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
  </div>
  <form class="row" onsubmit={saveKey}>
    <div class="label">
      {chosen.name} API key
      <div class="hint">
        {#if source === "environment"}
          From Ternt's own environment (development); Settings can't change it.
        {:else if saved}
          Saved in your Keychain.
        {:else}
          Not set. Make one at {chosen.where}; it is kept in your Keychain, never in a file.
        {/if}
      </div>
    </div>
    <input
      class="field key"
      type="password"
      bind:value={draft}
      placeholder={saved ? "Replace…" : chosen.keyHint}
      aria-label="{chosen.name} API key"
      autocomplete="off"
      spellcheck="false"
    />
    <button class="button primary" type="submit" disabled={saving || !draft.trim()}>{saving ? "Saving…" : "Save"}</button>
    {#if source === "keychain"}
      <button class="button" type="button" disabled={saving} onclick={() => change({ [provider]: "" })}>Remove</button>
    {/if}
  </form>
  {#if error}
    <div class="row"><span class="error" role="alert">{error}</span></div>
  {/if}
</div>

<h2>Memory</h2>
<div class="card">
  <div class="row">
    <div class="label">
      What the agent remembers
      <div class="hint">
        Each space's agent keeps facts it learns from your conversations there and brings them into later ones. Edit a
        fact to correct it, or forget it. Wipe forgets everything the agent has learned in a space; the conversations
        stay.
      </div>
    </div>
    <select class="field space" aria-label="Space" value={memorySpace} onchange={pickSpace}>
      {#each spaces as s (s.id)}
        <option value={s.id}>{s.name}</option>
      {/each}
    </select>
  </div>
  {#if memoryError}
    <div class="row"><span class="error" role="alert">{memoryError}</span></div>
  {/if}
  {#if memories === null}
    <div class="row"><div class="label hint">Loading…</div></div>
  {:else if facts.length === 0}
    <div class="row"><div class="label hint empty">Nothing remembered yet.</div></div>
  {:else}
    {#each shown as memory (memory.id)}
      <div class="row memory">
        {#if editing === memory.id}
          <form class="edit" onsubmit={saveEdit}>
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="field memory-text"
              bind:value={editDraft}
              onkeydown={editKeydown}
              aria-label="Memory"
              autofocus
              spellcheck="false"
            />
            <button class="button primary" type="submit" disabled={!editDraft.trim()}>Save</button>
            <button class="button" type="button" onclick={() => (editing = null)}>Cancel</button>
          </form>
        {:else}
          <div class="label">
            <span class="text">{memory.text}</span>
            <div class="hint">{when(memory)}</div>
          </div>
          <button class="button edit" onclick={() => edit(memory)}>Edit</button>
          <button class="button danger forget" onclick={() => forget(memory)}>Forget</button>
        {/if}
      </div>
    {/each}
    {#if facts.length > shown.length}
      <div class="row">
        <button class="button more" onclick={() => (showAll = true)}>Show all {facts.length}</button>
      </div>
    {/if}
  {/if}
  <div class="row">
    <div class="label hint count">
      {#if confirmWipe}
        Forget everything the {memorySpaceName} agent has learned? There is no undo.
      {:else}
        {facts.length === 1 ? "1 fact" : `${facts.length} facts`}
      {/if}
    </div>
    {#if confirmWipe}
      <button class="button danger wipe-confirm" onclick={wipe}>Wipe</button>
      <button class="button" onclick={() => (confirmWipe = false)}>Cancel</button>
    {:else}
      <button class="button danger wipe" disabled={!memories?.length} onclick={() => (confirmWipe = true)}>Wipe…</button>
    {/if}
  </div>
</div>

<style>
  .space {
    flex: none;
    max-width: 160px;
  }
  .memory .text {
    display: block;
    overflow-wrap: anywhere;
  }
  .memory .edit {
    display: flex;
    flex: 1;
    gap: 8px;
    min-width: 0;
  }
  .memory-text {
    flex: 1;
    min-width: 0;
  }
  .count {
    font-size: 12px;
  }
  .provider {
    flex: none;
    width: 160px;
  }
  .key {
    flex: 1;
    min-width: 0;
    max-width: 260px;
  }
  .status {
    margin-top: 4px;
  }
  .status[data-state="down"] {
    color: #c9302c;
  }
  .error {
    color: #c9302c;
    font-size: 12px;
  }
</style>
