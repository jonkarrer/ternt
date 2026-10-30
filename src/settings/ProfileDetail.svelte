<script lang="ts">
  import { browser, type BrowserState, type Profile } from "../lib/browser";
  import { spaceSwatch } from "../lib/spaces";
  import Avatar from "./Avatar.svelte";

  // One profile's page: its name, its spaces, its data. The Ternt page shows
  // the main profile; Profiles shows the selected space profile.
  let { browserState, profile: selected }: { browserState: BrowserState; profile: Profile } = $props();

  const isMain = $derived(selected.profile == null);
  const spaceById = (id: number) => browserState.spaces.find((s) => s.id === id);
  const members = $derived(selected.spaces.map(spaceById).filter((s) => s != null));
  const others = $derived(browserState.spaces.filter((s) => !selected.spaces.includes(s.id)));
  const favorites = $derived(
    browserState.tabs.filter((t) => t.zone === "favorite" && spaceById(t.space)?.profile === selected.profile).length,
  );

  function rename(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    if (selected.profile && input.value.trim() !== selected.name) browser.renameProfile(selected.profile, input.value);
  }

  function renameKeys(event: KeyboardEvent) {
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === "Enter") input.blur();
    if (event.key === "Escape") {
      input.value = selected.name;
      input.blur();
    }
  }

  // Two clicks to clear history, so a stray click can't.
  let confirmClear = $state(false);
  let cleared = $state(false);
  $effect(() => {
    void selected.profile;
    confirmClear = false;
    cleared = false;
  });
  function clearHistory() {
    if (!confirmClear) {
      confirmClear = true;
      return;
    }
    browser.clearProfileHistory(selected.profile);
    confirmClear = false;
    cleared = true;
  }
</script>

<header>
  <Avatar profile={selected} {browserState} large />
  <div>
    {#if isMain}
      <div class="title">{selected.name}</div>
      <div class="hint">The main profile, for every space that doesn't have its own.</div>
    {:else}
      {#key selected.profile}
        <input
          class="field title-field"
          value={selected.name}
          placeholder={members.map((s) => s.name).join(", ")}
          aria-label="Profile name"
          onblur={rename}
          onkeydown={renameKeys}
        />
      {/key}
      <div class="hint">Its own cookies, logins and history. Clear the name to name it after its spaces.</div>
    {/if}
  </div>
</header>

<h2>Spaces</h2>
<div class="card">
  {#each members as space (space.id)}
    <div class="row">
      {#if space.icon}
        <span class="space-icon">{space.icon}</span>
      {:else}
        <span class="dot" style:background={spaceSwatch(space.color)}></span>
      {/if}
      <span class="label">{space.name}</span>
      <select
        aria-label="Profile for {space.name}"
        value={selected.choice}
        onchange={(e) => browser.spaceProfile(space.id, e.currentTarget.value)}
      >
        {#each browserState.profiles as p (p.choice)}
          <option value={p.choice}>{p.name}</option>
        {/each}
        <option value="new">New Profile (signed out)</option>
      </select>
    </div>
  {:else}
    <div class="row"><span class="label hint">No space uses this profile.</span></div>
  {/each}
  {#if others.length}
    <div class="row">
      <span class="label hint">Move another space here; it signs in with this profile.</span>
      <select
        aria-label="Add a space to {selected.name}"
        value=""
        onchange={(e) => {
          const id = Number(e.currentTarget.value);
          e.currentTarget.value = "";
          if (id) browser.spaceProfile(id, selected.choice);
        }}
      >
        <option value="" disabled>Add Space…</option>
        {#each others as space (space.id)}
          <option value={space.id}>{space.name}</option>
        {/each}
      </select>
    </div>
  {/if}
</div>

<h2>Data</h2>
<div class="card">
  <div class="row">
    <div class="label">
      Favorites
      <div class="hint">Shown in every space using this profile.</div>
    </div>
    <span class="count">{favorites}</span>
  </div>
  <div class="row">
    <div class="label">
      History
      <div class="hint">Pages visited in this profile, searched by the command bar.</div>
    </div>
    <button class="button" class:danger={confirmClear} disabled={cleared} onclick={clearHistory}>
      {cleared ? "Cleared" : confirmClear ? "Clear History?" : "Clear History…"}
    </button>
  </div>
</div>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: 24px;
  }
  header > div {
    flex: 1;
    min-width: 0;
  }
  .title {
    font-size: 17px;
    font-weight: 600;
  }
  .title-field {
    width: 100%;
    height: 28px;
    font-size: 15px;
    font-weight: 600;
  }
  .hint {
    margin-top: 3px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .dot {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
  .space-icon {
    flex: none;
    width: 10px;
    font-size: 14px;
    line-height: 1;
    text-align: center;
  }
  .count {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
