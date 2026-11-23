<script lang="ts">
  import { onMount, tick } from "svelte";
  import SpaceIcon from "../lib/SpaceIcon.svelte";
  import { browser, type AgentRequest, type AgentSpaceStatus, type AgentStatus, type AgentTagged } from "../lib/browser";
  import type { ThreadSummary } from "./protocol";
  import { apply, emptyView, presenceText, rows, type ThreadView } from "./thread";

  let request = $state<AgentRequest | null>(null);
  /** The agent process; `null` until asked. */
  let status = $state<AgentStatus | null>(null);
  /** How this space's agent runs (its provider, whether it has a key); `null` until asked. */
  let spaceStatus = $state<AgentSpaceStatus | null>(null);
  /** The process just came back from a crash (shown for a moment). */
  let restarted = $state(false);
  let restartedTimer: ReturnType<typeof setTimeout> | undefined;
  let view = $state<ThreadView>(emptyView());
  let threads = $state<ThreadSummary[]>([]);
  let draft = $state("");
  /** Why the last send didn't go through. */
  let error = $state<string | null>(null);
  let input = $state<HTMLTextAreaElement>();
  let scroller = $state<HTMLElement>();

  const space = $derived(request?.space ?? null);
  const list = $derived(rows(view.log));
  const busy = $derived(view.live.doing !== null);
  const presence = $derived(presenceText(view.live.doing));
  const backend = $derived(status?.state === "running" ? status.backend : null);
  const stub = $derived(backend === "stub");
  /** The agent is up but has no key for this space's provider (Settings → Agent), so it can't answer yet. */
  const needsKey = $derived(status?.state === "running" && !stub && spaceStatus !== null && !spaceStatus.hasKey);
  const providerName = $derived(spaceStatus?.provider === "anthropic" ? "Anthropic" : "Fireworks AI");
  /** The process can take an input. */
  const ready = $derived(!status || (status.state === "running" && !needsKey));

  const SUGGESTIONS = ["What's open in this space?", "Summarize what I was doing here", "/tool show me a tool call"];

  onMount(() => {
    const opens = browser.onAgentOpen(show);
    const closes = browser.onAgentClose(() => (request = null));
    const events = browser.onAgentEvent(onEvent);
    const statuses = browser.onAgentStatus(onStatus);
    // Settings changed how this space's agent runs (its provider, say).
    const spaces = browser.onAgentSpace(({ space: id, status: next }) => {
      if (request?.space.id === id) spaceStatus = next;
    });
    // A request made before this page loaded is waiting in Rust; subscribe
    // first so none can slip in between.
    opens.then(() => browser.agentRequest()).then((pending) => pending && show(pending));
    return () => {
      opens.then((off) => off());
      closes.then((off) => off());
      events.then((off) => off());
      statuses.then((off) => off());
      spaces.then((off) => off());
    };
  });

  let showSeq = 0;
  async function show(next: AgentRequest) {
    const seq = ++showSeq;
    const sameSpace = request?.space.id === next.space.id;
    request = next;
    error = null;
    browser.agentStatus().then((now) => (status = now));
    if (!sameSpace) spaceStatus = null;
    askSpace(next.space.id);
    await tick();
    input?.focus();
    if (sameSpace && view.thread) {
      // Reopened on the same space: events while hidden were dropped, so
      // read what the thread gained.
      await catchUp();
      return;
    }
    // Without a key tokidoki can't open the agent, so even this fails: then there is nothing to show.
    threads = await browser.agentThreads(next.space.id).catch(() => [] as ThreadSummary[]);
    if (seq !== showSeq) return;
    await openThread(threads[0]?.id ?? null);
    scrollToEnd(true);
  }

  /** How the space's agent runs: whether it can answer, and on what. */
  function askSpace(id: number) {
    browser
      .agentSpaceStatus(id)
      .then((now) => {
        if (request?.space.id === id) spaceStatus = now;
      })
      .catch(() => {});
  }

  async function openThread(thread: string | null) {
    const spaceId = request?.space.id;
    if (spaceId == null) return;
    view = emptyView(thread);
    error = null;
    if (!thread) return;
    const log = await browser.agentHistory(spaceId, thread, 0);
    if (view.thread !== thread || request?.space.id !== spaceId) return;
    for (const event of log) apply(view, event);
    await catchUp();
    scrollToEnd(true);
  }

  /** Read the checkpoints the thread is missing. */
  async function catchUp() {
    const { thread } = view;
    const spaceId = request?.space.id;
    if (!thread || spaceId == null) return;
    const more = await browser.agentHistory(spaceId, thread, view.log.length);
    if (view.thread !== thread) return;
    for (const event of more) apply(view, event);
  }

  async function refreshThreads() {
    const spaceId = request?.space.id;
    if (spaceId == null) return;
    const next = await browser.agentThreads(spaceId);
    if (request?.space.id === spaceId) threads = next;
  }

  /** The process started, stopped or came back. */
  function onStatus(next: AgentStatus) {
    const before = status;
    status = next;
    if (next.state !== "running") {
      // Whatever it was writing is gone with it.
      view.live = { doing: null, text: "" };
      return;
    }
    if (next.restarted || (before && before.state !== "running")) {
      restarted = next.restarted;
      clearTimeout(restartedTimer);
      restartedTimer = setTimeout(() => (restarted = false), 8000);
      // A new process: ask again whether this space's agent can answer.
      if (request) askSpace(request.space.id);
      reload();
    }
  }

  /** After a restart: the agent may have kept everything (tokidoki) or
      nothing (the stub); show what it has now. */
  async function reload() {
    const spaceId = request?.space.id;
    if (spaceId == null) return;
    const next = await browser.agentThreads(spaceId).catch(() => [] as ThreadSummary[]);
    if (request?.space.id !== spaceId) return;
    threads = next;
    const keep = view.thread && next.some((t) => t.id === view.thread) ? view.thread : (next[0]?.id ?? null);
    await openThread(keep);
  }

  function onEvent({ space: from, event }: AgentTagged) {
    if (from !== request?.space.id) return;
    if (apply(view, event)) catchUp();
    if (event.type === "checkpoint" && event.change.kind === "turn_ended") refreshThreads();
    scrollToEnd();
  }

  async function send(text = draft) {
    text = text.trim();
    if (!text || !request || busy || !ready) return;
    restarted = false;
    const spaceId = request.space.id;
    draft = "";
    error = null;
    try {
      const sent = await browser.agentSend(spaceId, text, view.thread);
      if (request?.space.id !== spaceId) return;
      // A new conversation: its id wasn't known until now. History has what
      // the agent did meanwhile; streamed text picks up from the next delta.
      if (sent.thread !== view.thread) await openThread(sent.thread);
      refreshThreads();
    } catch (e) {
      error = String(e);
      draft = text;
    }
  }

  function stop() {
    if (request && view.thread) browser.agentAbort(request.space.id, view.thread);
  }

  /** Rust hides the panel and says so (`agent:close`), the same as for ⌘J. */
  function close() {
    return browser.closeAgent();
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey && !event.altKey && !event.isComposing) {
      event.preventDefault();
      send();
    } else if (event.key === "Escape") {
      event.preventDefault();
      close();
    }
  }

  /** Keep the newest message in view, unless the reader scrolled up. */
  function scrollToEnd(force = false) {
    const el = scroller;
    if (!el) return;
    const nearEnd = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    if (force || nearEnd) requestAnimationFrame(() => el.scrollTo({ top: el.scrollHeight }));
  }

  /** The textarea grows with the draft, up to a few lines. */
  $effect(() => {
    void draft;
    const el = input;
    if (!el) return;
    el.style.height = "0";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  });

  function pretty(value: unknown) {
    return typeof value === "string" ? value : JSON.stringify(value, null, 2);
  }
