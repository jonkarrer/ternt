<script lang="ts">
  import { onMount } from "svelte";
  import { browser, type AgentKeys, type AgentProvider, type AgentStatus } from "../lib/browser";
  import { EXA, JEV, PROVIDERS, type KeyInfo } from "../lib/agentKeys";
  import DefaultBrowser from "../settings/DefaultBrowser.svelte";

  // First launch (and Help → Set Up Ternt…): whether to use the AI agent, and the keys it
  // needs if so. One provider's key is required to go on with the agent on; Jev's is asked
  // for by name, because the agent is much better with it; Exa's is there for web search.
  // Nothing is saved until Continue, and then the keys go to the Keychain and nowhere else.

  let on = $state(true);
  let keys = $state<AgentKeys | null>(null);
  let status = $state<AgentStatus | null>(null);
  let drafts = $state<Record<keyof AgentKeys, string>>({ fireworks: "", anthropic: "", exa: "", typesafe: "" });
  let saving = $state(false);
  let error = $state<string | null>(null);

  /** The stand-in agent of a development build answers without any key. */
  const stub = $derived(status?.state === "running" && status.backend === "stub");
  const has = (id: keyof AgentKeys) => (keys?.[id] ?? "none") !== "none" || drafts[id].trim() !== "";
  const needsProvider = $derived(on && !stub && keys !== null && !PROVIDERS.some((p) => has(p.id)));
  const missesJev = $derived(on && !stub && keys !== null && !has("typesafe"));

  onMount(() => {
    browser.getPrefs().then((p) => (on = p.agentEnabled));
    browser.agentKeys().then((k) => (keys = k));
    browser.agentStatus().then((s) => (status = s));
    const statuses = browser.onAgentStatus((s) => (status = s));
    return () => statuses.then((off) => off());
  });

  async function finish(event: Event) {
    event.preventDefault();
    if (needsProvider || saving) return;
    saving = true;
    error = null;
    try {
      const typed = Object.fromEntries(
        (Object.keys(drafts) as (keyof AgentKeys)[]).filter((id) => drafts[id].trim()).map((id) => [id, drafts[id].trim()]),
      );
      const keyed = on && Object.keys(typed).length > 0;
      // The keys first: an agent switched on here starts with them, once, instead of
      // starting without and being restarted for them.
      if (keyed) await browser.agentSetKeys(typed);
      const prefs = await browser.getPrefs();
      if (prefs.agentEnabled !== on) await browser.updatePrefs({ agentEnabled: on });
      if (keyed) await useKeyedProviders(await browser.agentKeys());
      await browser.closeOnboarding();
    } catch (e) {
      error = String(e);
      saving = false;
    }
  }

  /** A space on a provider there is no key for moves to the one there is a key for. */
  async function useKeyedProviders(now: AgentKeys) {
    const keyed = PROVIDERS.filter((p) => now[p.id] !== "none").map((p) => p.id);
    if (keyed.length === 0) return;
    const { spaces } = await browser.getState();
    for (const space of spaces) {
      if (keyed.includes(space.agent.provider)) continue;
      const provider: AgentProvider = keyed[0];
      // Saved with the space at once; telling the agent can fail while it restarts with
      // the new keys, and it is told again when the space is first used.
      await browser.agentSetSpace(space.id, { ...space.agent, provider }).catch(() => {});
    }
  }
</script>

{#snippet keyRow(p: KeyInfo, note: string)}
  {@const source = keys?.[p.id] ?? "none"}
  <label class="row">
    <div class="label">
      {p.name}
      {#if p.recommended && !has(p.id)}<span class="badge">Recommended</span>{/if}
      <div class="hint">
        {#if source === "keychain"}
          Saved in your Keychain.
        {:else if source === "environment"}
          Already set in Ternt's environment.
        {:else}
          {note} Make one at {p.where}.
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
  </label>
{/snippet}

<form class="window" onsubmit={finish}>
  <div class="titlebar" data-tauri-drag-region></div>
  <main>
    <header>
      <svg class="bolt" viewBox="300 120 440 790" aria-hidden="true">
        <polygon points="612,150 318,572 494,572 404,884 722,428 556,428 656,150" />
      </svg>
      <div>
        <h1>Set up Ternt</h1>
        <p class="hint">A browser and a terminal in one window. Two things to settle before you start.</p>
      </div>
    </header>

    <div class="section-head">
      <h2>AI agent</h2>
      <input
        class="agent"
        type="checkbox"
        {...{ switch: true }}
        bind:checked={on}
        aria-label="Use the AI agent"
      />
    </div>
    <p class="caption">
      Every space gets its own assistant with its own memory: press ⌃A to ask it. It runs on your own API keys, which
      stay in your Keychain and go only to the services they are for.
    </p>
    {#if on}
      <div class="card keys">
        {#each PROVIDERS as p, i (p.id)}
          {@render keyRow(p, `Answers you. Required, unless you use ${PROVIDERS[1 - i].name}.`)}
        {/each}
        {@render keyRow(JEV, "Sorts messages and memory, fast and for very little. Highly recommended.")}
        {@render keyRow(EXA, "Optional: web search.")}
      </div>
    {:else}
      <div class="card">
        <div class="row">
          <div class="label hint off">
            Ternt works as a browser and a terminal without it: no ⌃A, nothing running in the background.
            Switch it on any time in Settings → Agent.
          </div>
        </div>
      </div>
    {/if}

    <DefaultBrowser />
  </main>

  <footer>
    <span class="say" class:error={error !== null} role="status">
      {#if error}
        {error}
      {:else if needsProvider}
        Add a Fireworks AI or Anthropic key to use the agent, or switch it off.
      {:else if missesJev}
        Ready. The agent works without Jev, and noticeably better with it.
      {:else}
        You can change all of this later in Settings (⌘,).
      {/if}
    </span>
    <button class="button primary continue" type="submit" disabled={needsProvider || saving}>
      {saving ? "Saving…" : "Continue"}
    </button>
  </footer>
</form>

<style>
  .window {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  /* Room for the traffic lights, and to drag the window by. */
  .titlebar {
    flex: none;
    height: 38px;
  }
  main {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 40px 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 32px;
  }
  .bolt {
    flex: none;
    width: 52px;
    height: 52px;
    padding: 9px;
    border-radius: 13px;
    background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 82%, white), var(--accent));
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.14);
    fill: white;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  header p {
    margin: 3px 0 0;
  }
  .key {
    flex: none;
    width: 220px;
  }
  .off {
    max-width: 60ch;
  }
  footer {
    flex: none;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 14px 40px 18px;
    border-top: 0.5px solid var(--row-separator);
  }
  .say {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .continue {
    min-width: 96px;
  }
</style>
