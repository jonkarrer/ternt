<script lang="ts">
  import { onMount } from "svelte";
  import { browser, type AgentKeys, type AgentStatus, type Prefs } from "../lib/browser";

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
  let basetenDraft = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);

  const chosen = $derived(PROVIDERS.find((p) => p.id === provider) ?? PROVIDERS[0]);
  const saved = $derived(keys?.[provider] ?? false);

  onMount(() => {
    browser.getPrefs().then((p) => (provider = p.agentProvider));
    browser.agentKeys().then((k) => (keys = k));
    browser.agentStatus().then((s) => (status = s));
    const statuses = browser.onAgentStatus((s) => (status = s));
    const prefs = browser.onPrefsChanged((p) => (provider = p.agentProvider));
    return () => {
      statuses.then((off) => off());
      prefs.then((off) => off());
    };
  });

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
        return status.hasKey ? `Running with your ${chosen.name} key.` : `Running, waiting for a ${chosen.name} key.`;
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
      basetenDraft = "";
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

  function saveBaseten(event: Event) {
    event.preventDefault();
    const key = basetenDraft.trim();
    if (key) change({ baseten: key });
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
        {#if saved}
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
    {#if saved}
      <button class="button" type="button" disabled={saving} onclick={() => change({ [provider]: "" })}>Remove</button>
    {/if}
  </form>
  {#if error}
    <div class="row"><span class="error" role="alert">{error}</span></div>
  {/if}
</div>

<h2>Router</h2>
<div class="card">
  <form class="row" onsubmit={saveBaseten}>
    <div class="label">
      Baseten API key <span class="optional">optional</span>
      <div class="hint">
        {#if keys?.baseten}
          Saved in your Keychain. The agent's router uses a small fast model there to place a message that it couldn't
          by itself.
        {:else}
          Not set. Without it the router goes with its best guess, which is fine. The agent's router uses a small fast
          model there to place a message that it couldn't by itself.
        {/if}
      </div>
    </div>
    <input
      class="field key"
      type="password"
      bind:value={basetenDraft}
      placeholder={keys?.baseten ? "Replace…" : "Baseten key"}
      aria-label="Baseten API key"
      autocomplete="off"
      spellcheck="false"
    />
    <button class="button primary" type="submit" disabled={saving || !basetenDraft.trim()}>Save</button>
    {#if keys?.baseten}
      <button class="button" type="button" disabled={saving} onclick={() => change({ baseten: "" })}>Remove</button>
    {/if}
  </form>
</div>

<style>
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
  .optional {
    margin-left: 4px;
    font-size: 11px;
    font-weight: 400;
    color: var(--text-muted);
  }
  .error {
    color: #c9302c;
    font-size: 12px;
  }
</style>
