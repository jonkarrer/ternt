<script lang="ts" module>
  // Icons that failed to load, and when: not refetched on every render, but
  // tried again after a while (the network may have been down).
  const failed = new Map<string, number>();
  const RETRY_AFTER = 60_000;
  const hasFailed = (src: string) => Date.now() - (failed.get(src) ?? -Infinity) < RETRY_AFTER;
</script>

<script lang="ts">
  import { isAppPage } from "./url";

  // `icon` is the one the page names (Tab.favicon); without it, or if it
  // won't load, the site's /favicon.ico, then the site's first letter.
  let { url, icon = null, loading = false }: { url: string; icon?: string | null; loading?: boolean } = $props();

  const origin = $derived.by(() => {
    try {
      const u = new URL(url);
      return u.protocol.startsWith("http") ? u.origin : null;
    } catch {
      return null;
    }
  });
  const letter = $derived(origin ? new URL(origin).hostname.replace(/^www\./, "")[0] ?? "" : "");
  const sources = $derived([icon, origin && `${origin}/favicon.ico`].filter((s): s is string => !!s));
  let src = $state<string | null>(null);
  // Re-picked when the icon changes, and when a load finishes.
  $effect(() => {
    loading;
    src = sources.find((s) => !hasFailed(s)) ?? null;
  });
  function onerror() {
    if (!src) return;
    failed.set(src, Date.now());
    src = sources.find((s) => !hasFailed(s)) ?? null;
  }
</script>

<span class="favicon" class:loading>
  {#if isAppPage(url)}
    <!-- Ternt's own pages (the welcome page): the app's bolt. -->
    <span class="bolt" aria-hidden="true">
      <svg viewBox="300 120 440 790"><polygon points="612,150 318,572 494,572 404,884 722,428 556,428 656,150" /></svg>
    </span>
  {:else if src}
    <img {src} alt="" draggable="false" {onerror} />
  {:else}
    <span class="letter">{letter.toUpperCase()}</span>
  {/if}
</span>

<style>
  .favicon {
    flex: none;
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
    border-radius: 4px;
  }
  img {
    width: 16px;
    height: 16px;
    object-fit: contain;
  }
  .letter {
    width: 16px;
    height: 16px;
    border-radius: 4px;
    display: grid;
    place-items: center;
    font-size: 10px;
    font-weight: 600;
    background: var(--fill-strong);
    color: var(--text-muted);
  }
  .bolt {
    width: 16px;
    height: 16px;
    border-radius: 4px;
    display: grid;
    place-items: center;
    background: linear-gradient(160deg, #2b1360, #0b0718);
  }
  .bolt svg {
    width: 10px;
    height: 12px;
    fill: #d7ecff;
  }
  .loading {
    animation: pulse 1s ease-in-out infinite alternate;
  }
  @keyframes pulse {
    to {
      opacity: 0.35;
    }
  }
</style>
