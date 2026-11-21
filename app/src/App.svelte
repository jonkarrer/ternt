<script lang="ts">
  import { onMount } from "svelte";
  import { core } from "./lib/core";
  import Conversation from "./lib/Conversation.svelte";
  import Keys from "./lib/Keys.svelte";
  import Memory from "./lib/Memory.svelte";
  import Presence from "./lib/Presence.svelte";
  import UpdateCard from "./lib/UpdateCard.svelte";
  import { app, openThread, show, start } from "./lib/state.svelte";

  let version = $state("");

  onMount(() => {
    start();
    core.version().then((v) => (version = v));
  });

  /** "now", "12 min", "3 h", then the date. */
  function ago(ms: number) {
    const minutes = Math.floor((Date.now() - ms) / 60_000);
    if (minutes < 1) return "now";
    if (minutes < 60) return `${minutes} min`;
    if (minutes < 24 * 60) return `${Math.floor(minutes / 60)} h`;
    return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }
</script>

{#if app.status === null}
  <div class="center" data-tauri-drag-region><Presence doing={{ kind: "thinking" }} size={44} /></div>
{:else if app.status.state === "needs_keys"}
  <div class="center" data-tauri-drag-region><Keys first ondone={start} /></div>
{:else if app.status.state === "failed"}
  <div class="center" data-tauri-drag-region>
    <div class="failed">
      <h1>Tokidoki couldn't start</h1>
      <p class="selectable">{app.status.message}</p>
      <button onclick={start}>Try again</button>
    </div>
  </div>
{:else}
  <div class="window">
    <nav>
      <div class="titlebar" data-tauri-drag-region></div>
      <button
        class="new"
        class:on={app.view === "conversation" && app.current === null}
        onclick={() => openThread(null)}>New conversation</button
      >
      <button class="memory" class:on={app.view === "memory"} onclick={() => show("memory")}>
        <span class="mote" class:fresh={app.unseen > 0}></span>
        <span>Memory</span>
        <span class="count">{app.unseen > 0 ? `${app.unseen} new` : app.memories.length || ""}</span>
      </button>
      <ul>
        {#each app.threads as thread (thread.id)}
          <li>
            <button
              class:on={app.view === "conversation" && app.current === thread.id}
              title={thread.about}
              onclick={() => openThread(thread.id)}
            >
              <span class="mark">
                {#if app.live[thread.id]?.doing}<Presence doing={app.live[thread.id].doing} size={12} />{/if}
              </span>
              <span class="title">{thread.title}</span>
              <time>{ago(thread.active_at)}</time>
            </button>
          </li>
        {/each}
      </ul>
      <footer>
        <UpdateCard />
        <div class="foot">
          <button onclick={() => show("keys")}>API keys</button>
          <button onclick={() => core.checkForUpdates()} title="Check for updates">Version {version}</button>
        </div>
      </footer>
    </nav>
    <main>
      {#if app.view === "keys"}
        <div class="center"><Keys ondone={() => show("conversation")} oncancel={() => show("conversation")} /></div>
      {:else if app.view === "memory"}
        <Memory />
      {:else}
        <Conversation />
      {/if}
    </main>
  </div>
{/if}

<style>
  .center {
    height: 100%;
    display: grid;
    place-items: center;
    padding: 40px;
  }
  .failed {
    max-width: 460px;
  }
  .failed h1 {
    margin: 0 0 8px;
    font: 600 22px/1.25 var(--voice);
  }
  .failed p {
    color: var(--muted);
  }
  .failed button {
    height: 32px;
    padding: 0 14px;
    border-radius: 8px;
    background: var(--indigo);
    color: var(--paper);
    font-weight: 600;
  }
  .window {
    height: 100%;
    display: grid;
    grid-template-columns: 236px 1fr;
  }
  nav {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--side);
    border-right: 1px solid var(--line);
  }
  .titlebar {
    height: 52px;
    flex: none;
  }
  .new {
    margin: 0 10px 10px;
    height: 32px;
    border-radius: 8px;
    border: 1px solid var(--line);
    font-weight: 600;
    text-align: center;
  }
  .new.on {
    background: var(--raised);
  }
  .memory {
    display: grid;
    grid-template-columns: 14px 1fr auto;
    align-items: center;
    gap: 6px;
    margin: 0 6px 10px;
    padding: 7px 8px 7px 4px;
    border-radius: 7px;
    text-align: left;
  }
  .memory:hover {
    background: color-mix(in srgb, var(--raised) 55%, transparent);
  }
  .memory.on {
    background: var(--raised);
  }
  .mote {
    justify-self: center;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    border: 1.5px solid var(--plum);
  }
  .mote.fresh {
    background: var(--plum);
  }
  .count {
    color: var(--muted);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }
  .fresh ~ .count {
    color: var(--plum);
    font-weight: 600;
  }
  ul {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 0 6px;
    list-style: none;
  }
  li button {
    width: 100%;
    display: grid;
    grid-template-columns: 14px 1fr auto;
    align-items: center;
    gap: 6px;
    padding: 7px 8px 7px 4px;
    border-radius: 7px;
    text-align: left;
  }
  li button:hover {
    background: color-mix(in srgb, var(--raised) 55%, transparent);
  }
  li button.on {
    background: var(--raised);
  }
  .mark {
    display: grid;
    place-items: center;
  }
  .title {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  time {
    color: var(--muted);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }
  footer {
    flex: none;
    padding: 8px 10px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .foot {
    display: flex;
    justify-content: space-between;
    color: var(--muted);
    font-size: 12px;
  }
  .foot button:hover {
    color: var(--ink);
  }
  main {
    min-width: 0;
    min-height: 0;
  }
  @media (max-width: 640px) {
    .window {
      grid-template-columns: 1fr;
    }
    nav {
      display: none;
    }
  }
</style>
