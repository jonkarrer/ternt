<script lang="ts" module>
  // Origins whose /favicon.ico failed, so we don't refetch on every render.
  const failed = new Set<string>();
</script>

<script lang="ts">
  import { isAppPage } from "./url";

  let { url, loading = false }: { url: string; loading?: boolean } = $props();

  const origin = $derived.by(() => {
    try {
      const u = new URL(url);
      return u.protocol.startsWith("http") ? u.origin : null;
    } catch {
      return null;
    }
  });
  const letter = $derived(origin ? new URL(origin).hostname.replace(/^www\./, "")[0] ?? "" : "");
  let broken = $state(false);
  $effect(() => {
    broken = origin == null || failed.has(origin);
  });
</script>

<span class="favicon" class:loading>
  {#if isAppPage(url)}
    <!-- Ternt's own pages (the welcome page): the app's bolt. -->
    <span class="bolt" aria-hidden="true">
      <svg viewBox="300 120 440 790"><polygon points="612,150 318,572 494,572 404,884 722,428 556,428 656,150" /></svg>
    </span>
  {:else if !broken && origin}
    <img
      src="{origin}/favicon.ico"
      alt=""
      draggable="false"
      onerror={() => {
        failed.add(origin);
        broken = true;
      }}
    />
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
