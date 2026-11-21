<script lang="ts">
  import { onMount } from "svelte";
  import { core } from "./core";
  import Presence from "./Presence.svelte";

  // Asks for the user's own API keys: at first launch (`first`), and later from the sidebar
  // to replace them. A field left empty keeps the key already stored.
  let { first = false, ondone, oncancel }: { first?: boolean; ondone: () => void; oncancel?: () => void } = $props();

  let anthropic = $state("");
  let baseten = $state("");
  let have = $state({ anthropic: false, baseten: false });
  let saving = $state(false);
  let error = $state<string | null>(null);

  onMount(async () => {
    have = await core.keysSet();
  });

  const ready = $derived(anthropic.trim() !== "" || (have.anthropic && baseten.trim() !== ""));

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!ready || saving) return;
    saving = true;
    error = null;
    try {
      await core.setKeys(anthropic.trim() || null, baseten.trim() || null);
      ondone();
    } catch (problem) {
      error = String(problem);
    } finally {
      saving = false;
    }
  }
</script>

<form class="keys" onsubmit={save}>
  {#if first}
    <Presence doing={saving ? { kind: "thinking" } : null} size={44} />
    <h1>Tokidoki runs on your own keys</h1>
    <p class="lede">
      It talks to the models directly from this Mac. Your keys are kept in the Keychain and sent nowhere else.
    </p>
  {:else}
    <h1>API keys</h1>
  {/if}

  <label>
    <span>Anthropic API key</span>
    <input
      type="password"
      bind:value={anthropic}
      placeholder={have.anthropic ? "Saved. Paste a new key to replace it" : "sk-ant-…"}
      autocomplete="off"
      spellcheck="false"
    />
    <small>Answers and memory. Create one at console.anthropic.com, under API keys.</small>
  </label>

  <label>
    <span>Baseten API key <em>optional</em></span>
    <input
      type="password"
      bind:value={baseten}
      placeholder={have.baseten ? "Saved. Paste a new key to replace it" : ""}
      autocomplete="off"
      spellcheck="false"
    />
    <small>
      A small fast model that helps file a message into the right conversation when it's unclear. Without it,
      Tokidoki goes with its best guess.
    </small>
  </label>

  {#if error}<p class="error selectable" role="alert">{error}</p>{/if}

  <div class="actions">
    {#if oncancel}<button type="button" class="quiet" onclick={oncancel}>Cancel</button>{/if}
    <button type="submit" class="primary" disabled={!ready || saving}>
      {saving ? "Checking the key…" : first ? "Save keys and start" : "Save keys"}
    </button>
  </div>
  {#if !first}<p class="note">Tokidoki restarts to use new keys. Nothing is lost.</p>{/if}
</form>

<style>
  .keys {
    width: min(440px, 100%);
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  h1 {
    margin: 6px 0 0;
    font: 600 22px/1.25 var(--voice);
    letter-spacing: -0.01em;
  }
  .lede {
    margin: -8px 0 4px;
    color: var(--muted);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  label span {
    font-weight: 600;
  }
  label em {
    font-style: normal;
    font-weight: 400;
    color: var(--muted);
    margin-left: 4px;
  }
  input {
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--raised);
    font-family: var(--code);
    font-size: 13px;
  }
  input:focus {
    outline: 2px solid var(--indigo);
    outline-offset: -1px;
  }
  small {
    color: var(--muted);
    font-size: 12px;
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions button {
    height: 32px;
    padding: 0 14px;
    border-radius: 8px;
    font-weight: 600;
  }
  .primary {
    background: var(--indigo);
    color: var(--paper);
  }
  .primary:disabled {
    opacity: 0.45;
  }
  .quiet {
    background: var(--side);
  }
  .note {
    margin: -8px 0 0;
    text-align: right;
    color: var(--muted);
    font-size: 12px;
  }
</style>
