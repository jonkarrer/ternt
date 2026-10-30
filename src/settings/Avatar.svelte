<script lang="ts">
  import type { BrowserState, Profile } from "../lib/browser";
  import { spaceSwatch } from "../lib/spaces";

  let { profile, browserState, large = false }: { profile: Profile; browserState: BrowserState; large?: boolean } =
    $props();

  /** The main profile uses the accent; others their first space's color. */
  const background = $derived(
    profile.profile == null
      ? "var(--accent)"
      : spaceSwatch(browserState.spaces.find((s) => s.id === profile.spaces[0])?.color ?? "gray"),
  );
  const initial = $derived(profile.name.trim().charAt(0).toUpperCase() || "?");
</script>

<span class="avatar" class:large style:background>{initial}</span>

<style>
  .avatar {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    font-size: 13px;
    font-weight: 600;
    color: white;
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.12);
  }
  .avatar.large {
    width: 48px;
    height: 48px;
    font-size: 21px;
  }
</style>
