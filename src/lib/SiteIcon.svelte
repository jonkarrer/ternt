<script lang="ts" module>
  import { browser } from "./browser";

  // One lookup per origin at a time; Rust remembers the answers.
  const icons = new Map<string, Promise<string | null>>();

  function iconFor(url: string, tab?: number): Promise<string | null> {
    let origin: string;
    try {
      origin = new URL(url).origin;
    } catch {
      return Promise.resolve(null);
    }
    let icon = icons.get(origin);
    if (!icon) {
      icon = browser.siteIcon(url, tab).catch(() => null);
      icons.set(origin, icon);
      // A site without an icon (or offline) is asked again next time.
      icon.then((found) => found || icons.delete(origin));
    }
    return icon;
  }
</script>

<script lang="ts">
  import Favicon from "./Favicon.svelte";

  // A site's app icon (apple-touch-icon or its largest icon), for favorites'
  // tiles; the favicon until it's known, or when the site has none. `tab`'s
  // live page is asked for its icons, so it's asked again once it loads.
  let { url, tab, loading = false }: { url: string; tab?: number; loading?: boolean } = $props();

  const origin = $derived.by(() => {
    try {
      return url.startsWith("http") ? new URL(url).origin : null;
    } catch {
      return null;
    }
  });
  let icon = $state<string | null>(null);
  $effect(() => {
    origin;
    icon = null;
  });
  $effect(() => {
    if (!origin || loading || icon) return;
    let current = true;
    iconFor(url, tab).then((i) => current && (icon = i));
    return () => (current = false);
  });
</script>

{#if icon}
  <img class="site-icon" class:loading src={icon} alt="" draggable="false" />
{:else}
  <Favicon {url} {loading} />
{/if}

<style>
  .site-icon {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    object-fit: contain;
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
