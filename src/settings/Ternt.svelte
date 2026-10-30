<script lang="ts">
  import type { BrowserState } from "../lib/browser";
  import ProfileDetail from "./ProfileDetail.svelte";

  // The main profile: the default cookie jar, for every space without its own.
  let { browserState }: { browserState: BrowserState } = $props();
</script>

{#if browserState.profiles[0]}
  <ProfileDetail {browserState} profile={browserState.profiles[0]} />
{/if}
