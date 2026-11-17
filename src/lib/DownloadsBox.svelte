<script lang="ts">
  import { fly } from "svelte/transition";
  import { browser, type Download } from "./browser";
  import { formatBytes, formatLeft, formatProgress } from "./bytes";
  import { downloads, holdDownloads, rateOf, watchDownloads } from "./downloads.svelte";

  // The Downloads box over the bottom of the sidebar, like Arc's: each file
  // with its progress while it downloads, then a click opens it. It opens
  // when a download starts; the footer's download button shows or hides it
  // (src/lib/downloads.svelte.ts).
  watchDownloads();
  const list = $derived(downloads.list);
  const finished = $derived(list.some((d) => d.state !== "active"));

  function extension(d: Download) {
    const dot = d.name.lastIndexOf(".");
    return dot > 0 && d.name.length - dot <= 5 ? d.name.slice(dot + 1).toUpperCase() : "";
  }

  function detail(d: Download) {
    switch (d.state) {
      case "active": {
        const rate = rateOf(d.id);
        const left = d.total != null && rate > 0 ? formatLeft((d.total - d.received) / rate) : "";
        // Speed only when the time left can't be known.
        const extra = left || (d.total == null && rate > 0 ? `${formatBytes(rate)}/s` : "");
        return [formatProgress(d.received, d.total), extra].filter(Boolean).join(" · ");
      }
      case "done":
        return d.missing ? "Moved or deleted" : formatBytes(d.total ?? d.received);
      case "failed":
        return "Failed";
      case "canceled":
        return "Canceled";
    }
  }

  function activate(d: Download) {
    if (d.state === "done" && !d.missing) browser.openDownload(d.id);
    else if (d.state === "done") browser.revealDownload(d.id);
    // Try a failed or canceled one again: its new tab closes once it's
    // only a file.
    else if (d.state !== "active") browser.newTab(d.url);
  }
</script>

{#if downloads.open && list.length}
  <section
    class="downloads"
    aria-label="Downloads"
    transition:fly={{ y: 12, duration: 180 }}
    onpointerenter={holdDownloads}
  >
    <header>
      <strong>Downloads</strong>
      {#if finished}
        <button class="clear" onclick={() => browser.clearDownloads()}>Clear</button>
      {/if}
    </header>
    <ul>
      {#each list as d (d.id)}
        <li class={d.state} class:missing={d.missing}>
          <button
            class="file"
            title={d.state === "done" ? (d.missing ? "Show the Downloads folder" : `Open ${d.name}`) : d.state === "active" ? d.name : "Try again"}
            onclick={() => activate(d)}
          >
            <span class="kind" aria-hidden="true">
              {#if extension(d)}{extension(d)}{:else}
                <svg viewBox="0 0 16 16"><path d="M4.5 2.5h4.5l3 3v8h-7.5z M9 2.5v3h3" /></svg>
              {/if}
            </span>
            <span class="text">
              <span class="name">{d.name}</span>
              <span class="detail">{detail(d)}</span>
              {#if d.state === "active"}
                <span
                  class="bar"
                  class:unknown={d.total == null}
                  style:--progress={d.total ? Math.min(1, d.received / d.total) : 0}
                ></span>
              {/if}
            </span>
          </button>
          {#if d.state === "active"}
            <button class="act" title="Cancel" aria-label="Cancel {d.name}" onclick={() => browser.cancelDownload(d.id)}>
              <svg viewBox="0 0 16 16"><path d="m5 5 6 6m0-6-6 6" /></svg>
            </button>
          {:else if d.state === "done" && !d.missing}
            <button class="act" title="Show in Finder" aria-label="Show {d.name} in Finder" onclick={() => browser.revealDownload(d.id)}>
              <svg viewBox="0 0 16 16"><circle cx="7" cy="7" r="3.5" /><path d="m9.6 9.6 3 3" /></svg>
            </button>
          {:else}
            <button class="act" title="Remove from list" aria-label="Remove {d.name} from the list" onclick={() => browser.clearDownloads(d.id)}>
              <svg viewBox="0 0 16 16"><path d="m5 5 6 6m0-6-6 6" /></svg>
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .downloads {
    margin: 8px 0 2px;
    padding: 8px 6px 6px;
    border-radius: 12px;
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--ring),
      0 6px 20px var(--shadow);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 6px 4px;
  }
  strong {
    font-size: 12px;
    font-weight: 600;
  }
  .clear {
    height: 20px;
    padding: 0 8px;
    border-radius: 6px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .clear:hover {
    background: var(--fill-hover);
    color: var(--text);
  }
  ul {
    max-height: 236px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }
  li {
    display: flex;
    position: relative;
    align-items: center;
    border-radius: 8px;
  }
  li:hover {
    background: var(--fill-hover);
  }
  .file {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 9px;
    min-width: 0;
    padding: 6px;
    text-align: left;
  }
  .kind {
    display: grid;
    flex: none;
    place-items: center;
    width: 28px;
    height: 32px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    font-size: 8.5px;
    font-weight: 700;
    letter-spacing: 0.02em;
  }
  .failed .kind,
  .canceled .kind,
  .missing .kind {
    background: var(--fill-strong);
    color: var(--text-muted);
  }
  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .name,
  .detail {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    font-size: 12px;
  }
  .failed .name,
  .canceled .name,
  .missing .name {
    color: var(--text-muted);
  }
  .detail {
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }
  .failed .detail {
    color: #d9534f;
  }
  .bar {
    height: 3px;
    margin-top: 2px;
    border-radius: 2px;
    background: linear-gradient(var(--accent), var(--accent)) no-repeat 0 0 / calc(var(--progress) * 100%) 100%,
      var(--fill-strong);
    transition: background-size 200ms linear;
  }
  /* The server didn't say how big it is. */
  .bar.unknown {
    background: linear-gradient(90deg, transparent, var(--accent), transparent) no-repeat 0 0 / 40% 100%,
      var(--fill-strong);
    animation: sweep 1.2s ease-in-out infinite;
  }
  @keyframes sweep {
    from {
      background-position: -60% 0, 0 0;
    }
    to {
      background-position: 160% 0, 0 0;
    }
  }
  .act {
    display: grid;
    flex: none;
    place-items: center;
    width: 22px;
    height: 22px;
    margin-right: 6px;
    border-radius: 6px;
    opacity: 0;
  }
  li:hover .act,
  .act:focus-visible,
  .active .act {
    opacity: 1;
  }
  .act:hover {
    background: var(--fill-strong);
  }
  svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .kind svg {
    width: 16px;
    height: 16px;
  }
</style>
