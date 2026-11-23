<script lang="ts">
  import { onMount } from "svelte";
  import { browser } from "../lib/browser";
  import { updates, watchUpdates } from "../lib/updates.svelte";

  let version = $state("");
  let unloadTabs = $state(true);
  /** Sites allowed the camera and microphone. */
  let mediaSites = $state<string[]>([]);
  watchUpdates();
  const status = $derived(updates.status);

  onMount(() => {
    browser.appVersion().then((v) => (version = v));
    const load = () =>
      browser.getPrefs().then((p) => {
        unloadTabs = p.unloadTabs;
        mediaSites = p.mediaSites;
      });
    load();
    // A site allowed in a tab while Settings is open.
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  });

  function forgetSite(site: string) {
    mediaSites = mediaSites.filter((s) => s !== site);
    browser.updatePrefs({ mediaSites });
  }

  function setUnloadTabs(event: Event) {
    unloadTabs = (event.currentTarget as HTMLInputElement).checked;
    browser.updatePrefs({ unloadTabs });
  }

  const busy = $derived(status.state === "downloading" || status.state === "installing");
  const updateText = $derived.by(() => {
    switch (status.state) {
      case "ready":
        return `Ternt ${status.version} is downloaded and installs when you quit.`;
      case "downloading":
        return status.progress == null
          ? `Downloading ${status.version}…`
          : `Downloading ${status.version}… ${Math.round(status.progress * 100)}%`;
      case "installing":
        return `Installing ${status.version}, Ternt will restart…`;
      case "upToDate":
        return "Ternt is up to date.";
      case "failed":
        return `The last check or update failed: ${status.message}`;
      default:
        return "Ternt checks for updates shortly after launch and every 15 minutes, and installs them when you quit.";
    }
  });
</script>

<h2>Ternt</h2>
<div class="card">
  <div class="row">
    <div class="label">
      Version {version}
      <div class="hint">{updateText}</div>
    </div>
    {#if status.state === "ready" || status.state === "failed"}
      <button class="button primary" onclick={() => browser.installUpdate()}>Restart to Update</button>
    {:else}
      <button class="button" disabled={busy} onclick={() => browser.checkForUpdates()}>Check for Updates</button>
    {/if}
  </div>
  <label class="row">
    <div class="label">
      Unload tabs you haven't looked at in 30 minutes
      <div class="hint">
        Saves memory: the tab stays in the sidebar and reloads when you open it. Tabs playing sound or using the
        camera or microphone stay loaded.
      </div>
    </div>
    <input class="unload" type="checkbox" {...{ switch: true }} checked={unloadTabs} onchange={setUnloadTabs} />
  </label>
  <div class="row">
    <div class="label">
      Welcome page
      <div class="hint">The shortcuts and a tour of the sidebar, in a new tab.</div>
    </div>
    <button class="button" onclick={() => browser.menuAction("welcome")}>Open</button>
  </div>
</div>

<h2>Camera and microphone</h2>
<p class="caption">
  A site asks the first time it wants your camera or microphone (a call joining, say). These are the sites you've
  allowed: remove one to have it ask again. Sharing your screen always asks, in macOS's picker.
</p>
<div class="card media">
  {#each mediaSites as site (site)}
    <div class="row site">
      <span class="origin">{site.replace(/^https:\/\//, "")}</span>
      <button class="button" onclick={() => forgetSite(site)}>Remove</button>
    </div>
  {:else}
    <div class="row"><span class="none">None yet.</span></div>
  {/each}
</div>

<style>
  .origin {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .none {
    color: var(--text-muted);
  }
</style>
