<script lang="ts" module>
  import { browser } from "./browser";

  // One lookup per origin at a time; Rust remembers the answers.
  const icons = new Map<string, Promise<string | null>>();

  function iconFor(url: string): Promise<string | null> {
    let origin: string;
    try {
      origin = new URL(url).origin;
    } catch {
      return Promise.resolve(null);
    }
    let icon = icons.get(origin);
    if (!icon) {
      icon = browser.siteIcon(url).catch(() => null);
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
  // tiles; the favicon until it's known, or when the site has none.
  let { url, loading = false }: { url: string; loading?: boolean } = $props();

  let icon = $state<string | null>(null);
  $effect(() => {
    let current = true;
    icon = null;
    if (url.startsWith("http")) iconFor(url).then((i) => current && (icon = i));
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
