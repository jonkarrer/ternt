<script lang="ts">
  import { browser, type BrowserState, type Profile } from "../lib/browser";
  import Avatar from "./Avatar.svelte";
  import ProfileDetail from "./ProfileDetail.svelte";

  // Profiles are where sign-ins live: each holds its own cookies, logins and
  // history, for the spaces using it. Default first, then each separate
  // one: a list on the left, the selected one's page on the right.
  let { browserState }: { browserState: BrowserState } = $props();

  const DEFAULT = "default";
  const keyOf = (p: Profile) => p.profile ?? DEFAULT;
  let selectedKey = $state<string>(DEFAULT);
  /** Falls back to Default when the selected profile goes away. */
  const selected = $derived(
    browserState.profiles.find((p) => keyOf(p) === selectedKey) ?? browserState.profiles[0],
  );

  async function newSpace() {
    const id = await browser.createSpace();
    // New spaces get their own profile.
    const space = (await browser.getState()).spaces.find((s) => s.id === id);
    if (space?.profile) selectedKey = space.profile;
  }
</script>

<div class="profiles">
  <aside>
    <ul>
      {#each browserState.profiles as p (keyOf(p))}
        <li>
          <button class="profile" class:selected={p === selected} onclick={() => (selectedKey = keyOf(p))}>
            <Avatar profile={p} {browserState} />
            <span class="who">
              <span class="name">{p.name}</span>
              <span class="sub">{p.spaces.length === 1 ? "1 space" : `${p.spaces.length} spaces`}</span>
            </span>
          </button>
        </li>
      {/each}
    </ul>
    <button class="button add" onclick={newSpace}>+ New Space</button>
  </aside>

  {#if selected}
    <section class="detail">
      {#key keyOf(selected)}
        <ProfileDetail {browserState} profile={selected} />
      {/key}
    </section>
  {/if}
</div>

<style>
  .profiles {
    display: flex;
    height: 100%;
  }

  aside {
    flex: none;
    width: 220px;
    display: flex;
    flex-direction: column;
    padding: 12px 10px;
    border-right: 0.5px solid var(--separator);
  }
  ul {
    flex: 1;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }
  .profile {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 6px 8px;
    border-radius: 8px;
    text-align: left;
  }
  .profile:hover {
    background: var(--fill);
  }
  .profile.selected {
    background: var(--fill-strong);
  }
  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: 11px;
    color: var(--text-muted);
  }
  .add {
    align-self: flex-start;
    margin: 8px 0 0 4px;
  }

  .detail {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 24px 28px;
  }
</style>
