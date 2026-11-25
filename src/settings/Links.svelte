<script lang="ts">
  import { onMount } from "svelte";
  import { browser, type LinkRule, type Prefs, type Space } from "../lib/browser";
  import DefaultBrowser from "./DefaultBrowser.svelte";

  let { spaces }: { spaces: Space[] } = $props();

  let prefs = $state<Prefs | null>(null);
  /** The rules being edited (saved as they change). */
  let rules = $state<LinkRule[]>([]);
  /** Why each rule's pattern is no good, by index. */
  let problems = $state<(string | null)[]>([]);
  onMount(() => {
    browser.getPrefs().then((p) => {
      prefs = p;
      rules = p.linkRules.map((r) => ({ ...r }));
    });
  });

  function setLinks(externalLinks: Prefs["externalLinks"]) {
    if (!prefs) return;
    prefs = { ...prefs, externalLinks };
    browser.updatePrefs({ externalLinks });
  }

  // --- Link rules ------------------------------------------------------------

  async function save() {
    problems = await Promise.all(rules.map((r) => (r.pattern.trim() ? browser.checkLinkRule(r.kind, r.pattern) : null)));
    // Blank rules stay while they're edited, but aren't saved.
    await browser.updatePrefs({ linkRules: $state.snapshot(rules).filter((r) => r.pattern.trim()) });
    test();
  }

  function addRule() {
    rules.push({ kind: "host", pattern: "", space: spaces[0]?.id ?? 0 });
    problems.push(null);
    // Focus the new rule's pattern.
    queueMicrotask(() => [...document.querySelectorAll<HTMLInputElement>(".rule .pattern")].at(-1)?.focus());
  }

  function removeRule(index: number) {
    rules.splice(index, 1);
    save();
  }

  function moveUp(index: number) {
    if (index === 0) return;
    [rules[index - 1], rules[index]] = [rules[index], rules[index - 1]];
    save();
  }

  const spaceName = (id: number) => spaces.find((s) => s.id === id)?.name;

  /** The link in the tester, and where it goes. */
  let sample = $state("");
  let result = $state<{ rule: number; space: number } | null>(null);
  async function test() {
    result = sample.trim() ? await browser.routeLink(sample) : null;
  }
  const fallback = $derived(prefs?.externalLinks === "tab" ? "a new tab in the space you're in" : "Little Ternt");
</script>

<DefaultBrowser />

<h2>Links from other apps</h2>
<div class="card" role="radiogroup" aria-label="Links from other apps">
  <label class="row choice">
    <input type="radio" name="links" checked={prefs?.externalLinks === "little"} onchange={() => setLinks("little")} />
    <div class="label">
      Little Ternt
      <div class="hint">A small window of their own: read, close, or move into a tab with Open in Ternt.</div>
    </div>
  </label>
  <label class="row choice">
    <input type="radio" name="links" checked={prefs?.externalLinks === "tab"} onchange={() => setLinks("tab")} />
    <div class="label">
      New tab
      <div class="hint">A new tab in the space you're in.</div>
    </div>
  </label>
</div>

<h2>Link rules</h2>
<p class="caption">
  A link from another app that matches a rule opens as a tab in that rule's space; the rest go to {fallback}. The first
  rule that matches wins. A host matches its subdomains too, and with a path (<code>github.com/acme</code>) only the
  pages under it; a regex is tried against the whole URL.
</p>
<div class="card rules">
  {#each rules as rule, i (i)}
    <div class="row rule">
      <select class="kind" aria-label="Match by" bind:value={rule.kind} onchange={save}>
        <option value="host">Host</option>
        <option value="regex">Regex</option>
      </select>
      <div class="pattern-field">
        <input
          class="field pattern"
          class:regex={rule.kind === "regex"}
          class:invalid={problems[i]}
          aria-label="Pattern"
          placeholder={rule.kind === "host" ? "github.com" : "^https://docs\\.google\\.com/"}
          spellcheck="false"
          autocomplete="off"
          bind:value={rule.pattern}
          onchange={save}
        />
        {#if problems[i]}<div class="problem">{problems[i]}</div>{/if}
      </div>
      <span class="arrow" aria-hidden="true">→</span>
      <select class="space" aria-label="Space" bind:value={rule.space} onchange={save}>
        {#if !spaceName(rule.space)}<option value={rule.space}>Deleted space</option>{/if}
        {#each spaces as space (space.id)}
          <option value={space.id}>{space.name}</option>
        {/each}
      </select>
      <button class="icon" title="Move up" aria-label="Move rule up" disabled={i === 0} onclick={() => moveUp(i)}>
        <svg viewBox="0 0 16 16"><path d="M8 12.5v-9M4.5 7 8 3.5 11.5 7" /></svg>
      </button>
      <button class="icon" title="Remove" aria-label="Remove rule" onclick={() => removeRule(i)}>
        <svg viewBox="0 0 16 16"><path d="m5 5 6 6m0-6-6 6" /></svg>
      </button>
    </div>
  {/each}
  <div class="row">
    <button class="button add" onclick={addRule}>Add Rule</button>
  </div>
  <div class="row tester">
    <input
      class="field sample"
      aria-label="Test a link"
      placeholder="Test a link: paste a URL to see where it goes"
      spellcheck="false"
      autocomplete="off"
      bind:value={sample}
      oninput={test}
    />
    {#if sample.trim()}
      <span class="verdict" class:matched={result}>
        {#if result}
          {spaceName(result.space)} <span class="which">(rule {result.rule + 1})</span>
        {:else}
          No rule: {fallback}
        {/if}
      </span>
    {/if}
  </div>
</div>

<h2>Links inside Ternt</h2>
<div class="card">
  <div class="row">
    <div class="label">
      New-window links open as tabs in the current space
      <div class="hint">Sign-in popups open in a window that shares the space's profile.</div>
    </div>
  </div>
</div>

<style>
  .choice {
    cursor: default;
  }
  code {
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: 11.5px;
  }
  .rule {
    gap: 8px;
  }
  .kind {
    width: 84px;
  }
  .space {
    width: 130px;
  }
  .pattern-field {
    flex: 1;
    min-width: 0;
  }
  .pattern,
  .sample {
    width: 100%;
  }
  .pattern.regex {
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: 12px;
  }
  .pattern.invalid {
    box-shadow: 0 0 0 1.5px var(--danger);
  }
  .problem {
    margin-top: 4px;
    font-size: 12px;
    color: var(--danger);
  }
  .arrow {
    color: var(--text-muted);
  }
  .icon {
    display: grid;
    flex: none;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 6px;
    color: var(--text-muted);
  }
  .icon:hover:not(:disabled) {
    background: var(--fill-hover);
    color: var(--text);
  }
  .icon:disabled {
    opacity: 0.3;
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
  .sample {
    flex: 1;
    min-width: 0;
  }
  .verdict {
    flex: none;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--text-muted);
  }
  .verdict.matched {
    color: var(--accent);
    font-weight: 600;
  }
  .which {
    font-weight: 400;
    color: var(--text-muted);
  }
</style>
