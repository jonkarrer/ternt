<script lang="ts">
  import { onMount } from "svelte";
  import { browser, type BrowserState, type Prefs } from "../lib/browser";
  import Avatar from "./Avatar.svelte";

  // You, the person using this Ternt: a name and an email, kept on this Mac.
  // Your sign-ins live in profiles (one per group of spaces), listed below.
  let { browserState, onprofiles }: { browserState: BrowserState; onprofiles: () => void } = $props();

  let account = $state<Prefs["account"] | null>(null);
  onMount(() => {
    browser.getPrefs().then((p) => (account = p.account));
  });

  const initial = $derived(account?.name.trim().charAt(0).toUpperCase() || "T");

  function save(field: "name" | "email", event: Event) {
    if (!account) return;
    const value = (event.currentTarget as HTMLInputElement).value.trim();
    if (value === account[field]) return;
    account = { ...account, [field]: value };
    browser.updatePrefs({ account });
  }

  function keys(field: "name" | "email", event: KeyboardEvent) {
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === "Enter") input.blur();
    if (event.key === "Escape" && account) {
      input.value = account[field];
      input.blur();
    }
  }
</script>

{#if account}
  <header>
    <span class="avatar">{initial}</span>
    <div class="fields">
      <input
        class="field name"
        value={account.name}
        placeholder="Your name"
        aria-label="Name"
        onblur={(e) => save("name", e)}
        onkeydown={(e) => keys("name", e)}
      />
      <input
        class="field email"
        type="email"
        value={account.email}
        placeholder="Email"
        aria-label="Email"
        spellcheck="false"
        onblur={(e) => save("email", e)}
        onkeydown={(e) => keys("email", e)}
      />
      <div class="hint">Kept on this Mac. Sign-ins to sites live in your profiles.</div>
    </div>
  </header>

  <h2>Profiles</h2>
  <div class="card">
    {#each browserState.profiles as p (p.profile ?? "default")}
      <div class="row">
        <Avatar profile={p} {browserState} />
        <span class="label">
          {p.name}
          <div class="hint">
            {p.spaces.length
              ? p.spaces.map((id) => browserState.spaces.find((s) => s.id === id)?.name).join(", ")
              : "No spaces"}
          </div>
        </span>
      </div>
    {/each}
    <div class="row">
      <span class="label hint">Each profile keeps its own cookies, logins and history for the spaces using it.</span>
      <button class="button" onclick={onprofiles}>Manage Profiles…</button>
    </div>
  </div>
{/if}

<style>
  header {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 24px;
  }
  .avatar {
    flex: none;
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    border-radius: 50%;
    background: var(--accent);
    color: white;
    font-size: 28px;
    font-weight: 600;
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.12);
  }
  .fields {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field {
    width: 100%;
  }
  .name {
    height: 28px;
    font-size: 15px;
    font-weight: 600;
  }
  .hint {
    font-size: 11px;
    color: var(--text-muted);
  }
</style>
