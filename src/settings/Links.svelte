<script lang="ts">
  import { onMount } from "svelte";
  import { browser, type Prefs } from "../lib/browser";

  let prefs = $state<Prefs | null>(null);
  let isDefault = $state<boolean | null>(null);
  let asked = $state(false);

  async function refreshDefault() {
    isDefault = (await browser.defaultBrowser()).isDefault;
  }

  onMount(() => {
    browser.getPrefs().then((p) => (prefs = p));
    refreshDefault();
    // macOS confirms in a dialog of its own: check again when we're back.
    const onFocus = () => refreshDefault();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });

  function setLinks(externalLinks: Prefs["externalLinks"]) {
    if (!prefs) return;
    prefs = { ...prefs, externalLinks };
    browser.updatePrefs({ externalLinks });
  }

  async function makeDefault() {
    asked = true;
    await browser.makeDefaultBrowser();
    setTimeout(refreshDefault, 500);
  }
</script>

<h2>Default browser</h2>
<div class="card">
  <div class="row">
    <div class="label">
      {#if isDefault}
        Ternt is your default browser
        <div class="hint">Links you click in other apps open in Ternt.</div>
      {:else}
        Ternt isn't your default browser
        <div class="hint">
          {asked ? "Confirm in the dialog macOS shows, or pick Ternt in System Settings → Desktop & Dock." : "Make it the default to open links from other apps here."}
        </div>
      {/if}
    </div>
    {#if isDefault === false}
      <button class="button primary" onclick={makeDefault}>Make Default</button>
    {/if}
  </div>
</div>

<h2>Links from other apps</h2>
<div class="card" role="radiogroup" aria-label="Links from other apps">
  <label class="row choice">
    <input type="radio" name="links" checked={prefs?.externalLinks === "little"} onchange={() => setLinks("little")} />
    <div class="label">
      Little Ternt
      <div class="hint">A small window of their own: read, close, or move into a tab with Open in Ternt.</div>
    </div>
  </label>
  <label class="row choice">
    <input type="radio" name="links" checked={prefs?.externalLinks === "tab"} onchange={() => setLinks("tab")} />
    <div class="label">
      New tab
      <div class="hint">A new tab in the space you're in.</div>
    </div>
  </label>
</div>

<h2>Links inside Ternt</h2>
<div class="card">
  <div class="row">
    <div class="label">
      New-window links open as tabs in the current space
      <div class="hint">Sign-in popups open in a window that shares the space's profile.</div>
    </div>
  </div>
</div>

<style>
  .choice {
    cursor: default;
  }
  input[type="radio"] {
    flex: none;
    margin: 0;
    accent-color: var(--accent);
  }
</style>
