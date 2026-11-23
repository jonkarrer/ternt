<script lang="ts">
  import { onMount } from "svelte";
  import {
    browser,
    type AgentKeys,
    type AgentProvider,
    type AgentSpaceStatus,
    type AgentStatus,
    type Space,
  } from "../lib/browser";
  import type { Memory } from "../agent/protocol";

  let { spaces, activeSpace }: { spaces: Space[]; activeSpace: number | null } = $props();

  // Each space has its own agent (⌘J), and each picks who answers for it. The keys are
  // yours and shared between spaces: in the Keychain, handed to the agent process, nowhere else.
  const PROVIDERS: { id: AgentProvider; name: string; keyHint: string; where: string }[] = [
    { id: "fireworks", name: "Fireworks AI", keyHint: "fw_…", where: "fireworks.ai → API keys" },
    { id: "anthropic", name: "Anthropic", keyHint: "sk-ant-…", where: "console.anthropic.com → API keys" },
  ];

  /** The space whose agent is shown and changed (the active one until picked). */
  let spaceId = $state<number | null>(null);
  const space = $derived(spaces.find((s) => s.id === spaceId) ?? null);
  const provider = $derived<AgentProvider>(space?.agent.provider ?? "fireworks");
  const chosen = $derived(PROVIDERS.find((p) => p.id === provider) ?? PROVIDERS[0]);

  /** The process. */
  let status = $state<AgentStatus | null>(null);
  const stub = $derived(status?.state === "running" && status.backend === "stub");
  /** How the chosen space's agent runs, as the process says; `null` until asked. */
  let spaceStatus = $state<AgentSpaceStatus | null>(null);
  let spaceError = $state<string | null>(null);

  let keys = $state<AgentKeys | null>(null);
  let drafts = $state<Record<AgentProvider, string>>({ fireworks: "", anthropic: "" });
  let saving = $state(false);
  let keyError = $state<string | null>(null);

  $effect(() => {
    // Follow the active space until the user picks one; survive a space being deleted.
    if (spaceId === null || !spaces.some((s) => s.id === spaceId)) {
      spaceId = spaces.some((s) => s.id === activeSpace) ? activeSpace : (spaces[0]?.id ?? null);
    }
  });

  onMount(() => {
    browser.agentKeys().then((k) => (keys = k));
    browser.agentStatus().then((s) => (status = s));
    const statuses = browser.onAgentStatus((s) => {
      // The process came (back) up: ask it again how the space runs and what it remembers.
      if (s.state === "running" && status?.state !== "running") {
        askSpace();
        reload();
      }
      status = s;
    });
    // Coming back from a conversation: the agent may have learned something.
    window.addEventListener("focus", reload);
    return () => {
      statuses.then((off) => off());
      window.removeEventListener("focus", reload);
    };
  });

  // --- The space's agent -------------------------------------------------------

  let asking = 0;
  async function askSpace() {
    const id = spaceId;
    if (id === null) return;
    const turn = ++asking;
    spaceError = null;
    try {
      const now = await browser.agentSpaceStatus(id);
      if (turn === asking && spaceId === id) spaceStatus = now;
    } catch (e) {
      if (turn === asking) {
        spaceStatus = null;
        spaceError = String(e);
      }
    }
  }

  $effect(() => {
    const id = spaceId;
    spaceStatus = null;
    if (id !== null) askSpace();
  });

  function pickSpace(event: Event) {
    spaceId = Number((event.currentTarget as HTMLSelectElement).value);
  }

  async function setProvider(event: Event) {
    const next = (event.currentTarget as HTMLSelectElement).value as AgentProvider;
    if (!space || next === provider) return;
    spaceError = null;
    try {
      // The agent runs on the new provider at once; its conversations go on.
      spaceStatus = await browser.agentSetSpace(space.id, { ...space.agent, provider: next });
    } catch (e) {
      spaceError = String(e);
    }
  }

  const processText = $derived.by(() => {
    if (!status) return "";
    switch (status.state) {
      case "starting":
        return "Starting the agent process…";
      case "restarting":
        return status.asked ? "Restarting the agent process…" : "The agent process stopped; starting it again…";
      case "down":
        return status.reason;
      default:
        return stub ? "A stand-in agent is running (this build has no tokidoki); no key is needed." : "Running.";
    }
  });

  const spaceText = $derived.by(() => {
    if (!space) return "";
    if (stub) return "The stand-in agent answers; a provider makes no difference to it.";
    if (status && status.state !== "running") return "Waiting for the agent process.";
    if (!spaceStatus) return "Asking the agent…";
    if (!spaceStatus.hasKey) return `Waiting for a ${chosen.name} key: add yours under Keys.`;
    const model = spaceStatus.model?.replace(/^accounts\/fireworks\/models\//, "");
    return model ? `Answering with ${model}.` : "Answering.";
  });

  // --- Keys ---------------------------------------------------------------------

  async function change(update: Partial<Record<AgentProvider, string>>) {
    saving = true;
    keyError = null;
    try {
      await browser.agentSetKeys(update);
      keys = await browser.agentKeys();
      for (const id of Object.keys(update) as AgentProvider[]) drafts[id] = "";
    } catch (e) {
      keyError = String(e);
    } finally {
      saving = false;
    }
  }

  function saveKey(id: AgentProvider, event: Event) {
    event.preventDefault();
    const key = drafts[id].trim();
    if (key) change({ [id]: key });
  }

  // --- Memory ------------------------------------------------------------------
  // What the space's agent remembers: its facts, to correct or forget one by one, or
  // wipe all at once. Procedures and episodes are kept too but not listed.

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

  let loading = 0;
  async function load(id: number) {
    const turn = ++loading;
    memoryError = null;
    try {
      const list = await browser.agentMemories(id);
      if (turn === loading) memories = list;
    } catch (e) {
      if (turn === loading) {
        memories = memories ?? [];
        memoryError = String(e);
      }
    }
  }

  function reload() {
    if (spaceId !== null) load(spaceId);
  }

  $effect(() => {
    const id = spaceId;
    memories = null;
    editing = null;
    showAll = false;
    confirmWipe = false;
    if (id !== null) load(id);
  });

  function edit(memory: Memory) {
    editing = memory.id;
    editDraft = memory.text;
    confirmWipe = false;
  }

  async function saveEdit(event: Event) {
    event.preventDefault();
    const id = editing;
    const text = editDraft.trim();
    if (id === null || spaceId === null || !text) return;
    const space = spaceId;
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
    if (spaceId === null) return;
    const space = spaceId;
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
    if (spaceId === null) return;
    const space = spaceId;
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
</script>

<h2>Agent</h2>
<div class="card">
  <div class="row">
    <div class="label">
      Tokidoki, your spaces' agents
      <div class="hint">
        Press ⌘J to talk to the agent of the space you are in. Each space has its own, with its own memory and its own
        choice of who answers. Ternt runs them in a separate process; your messages go from it to the provider and
        nowhere else.
      </div>
      {#if status}<div class="hint status" data-state={status.state}>{processText}</div>{/if}
    </div>
    {#if status?.state === "down"}
      <button class="button" onclick={() => browser.agentRestart()}>Restart</button>
    {/if}
  </div>
</div>

<h2>Space</h2>
<div class="card">
  <div class="row">
    <div class="label">
      Settings for
      <div class="hint">Each space's agent is set up on its own.</div>
    </div>
    <select class="field space" aria-label="Space" value={spaceId} onchange={pickSpace}>
      {#each spaces as s (s.id)}
        <option value={s.id}>{s.name}</option>
      {/each}
    </select>
  </div>
  <div class="row">
    <div class="label">
      Provider
      <div class="hint">Who answers in this space, with your key for them. Changing it keeps the conversations.</div>
      {#if spaceText}<div class="hint status" data-state={spaceStatus && !spaceStatus.hasKey && !stub ? "keys" : ""}>{spaceText}</div>{/if}
    </div>
    <select class="field provider" aria-label="Provider" value={provider} disabled={!space} onchange={setProvider}>
      {#each PROVIDERS as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
  </div>
  {#if spaceError}
    <div class="row"><span class="error" role="alert">{spaceError}</span></div>
  {/if}
</div>

<h2>Keys</h2>
<div class="card">
  {#each PROVIDERS as p (p.id)}
    {@const source = keys?.[p.id] ?? "none"}
    <form class="row" onsubmit={(e) => saveKey(p.id, e)}>
      <div class="label">
        {p.name} API key
        <div class="hint">
          {#if source === "environment"}
            From Ternt's own environment (development); Settings can't change it.
          {:else if source === "keychain"}
            Saved in your Keychain.
          {:else}
            Not set. Make one at {p.where}; it is kept in your Keychain, never in a file.
          {/if}
        </div>
      </div>
      <input
        class="field key"
        data-provider={p.id}
        type="password"
        bind:value={drafts[p.id]}
        placeholder={source === "none" ? p.keyHint : "Replace…"}
        aria-label="{p.name} API key"
        autocomplete="off"
        spellcheck="false"
      />
      <button class="button primary" type="submit" disabled={saving || !drafts[p.id].trim()}>{saving ? "Saving…" : "Save"}</button>
      {#if source === "keychain"}
        <button class="button" type="button" disabled={saving} onclick={() => change({ [p.id]: "" })}>Remove</button>
      {/if}
    </form>
  {/each}
  {#if keyError}
    <div class="row"><span class="error" role="alert">{keyError}</span></div>
  {/if}
</div>

<h2>Memory</h2>
<div class="card">
  <div class="row">
    <div class="label">
      What the {space?.name ?? ""} agent remembers
      <div class="hint">
        Facts it learned from your conversations in this space, brought into later ones. Edit a fact to correct it, or
        forget it. Wipe forgets everything the agent has learned here; the conversations stay.
      </div>
    </div>
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
        Forget everything the {space?.name ?? ""} agent has learned? There is no undo.
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
  .space,
  .provider {
    flex: none;
    width: 160px;
  }
  .key {
    flex: 1;
    min-width: 0;
    max-width: 260px;
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
  .status {
    margin-top: 4px;
  }
  .status[data-state="down"],
  .status[data-state="keys"] {
    color: #c9302c;
  }
  .error {
    color: #c9302c;
    font-size: 12px;
  }
</style>
