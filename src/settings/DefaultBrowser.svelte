<script lang="ts">
  import { onMount } from "svelte";
  import { browser } from "../lib/browser";

  // Whether Ternt opens links from other apps, with a button to ask macOS.
  // Shown on Account (the first page) and Links.
  let isDefault = $state<boolean | null>(null);
  let asked = $state(false);

  async function refresh() {
    isDefault = (await browser.defaultBrowser()).isDefault;
  }

  onMount(() => {
    refresh();
    // macOS confirms in a dialog of its own: check again when we're back.
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });

  async function makeDefault() {
    asked = true;
    await browser.makeDefaultBrowser();
    setTimeout(refresh, 500);
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
      <button class="button primary" onclick={makeDefault}>Make Default Browser</button>
    {/if}
  </div>
</div>