</script>

{#if request && space}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" style:--content-left="{request.contentLeft}px" onclick={(e) => e.target === e.currentTarget && close()}>
    <div class="panel" role="dialog" aria-label="Ask {space.name}">
      <header>
        <SpaceIcon icon={space.icon} name={space.name} color={space.color} size={16} />
        <h1>
          <span class="who">Tokidoki</span>
          <span class="dot">·</span>
          <span class="space">{space.name}</span>
        </h1>
        {#if stub}
          <span class="badge" title="A stand-in answers until tokidoki is connected. Data folder: agents/{request.folder}">stub</span>
        {/if}
        <span class="grow"></span>
        {#if threads.length > 1 || (threads.length === 1 && !view.thread)}
          <select
            class="threads"
            aria-label="Conversation"
            value={view.thread ?? ""}
            onchange={(e) => openThread((e.currentTarget as HTMLSelectElement).value || null)}
          >
            <option value="">New conversation</option>
            {#each threads as thread (thread.id)}
              <option value={thread.id}>{thread.about ?? thread.title}</option>
            {/each}
          </select>
        {/if}
        <button class="tool" title="New conversation" aria-label="New conversation" disabled={!view.thread} onclick={() => openThread(null)}>
          <svg viewBox="0 0 16 16"><path d="M8 3.5v9M3.5 8h9" /></svg>
        </button>
        <button class="tool" title="Close (esc)" aria-label="Close" onclick={close}>
          <svg viewBox="0 0 16 16"><path d="m4.5 4.5 7 7m0-7-7 7" /></svg>
        </button>
      </header>

      <div class="transcript" bind:this={scroller}>
        {#if !list.length && !busy}
          <div class="empty">
            <p>Ask the <b>{space.name}</b> space's agent anything. Every space has its own agent, with its own memory of what you did there.</p>
            <div class="suggestions">
              {#each SUGGESTIONS as text}
                <button class="chip" onclick={() => send(text)}>{text}</button>
              {/each}
            </div>
          </div>
        {/if}
        {#each list as row (row.key)}
          {#if row.kind === "user"}
            <div class="row user" data-row="user"><div class="bubble">{row.text}</div></div>
          {:else if row.kind === "assistant"}
            <div class="row assistant" data-row="assistant"><div class="text">{row.text}</div></div>
          {:else if row.kind === "tool"}
            <details class="row tool" data-row="tool">
              <summary>
                <svg viewBox="0 0 16 16"><path d="M9.5 2.5 6 8h4l-3.5 5.5" /></svg>
                <span class="name">{row.name}</span>
                <span class="state" class:error={row.isError}>{row.result == null ? "running" : row.isError ? "failed" : "done"}</span>
              </summary>
              {#if row.input != null}<pre class="io">{pretty(row.input)}</pre>{/if}
              {#if row.result != null}<pre class="io result" class:error={row.isError}>{row.result}</pre>{/if}
            </details>
          {:else}
            <div class="row note" data-row="note">{row.text}</div>
          {/if}
        {/each}
        {#if view.live.text}
          <div class="row assistant live" data-row="live"><div class="text">{view.live.text}<span class="caret"></span></div></div>
        {/if}
        {#if presence}
          <div class="presence" aria-live="polite"><span class="pulse"></span>{presence}</div>
        {/if}
        {#if view.unanswered}
          <div class="unanswered" role="alert">No answer: {view.unanswered}</div>
        {/if}
        {#if error}
          <div class="unanswered" role="alert">{error}</div>
        {/if}
      </div>

      {#if needsKey}
        <div class="status keys" data-state="keys" role="status">
          <span>Tokidoki needs your {providerName} API key to answer. Add it in Settings → Agent.</span>
          <button class="restart" onclick={() => browser.menuAction("settings")}>Open Settings</button>
        </div>
      {/if}
      {#if status && status.state !== "running"}
        <div class="status" class:down={status.state === "down"} data-state={status.state} role="status">
          <span class="pulse" class:still={status.state === "down"}></span>
          {#if status.state === "starting"}
            Starting the agent…
          {:else if status.state === "restarting" && status.asked}
            Restarting the agent…
          {:else if status.state === "restarting"}
            The agent stopped; starting it again{status.attempt > 1 ? ` (try ${status.attempt})` : ""}…
          {:else}
            {status.reason}
            <button class="restart" onclick={() => browser.agentRestart()}>Restart</button>
          {/if}
        </div>
      {:else if restarted}
        <div class="status" data-state="restarted" role="status">The agent crashed and was restarted. This is what it kept.</div>
      {/if}
      <div class="composer">
        <textarea
          bind:this={input}
          bind:value={draft}
          {onkeydown}
          rows="1"
          placeholder={ready ? `Ask ${space.name}…` : needsKey ? "Add your API key in Settings → Agent" : "Waiting for the agent…"}
          disabled={!ready}
          spellcheck="true"
          aria-label="Message to the {space.name} space's agent"
        ></textarea>
        {#if busy}
          <button class="stop" onclick={stop} title="Stop">
            <svg viewBox="0 0 16 16"><rect x="4" y="4" width="8" height="8" rx="1.5" /></svg>
            Stop
          </button>
        {:else}
          <button class="send" onclick={() => send()} disabled={!draft.trim() || !ready} aria-label="Send (↵)">
            <svg viewBox="0 0 16 16"><path d="M8 13V3m0 0L3.5 7.5M8 3l4.5 4.5" /></svg>
          </button>
        {/if}
      </div>
      <footer>
        <span><kbd>↵</kbd> send</span>
        <span><kbd>⇧↵</kbd> new line</span>
        <span><kbd>esc</kbd> or <kbd>⌘J</kbd> close</span>
        {#if stub}<span><kbd>/tool</kbd> <kbd>/error</kbd> try the stub</span>{/if}
        {#if !stub && backend}<button type="button" class="link" onclick={() => browser.menuAction("settings")}>Settings</button>{/if}
      </footer>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.2);
    animation: fade 120ms ease-out;
  }
  .panel {
    position: absolute;
    top: 8vh;
    /* Centered over the content pane (right of the sidebar), not the window. */
    left: calc(var(--content-left) + (100vw - var(--content-left) - var(--inset)) / 2);
    width: min(760px, calc(100vw - var(--content-left) - var(--inset) - 48px));
    max-height: 80vh;
    translate: -50% 0;
    display: flex;
    flex-direction: column;
    border-radius: 14px;
    background: var(--surface-raised);
    box-shadow:
      0 0 0 0.5px var(--ring),
      0 24px 64px rgb(0 0 0 / 0.3);
    overflow: hidden;
    animation: rise 140ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 44px;
    padding: 0 10px 0 16px;
    border-bottom: 0.5px solid var(--ring);
  }
  h1 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
  }
  .who {
    color: var(--text-muted);
  }
  .dot {
    margin: 0 4px;
    color: var(--text-muted);
  }
  .badge {
    padding: 1px 6px;
    border-radius: 5px;
    background: color-mix(in srgb, #e5a50a 22%, transparent);
    color: color-mix(in srgb, #e5a50a 70%, var(--text));
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .grow {
    flex: 1;
  }
  .threads {
    max-width: 220px;
    height: 26px;
    padding: 0 6px;
    border: 0;
    border-radius: 6px;
    background: var(--fill);
    color: var(--text);
    font: inherit;
    font-size: 12px;
  }
  .tool {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 7px;
    color: var(--text-muted);
  }
  .tool:hover:not(:disabled) {
    background: var(--fill-hover);
    color: var(--text);
  }
  .tool:disabled {
    opacity: 0.35;
  }
  svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .transcript {
    flex: 1;
    min-height: 120px;
    overflow-y: auto;
    padding: 14px 16px 6px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: 14px;
    line-height: 1.45;
    user-select: text;
    -webkit-user-select: text;
  }
  .empty {
    margin: auto 0;
    padding: 24px 8px 16px;
    color: var(--text-muted);
    text-align: center;
  }
  .empty p {
    max-width: 440px;
    margin: 0 auto 16px;
  }
  .empty b {
    color: var(--text);
    font-weight: 600;
  }
  .suggestions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
  }
  .chip {
    padding: 5px 11px;
    border-radius: 999px;
    background: var(--fill);
    color: var(--text);
    font-size: 12px;
  }
  .chip:hover {
    background: var(--fill-strong);
  }
  .row {
    display: flex;
  }
  .row.user {
    justify-content: flex-end;
  }
  .bubble {
    max-width: 78%;
    padding: 8px 12px;
    border-radius: 14px 14px 4px 14px;
    background: var(--accent);
    color: white;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .text {
    max-width: 92%;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .caret {
    display: inline-block;
    width: 2px;
    height: 1em;
    margin-left: 1px;
    vertical-align: -0.15em;
    background: var(--text);
    animation: blink 1s steps(2) infinite;
  }
  .tool {
    display: block;
  }
  details.tool {
    width: auto;
    height: auto;
    max-width: 92%;
    border-radius: 9px;
    background: var(--fill);
    color: var(--text);
    font-size: 12px;
  }
  details.tool summary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    list-style: none;
  }
  details.tool summary::-webkit-details-marker {
    display: none;
  }
  details.tool svg {
    width: 13px;
    height: 13px;
    color: var(--text-muted);
  }
  .name {
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: 11.5px;
  }
  .state {
    color: var(--text-muted);
  }
  .state.error {
    color: #e0443e;
  }
  .io {
    margin: 0 8px 8px;
    padding: 8px;
    max-height: 160px;
    overflow: auto;
    border-radius: 6px;
    background: var(--surface);
    font: 11.5px ui-monospace, "SF Mono", Menlo, monospace;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .io.error {
    color: #e0443e;
  }
  .note {
    justify-content: center;
    color: var(--text-muted);
    font-size: 11.5px;
  }
  .presence {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--text-muted);
    font-size: 12px;
  }
  .pulse {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.1s ease-in-out infinite;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 16px 8px;
    padding: 7px 10px;
    border-radius: 8px;
    background: var(--fill);
    color: var(--text-muted);
    font-size: 12.5px;
  }
  .status.down {
    background: color-mix(in srgb, #e0443e 12%, transparent);
    color: #c9302c;
  }
  .status .pulse.still {
    background: #e0443e;
    animation: none;
  }
  .restart {
    margin-left: auto;
    padding: 3px 9px;
    border-radius: 6px;
    background: var(--accent);
    color: white;
    font-size: 12px;
    font-weight: 600;
  }
  .status.keys span {
    flex: 1;
  }
  .link {
    padding: 0;
    background: none;
    color: var(--text-muted);
    font: inherit;
    font-size: 12px;
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
  }
  .link:hover {
    color: var(--text);
  }
  .unanswered {
    padding: 7px 10px;
    border-radius: 8px;
    background: color-mix(in srgb, #e0443e 12%, transparent);
    color: #c9302c;
    font-size: 12.5px;
  }

  .composer {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 8px 10px 8px 16px;
    border-top: 0.5px solid var(--ring);
  }
  textarea {
    flex: 1;
    min-height: 36px;
    max-height: 160px;
    padding: 8px 0;
    border: 0;
    outline: 0;
    resize: none;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: 15px;
    line-height: 1.35;
    user-select: text;
    -webkit-user-select: text;
  }
  textarea::placeholder {
    color: var(--text-muted);
  }
  .send,
  .stop {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 30px;
    padding: 0 9px;
    border-radius: 9px;
    background: var(--accent);
    color: white;
    font-size: 12px;
    font-weight: 600;
  }
  .send:disabled {
    background: var(--fill-strong);
    color: var(--text-muted);
  }
  .stop {
    background: var(--fill-strong);
    color: var(--text);
  }
  .stop svg rect {
    fill: currentColor;
    stroke: none;
  }
  footer {
    display: flex;
    gap: 14px;
    padding: 7px 14px;
    border-top: 0.5px solid var(--ring);
    font-size: 11px;
    color: var(--text-muted);
  }
  footer span {
    display: flex;
    align-items: center;
    gap: 3px;
  }
  kbd {
    min-width: 18px;
    padding: 1px 5px;
    border-radius: 5px;
    background: var(--fill-strong);
    font: inherit;
    font-size: 11px;
    text-align: center;
    color: var(--text-muted);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      scale: 0.98;
    }
  }
  @keyframes blink {
    to {
      opacity: 0;
    }
  }
  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }
</style>
