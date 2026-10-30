<script lang="ts">
  import { onMount } from "svelte";

  // Emoji grid for a space's icon, opened under the space header (space
  // menu → Change Icon…). Any emoji can be typed in the field too.
  let {
    current,
    onpick,
    onclose,
  }: { current: string | null; onpick: (icon: string | null) => void; onclose: () => void } = $props();

  const ICONS = [
    "🏠", "💼", "💻", "🛠️", "🧪", "🚀", "⚡", "🔥",
    "📚", "📝", "📅", "📈", "💰", "🛒", "🎓", "🧠",
    "🎨", "🎵", "🎬", "📷", "🎮", "🧩", "🎯", "🎉",
    "🌍", "✈️", "🏖️", "🌱", "🌙", "☀️", "🌊", "⭐",
    "❤️", "☕", "🍕", "🏋️", "🧘", "🐶", "🐱", "🔒",
  ];

  let panel = $state<HTMLElement>();
  let custom = $state("");

  /** The first emoji (grapheme) typed, so "🏠abc" picks 🏠. */
  function firstGrapheme(text: string): string {
    const segments = new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(text.trim());
    return segments[Symbol.iterator]().next().value?.segment ?? "";
  }

  function submitCustom(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      const icon = firstGrapheme(custom);
      if (icon) onpick(icon);
    }
  }

  onMount(() => {
    // A click anywhere else, or Escape, closes it.
    const outside = (e: PointerEvent) => panel && !panel.contains(e.target as Node) && onclose();
    const escape = (e: KeyboardEvent) => e.key === "Escape" && onclose();
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("keydown", escape);
    };
  });
</script>

<div class="picker" role="dialog" aria-label="Space icon" bind:this={panel}>
  <div class="grid">
    {#each ICONS as icon (icon)}
      <button class="emoji" class:current={icon === current} aria-label={icon} onclick={() => onpick(icon)}>{icon}</button>
    {/each}
  </div>
  <div class="footer">
    <input
      class="custom"
      placeholder="Or type an emoji…"
      maxlength="16"
      bind:value={custom}
      onkeydown={submitCustom}
      aria-label="Custom icon"
    />
    <button class="remove" disabled={!current} onclick={() => onpick(null)}>Remove</button>
  </div>
</div>

<style>
  .picker {
    position: absolute;
    z-index: 10;
    top: 30px;
    left: 4px;
    right: 4px;
    padding: 8px;
    border-radius: 10px;
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--ring),
      0 8px 24px var(--shadow);
    animation: pop 120ms ease-out;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(26px, 1fr));
    gap: 2px;
  }
  .emoji {
    height: 26px;
    border-radius: 6px;
    font-size: 16px;
    line-height: 26px;
  }
  .emoji:hover {
    background: var(--fill-hover);
  }
  .emoji.current {
    background: var(--fill-strong);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .footer {
    display: flex;
    gap: 6px;
    margin-top: 8px;
  }
  .custom {
    flex: 1;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    border: 0;
    border-radius: 6px;
    outline: 0;
    background: var(--fill);
    box-shadow: inset 0 0 0 0.5px var(--ring);
    color: var(--text);
    font: inherit;
    font-size: 12px;
  }
  .custom:focus {
    box-shadow: 0 0 0 1.5px var(--accent);
  }
  .remove {
    padding: 0 8px;
    border-radius: 6px;
    background: var(--fill-strong);
    font-size: 11px;
    font-weight: 500;
  }
  .remove:disabled {
    opacity: 0.4;
  }
</style>
