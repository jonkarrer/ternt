<script lang="ts">
  import { browser } from "../lib/browser";
</script>

<h2>Data</h2>
<div class="card">
  <div class="row">
    <div class="label">
      Data folder
      <div class="hint">Your session (spaces, tabs) and history. Profile cookies live with WebKit.</div>
    </div>
    <button class="button" onclick={() => browser.revealDataDir()}>Show in Finder</button>
  </div>
</div>

<h2>Updates</h2>
<div class="card">
  <div class="row">
    <div class="label">
      Signed releases
      <div class="hint">Every update is signed and checked against Ternt's built-in key before it installs.</div>
    </div>
  </div>
</div>
