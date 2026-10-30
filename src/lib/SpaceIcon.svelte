<script lang="ts">
  import { symbolPath } from "./spaceIcons";
  import { spaceSwatch } from "./spaces";

  // A space's icon: its line symbol in the space's color, its emoji, or,
  // without either, its initial.
  let { icon, name, color, size = 14 }: { icon: string | null; name: string; color: string; size?: number } =
    $props();

  const path = $derived(symbolPath(icon));
  const emoji = $derived(icon && !icon.startsWith("sym:") ? icon : null);
</script>

<span class="space-glyph" style:--size="{size}px" style:color={spaceSwatch(color)} aria-hidden="true">
  {#if path}
    <svg viewBox="0 0 16 16"><path d={path} /></svg>
  {:else if emoji}
    <span class="emoji">{emoji}</span>
  {:else}
    <span class="initial">{name.trim().charAt(0).toUpperCase() || "?"}</span>
  {/if}
</span>

<style>
  .space-glyph {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: var(--size);
    height: var(--size);
    line-height: 1;
  }
  svg {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .emoji {
    font-size: var(--size);
  }
  .initial {
    font-size: calc(var(--size) * 0.85);
    font-weight: 700;
  }
</style>
