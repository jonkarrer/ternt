<script lang="ts">
  import type { Presence } from "./state.svelte";

  // Tokidoki itself, on screen: a disc that breathes while it rests, with a mote that
  // circles it while it works, plum while it remembers. The app icon is this mark, still.
  let { doing, size = 22 }: { doing: Presence; size?: number } = $props();
  const kind = $derived(doing?.kind ?? "resting");
</script>

<svg class={kind} width={size} height={size} viewBox="0 0 48 48" aria-hidden="true">
  <circle class="ring" cx="24" cy="24" r="19" />
  <circle class="disc" cx="24" cy="24" r="11" />
  <g class="orbit"><circle class="mote" cx="24" cy="5" r="3.6" /></g>
</svg>

<style>
  svg {
    flex: none;
    overflow: visible;
  }
  .ring {
    fill: none;
    stroke: var(--indigo);
    stroke-width: 1.5;
    opacity: 0.35;
  }
  .disc {
    fill: var(--indigo);
    transform-origin: 24px 24px;
    animation: breathe 5.5s ease-in-out infinite;
  }
  .orbit {
    transform-origin: 24px 24px;
    opacity: 0;
    transition: opacity 0.4s;
  }
  .mote {
    fill: var(--indigo);
  }
  .remembering .orbit,
  .thinking .orbit,
  .tool .orbit,
  .writing .orbit {
    opacity: 1;
    animation: circle 2.4s linear infinite;
  }
  .remembering .mote {
    fill: var(--plum);
  }
  .remembering .orbit {
    animation-duration: 4s;
    animation-direction: reverse;
  }
  .writing .orbit {
    animation-duration: 1.2s;
  }
  .writing .disc {
    animation-duration: 1.6s;
  }
  @keyframes breathe {
    0%,
    100% {
      transform: scale(0.86);
    }
    50% {
      transform: scale(1);
    }
  }
  @keyframes circle {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .disc,
    .orbit {
      animation: none !important;
    }
  }
</style>
