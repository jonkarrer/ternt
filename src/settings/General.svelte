<script lang="ts">
  import { onMount } from "svelte";
  import { browser, type UpdateStatus } from "../lib/browser";

  let version = $state("");
  let status = $state<UpdateStatus>({ state: "idle" });

  onMount(() => {
    browser.appVersion().then((v) => (version = v));
    const unlisten = browser.onUpdate((next) => (status = next));
    unlisten.then(() => browser.updateStatus()).then((s) => (status = s));
    return () => unlisten.then((off) => off());
  });

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
  <div class="row">
    <div class="label">
      Welcome page
      <div class="hint">The shortcuts and a tour of the sidebar, in a new tab.</div>
    </div>
    <button class="button" onclick={() => browser.menuAction("welcome")}>Open</button>
  </div>
</div>
