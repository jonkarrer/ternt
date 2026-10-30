<script lang="ts">
  import { browser, type BrowserState, type Profile } from "../lib/browser";
  import SpaceIcon from "../lib/SpaceIcon.svelte";
  import { SPACE_COLORS, spaceSwatch } from "../lib/spaces";
  import Avatar from "./Avatar.svelte";

  // One profile's page (Settings → Profiles): its name, its spaces, its data.
  // The Default profile can't be renamed.
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
      <div class="hint">The default cookie jar: every space without a profile of its own signs in here.</div>
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
      <SpaceIcon icon={space.icon} name={space.name} color={space.color} />
      <div class="label">
        {space.name}
        <div class="swatches" role="radiogroup" aria-label="Color for {space.name}">
          {#each SPACE_COLORS as color (color)}
            <button
              class="swatch"
              class:selected={space.color === color}
              role="radio"
              aria-checked={space.color === color}
              aria-label={color}
              title={color[0].toUpperCase() + color.slice(1)}
              style:--swatch={spaceSwatch(color)}
              onclick={() => browser.setSpaceColor(space.id, color)}
            ></button>
          {/each}
        </div>
      </div>
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
  .swatches {
    display: flex;
    gap: 5px;
    margin-top: 5px;
  }
  .swatch {
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: var(--swatch);
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.15);
  }
  .swatch:hover {
    scale: 1.15;
  }
  .swatch.selected {
    box-shadow:
      0 0 0 1.5px var(--card),
      0 0 0 3px var(--swatch);
  }
  .count {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
