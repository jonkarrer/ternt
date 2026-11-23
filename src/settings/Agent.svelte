<script lang="ts">
  import { onMount } from "svelte";
  import {
    browser,
    type AgentKeys,
    type AgentProvider,
    type KeySource,
    type AgentSpaceStatus,
    type AgentStatus,
    type Space,
  } from "../lib/browser";
  import type { Memory } from "../agent/protocol";

  let { spaces, activeSpace }: { spaces: Space[]; activeSpace: number | null } = $props();

  // Each space has its own agent (⌘J), and each picks who answers for it and what it may use. The keys are
  // yours and shared between spaces: in the Keychain, handed to the agent process, nowhere else.
  const PROVIDERS: { id: AgentProvider; name: string; keyHint: string; where: string }[] = [
    { id: "fireworks", name: "Fireworks AI", keyHint: "fw_…", where: "fireworks.ai → API keys" },
    { id: "anthropic", name: "Anthropic", keyHint: "sk-ant-…", where: "console.anthropic.com → API keys" },
  ];

  // What a space's agent can be given to do, each switched on per space. One that needs a
  // key of its own names it; that key is shared between spaces like the providers'.
  const TOOLS: { id: string; name: string; hint: string; key: keyof AgentKeys; keyName: string }[] = [
    {
      id: "web_search",
      name: "Web search",
      hint: "Looks things up through Exa when it needs to, and says where it read them.",
      key: "exa",
      keyName: "Exa",
    },
  ];
  /** Every key Settings keeps: the providers', then the tools'. */
  const KEYS: { id: keyof AgentKeys; name: string; keyHint: string; where: string }[] = [
    ...PROVIDERS,
    { id: "exa", name: "Exa", keyHint: "Web search…", where: "dashboard.exa.ai → API keys" },
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
  let drafts = $state<Record<keyof AgentKeys, string>>({ fireworks: "", anthropic: "", exa: "" });
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

  /** A tool is on when the space asks for it. Whether it runs is the process's to say. */
  const isOn = (tool: string) => space?.agent.tools.includes(tool) ?? false;
  const keyFor = (tool: (typeof TOOLS)[number]): KeySource => keys?.[tool.key] ?? "none";
  /** The stand-in agent needs no keys; the real one can't run a tool without its key. */
  const lacksKey = (tool: (typeof TOOLS)[number]) => !stub && keyFor(tool) === "none";

  async function setTool(tool: string, event: Event) {
    const on = (event.currentTarget as HTMLInputElement).checked;
    if (!space || on === isOn(tool)) return;
    const tools = on ? [...space.agent.tools, tool].sort() : space.agent.tools.filter((t) => t !== tool);
    spaceError = null;
    try {
      // Like the provider: at once, and the conversations already going get it too.
      spaceStatus = await browser.agentSetSpace(space.id, { ...space.agent, tools });
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

  async function change(update: Partial<Record<keyof AgentKeys, string>>) {
    saving = true;
    keyError = null;
    try {
      await browser.agentSetKeys(update);
      keys = await browser.agentKeys();
      for (const id of Object.keys(update) as (keyof AgentKeys)[]) drafts[id] = "";
    } catch (e) {
      keyError = String(e);
    } finally {
      saving = false;
    }
  }

  function saveKey(id: keyof AgentKeys, event: Event) {
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

<header class="pane">
  <span class="glyph" aria-hidden="true">
    <svg viewBox="0 0 16 16"><path d="M8 1.5 9.6 6l4.4 1.6L9.6 9.4 8 14l-1.6-4.6L2 7.6 6.4 6Z" /></svg>
  </span>
  <div class="about">
    <div class="title">Tokidoki</div>
    <p class="hint">Every space has its own agent, with its own memory. Press ⌘J to talk to the one for the space you're in.</p>
    {#if status}<div class="hint status process" data-state={status.state}><i></i>{processText}</div>{/if}
  </div>
  {#if status?.state === "down"}
    <button class="button" onclick={() => browser.agentRestart()}>Restart</button>
  {/if}
</header>

<div class="section-head">
  <h2>Space</h2>
  <select class="space" aria-label="Space" value={spaceId} onchange={pickSpace}>
    {#each spaces as s (s.id)}
      <option value={s.id}>{s.name}</option>
    {/each}
  </select>
</div>
<div class="card">
  <div class="row">
    <div class="label">
      Provider
      {#if spaceText}<div class="hint status" data-state={spaceStatus && !spaceStatus.hasKey && !stub ? "keys" : ""}>{spaceText}</div>{/if}
    </div>
    <select class="provider" aria-label="Provider" value={provider} disabled={!space} onchange={setProvider}>
      {#each PROVIDERS as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
  </div>
  {#each TOOLS as tool (tool.id)}
    {@const on = isOn(tool.id)}
    <label class="row tool" data-tool={tool.id}>
      <div class="label">
        {tool.name}
        <div class="hint">{tool.hint}</div>
        {#if lacksKey(tool)}
          <div class="hint status" data-state={on ? "keys" : ""}>
            {on ? `On, but waiting for an ${tool.keyName} key` : `Needs an ${tool.keyName} key`}: add yours under API keys.
          </div>
        {/if}
      </div>
      <!-- Without its key it can't be switched on, but one left on can still be switched off. -->
      <input
        type="checkbox"
        {...{ switch: true }}
        checked={on}
        disabled={!space || (lacksKey(tool) && !on)}
        aria-label={tool.name}
        onchange={(e) => setTool(tool.id, e)}
      />
    </label>
  {/each}
  {#if spaceError}
    <div class="row"><span class="error" role="alert">{spaceError}</span></div>
  {/if}
</div>

<h2>Memory</h2>
<p class="caption">
  What the {space?.name ?? ""} agent has learned from your conversations and brings into later ones. Wiping it keeps the
  conversations.
</p>
<div class="card">
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
          <div class="actions">
            <button class="button quiet edit" onclick={() => edit(memory)}>Edit</button>
            <button class="button quiet danger forget" onclick={() => forget(memory)}>Forget</button>
          </div>
        {/if}
      </div>
    {/each}
    {#if facts.length > shown.length}
      <div class="row">
        <button class="button quiet more" onclick={() => (showAll = true)}>Show all {facts.length}</button>
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

<h2>API keys</h2>
<div class="card">
  {#each KEYS as p (p.id)}
    {@const source = keys?.[p.id] ?? "none"}
    {@const draft = drafts[p.id].trim()}
    <form class="row" onsubmit={(e) => saveKey(p.id, e)}>
      <div class="label">
        {p.name}
        <div class="hint">
          {#if source === "environment"}
            From Ternt's own environment (development); Settings can't change it.
          {:else if source === "keychain"}
            Saved in your Keychain.
          {:else}
            Not set. Make one at {p.where}.
          {/if}
        </div>
      </div>
      <div class="entry">
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
        <!-- One button at a time: Save once something is typed, else Remove for a saved key. -->
        {#if draft}
          <button class="button primary" type="submit" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
        {:else if source === "keychain"}
          <button class="button" type="button" disabled={saving} onclick={() => change({ [p.id]: "" })}>Remove</button>
        {/if}
      </div>
    </form>
  {/each}
  {#if keyError}
    <div class="row"><span class="error" role="alert">{keyError}</span></div>
  {/if}
</div>
<p class="caption">
  Shared by every space and kept in your Keychain, never in a file. Your messages go from Ternt to the provider you chose
  and nowhere else.
</p>

<style>
  /* The page's head: what this is, and whether it is running. */
  .pane {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 32px;
  }
  .glyph {
    flex: none;
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    border-radius: 13px;
    background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 82%, white), var(--accent));
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.14);
  }
  .glyph svg {
    width: 28px;
    height: 28px;
    fill: white;
  }
  .about {
    flex: 1;
    min-width: 0;
  }
  .title {
    font-size: 17px;
    font-weight: 600;
  }
  .about p {
    margin: 2px 0 0;
    max-width: 52ch;
  }
  .process {
    display: flex;
    align-items: baseline;
    gap: 7px;
    margin-top: 8px;
    color: var(--text);
  }
  .process i {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--warn);
  }
  .process[data-state="running"] i {
    background: var(--ok);
  }
  .process[data-state="down"] i {
    background: var(--danger);
  }

  .space,
  .provider {
    width: 170px;
  }
  /* The field gives way to its button, so the row's right edge and its label stay put. */
  .entry {
    flex: none;
    display: flex;
    gap: 8px;
    width: 260px;
  }
  .key {
    flex: 1;
    min-width: 0;
  }
  .memory .text {
    display: block;
    overflow-wrap: anywhere;
  }
  .memory form.edit {
    display: flex;
    flex: 1;
    gap: 8px;
    min-width: 0;
  }
  .memory-text {
    flex: 1;
    min-width: 0;
  }
  .actions {
    flex: none;
    display: flex;
    gap: 2px;
    margin-right: -8px;
  }
  .more {
    margin-left: -8px;
  }
  .status {
    margin-top: 4px;
  }
  .status[data-state="keys"] {
    color: var(--danger);
  }
</style>
