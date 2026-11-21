<script lang="ts">
  import { tick } from "svelte";
  import { render } from "./markdown";
  import Presence from "./Presence.svelte";
  import { app, busy, items, send, show, stop, type Presence as Doing } from "./state.svelte";

  // The open thread: what was said, the reply being written, and where to write the next
  // message. With no thread open it is a new conversation, which Tokidoki files itself.
  let draft = $state("");
  let sending = $state(false);
  let scroller = $state<HTMLElement>();
  let input = $state<HTMLTextAreaElement>();

  const learned = $derived(app.memories.filter((memory) => memory.source?.thread === app.current));
  const lines = $derived(items(app.log, learned));
  const about = $derived(app.threads.find((thread) => thread.id === app.current)?.about);
  const now = $derived(app.current ? app.live[app.current] : undefined);
  const answering = $derived(busy(app.log));
  // Between the commit that starts a turn and the first sign of work, it is thinking.
  const doing = $derived<Doing>(now?.doing ?? (answering ? { kind: "thinking" } : null));
  const queued = $derived(app.queued.filter((q) => q.thread === app.current));
  const unanswered = $derived(app.current ? app.unanswered[app.current] : undefined);

  function label(doing: Doing) {
    switch (doing?.kind) {
      case "remembering":
        return "Remembering";
      case "thinking":
        return "Thinking";
      case "tool":
        return `Using ${doing.name}`;
      default:
        return "";
    }
  }

  function greeting() {
    const hour = new Date().getHours();
    return hour < 5 ? "Still up." : hour < 12 ? "Good morning." : hour < 18 ? "Good afternoon." : "Good evening.";
  }

  /** Why an input got no answer, in the user's terms. */
  function explain(reason: string) {
    if (/\b401\b|\b403\b|authentication/i.test(reason))
      return "Anthropic didn't accept the API key. Replace it under API keys, in the sidebar.";
    if (reason.startsWith("model_error")) return `The model couldn't be reached. ${reason.replace(/^model_error:\s*/, "")}`;
    if (reason === "turn_limit") return "Tokidoki stopped after too many steps without an answer.";
    return `No answer: ${reason}`;
  }

  // Follow the conversation down as it grows, unless the user has scrolled up to read.
  let pinned = true;
  function onscroll() {
    if (scroller) pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  }
  $effect(() => {
    void [lines.length, now?.text, doing, queued.length];
    if (pinned) tick().then(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
  });
  $effect(() => {
    void app.current;
    pinned = true;
    input?.focus();
  });

  // The box grows with what is written in it (`field-sizing` does this alone where the
  // system's WebKit has it).
  $effect(() => {
    void draft;
    if (!input || CSS.supports("field-sizing", "content")) return;
    input.style.height = "auto";
    input.style.height = `${input.scrollHeight}px`;
  });

  async function submit() {
    const text = draft.trim();
    if (!text || sending) return;
    sending = true;
    pinned = true;
    if (await send(text)) draft = "";
    sending = false;
    input?.focus();
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      submit();
    }
  }
</script>

<section>
  <header data-tauri-drag-region></header>

  <div class="scroll" bind:this={scroller} {onscroll}>
    <div class="column">
      {#if app.current === null}
        <div class="hello">
          <Presence doing={sending ? { kind: "thinking" } : null} size={56} />
          <h1>{greeting()}</h1>
          <p>
            Say what's on your mind. Tokidoki files it with the conversation it belongs to, or starts a new one, and
            keeps what matters for next time.
          </p>
        </div>
      {/if}

      {#if about}<p class="about selectable">{about}</p>{/if}

      {#each lines as line (line.key)}
        {#if line.kind === "user"}
          <p class="said selectable">{line.text}</p>
        {:else if line.kind === "reply"}
          <!-- eslint-disable-next-line svelte/no-at-html-tags (sanitized in render) -->
          <div class="reply selectable">{@html render(line.text)}</div>
        {:else if line.kind === "recalled"}
          <ul class="recalled">
            {#each line.memories as memory}<li class="selectable">{memory}</li>{/each}
          </ul>
        {:else if line.kind === "learned"}
          <button class="learned" onclick={() => show("memory")} title="Open memory">
            <span>{line.memory.version > 1 ? "Updated what it knows" : "Learned"}</span>
            {line.memory.text}
          </button>
        {:else if line.kind === "tool"}
          <p class="aside">Used {line.name}</p>
        {:else if line.kind === "compacted"}
          <p class="aside rule">Earlier messages are summarized from here</p>
        {/if}
      {/each}

      {#if now?.text}
        <div class="reply selectable">{@html render(now.text)}</div>
      {/if}
      {#if doing && doing.kind !== "writing"}
        <p class="working"><Presence {doing} size={18} /> {label(doing)}</p>
      {/if}
      {#each queued as waiting (waiting.id)}
        <p class="said waiting">{waiting.text} <span>Sends after this answer</span></p>
      {/each}
      {#if unanswered !== undefined && !answering}
        <p class="problem selectable" role="alert">{explain(unanswered)}</p>
      {/if}
    </div>
  </div>

  <form class="composer" onsubmit={(event) => (event.preventDefault(), submit())}>
    {#if app.sendError}<p class="problem selectable" role="alert">{app.sendError}</p>{/if}
    <div class="box">
      <textarea
        bind:this={input}
        bind:value={draft}
        {onkeydown}
        rows="1"
        placeholder={app.current ? "Reply" : "Message Tokidoki"}
        aria-label="Message"
      ></textarea>
      {#if answering}
        <button type="button" class="stop" onclick={stop}>Stop</button>
      {/if}
      <button type="submit" class="send" disabled={!draft.trim() || sending}>Send</button>
    </div>
  </form>
</section>

<style>
  section {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  header {
    height: 38px;
    flex: none;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .column {
    width: min(680px, 100%);
    margin: 0 auto;
    padding: 8px 28px 28px;
  }
  .hello {
    padding-top: 14vh;
  }
  .hello h1 {
    margin: 22px 0 8px;
    font: 500 34px/1.1 var(--voice);
    letter-spacing: -0.02em;
  }
  .hello p {
    margin: 0;
    max-width: 44ch;
    color: var(--muted);
    font-size: 15px;
  }

  /* What the user said: the interface's own face, marked by a rule. */
  .said {
    margin: 30px 0 12px;
    padding-left: 12px;
    border-left: 2px solid var(--indigo);
    font-weight: 550;
    font-size: 15px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .said:first-child {
    margin-top: 6px;
  }
  .waiting {
    color: var(--muted);
    border-left-style: dashed;
  }
  .waiting span {
    display: block;
    font-weight: 400;
    font-size: 12px;
  }

  /* What Tokidoki says: its own voice, in the serif. */
  .reply {
    font: 16.5px/1.62 var(--voice);
    overflow-wrap: anywhere;
  }
  .reply :global(p) {
    margin: 0 0 0.85em;
  }
  .reply :global(:last-child) {
    margin-bottom: 0;
  }
  .reply :global(h1),
  .reply :global(h2),
  .reply :global(h3) {
    margin: 1.3em 0 0.45em;
    font-size: 1.08em;
    font-weight: 650;
    line-height: 1.3;
  }
  .reply :global(ul),
  .reply :global(ol) {
    margin: 0 0 0.85em;
    padding-left: 1.3em;
  }
  .reply :global(li) {
    margin: 0.2em 0;
  }
  .reply :global(a) {
    color: var(--indigo);
  }
  .reply :global(code) {
    font: 0.84em var(--code);
    padding: 0.12em 0.35em;
    border-radius: 4px;
    background: var(--side);
  }
  .reply :global(pre) {
    margin: 0 0 0.85em;
    padding: 12px 14px;
    border-radius: 8px;
    background: var(--side);
    overflow-x: auto;
    line-height: 1.5;
  }
  .reply :global(pre code) {
    padding: 0;
    background: none;
  }
  .reply :global(blockquote) {
    margin: 0 0 0.85em;
    padding-left: 12px;
    border-left: 2px solid var(--line);
    color: var(--muted);
  }
  .reply :global(table) {
    border-collapse: collapse;
    margin: 0 0 0.85em;
    font: 14px/1.4 var(--ui);
  }
  .reply :global(th),
  .reply :global(td) {
    padding: 5px 10px;
    border-bottom: 1px solid var(--line);
    text-align: left;
  }

  /* What it remembered, coming into the conversation. */
  .recalled {
    margin: 0 0 12px;
    padding: 0;
    list-style: none;
    color: var(--plum);
    font-size: 13px;
  }
  .recalled li {
    padding-left: 16px;
    position: relative;
  }
  .recalled li::before {
    content: "";
    position: absolute;
    left: 2px;
    top: 0.55em;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--plum);
  }
  /* What it learned here: the same plum, set apart from what it recalled by its outline. */
  .learned {
    display: block;
    margin: 10px 0 12px;
    padding: 7px 12px;
    border: 1px solid color-mix(in srgb, var(--plum) 45%, transparent);
    border-radius: 9px;
    color: var(--plum);
    font-size: 13px;
    text-align: left;
  }
  .learned span {
    display: block;
    font-weight: 650;
  }
  .learned:hover {
    background: color-mix(in srgb, var(--plum) 8%, transparent);
  }
  /* Tokidoki's own note on what the conversation is about. */
  .about {
    margin: 0 0 22px;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--line);
    color: var(--muted);
    font: 14px/1.5 var(--voice);
  }
  .aside {
    margin: 10px 0;
    color: var(--muted);
    font-size: 13px;
  }
  .rule {
    padding-bottom: 10px;
    border-bottom: 1px solid var(--line);
  }
  .working {
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 12px 0 0;
    color: var(--muted);
    font-size: 13px;
  }
  .problem {
    margin: 12px 0 0;
    color: var(--danger);
    font-size: 13px;
  }

  .composer {
    flex: none;
    width: min(680px, 100%);
    margin: 0 auto;
    padding: 0 28px 22px;
  }
  .composer .problem {
    margin: 0 0 8px;
  }
  .box {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 8px 8px 8px 14px;
    border: 1px solid var(--line);
    border-radius: 14px;
    background: var(--raised);
  }
  .box:focus-within {
    border-color: var(--indigo);
  }
  textarea {
    flex: 1;
    min-width: 0;
    max-height: 40vh;
    padding: 5px 0;
    border: 0;
    outline: 0;
    resize: none;
    background: none;
    font-size: 15px;
    line-height: 1.45;
    field-sizing: content;
  }
  textarea::placeholder {
    color: var(--muted);
  }
  .box button {
    height: 30px;
    padding: 0 13px;
    border-radius: 8px;
    font-weight: 600;
  }
  .send {
    background: var(--indigo);
    color: var(--paper);
  }
  .send:disabled {
    opacity: 0.4;
  }
  .stop {
    background: var(--side);
  }
</style>
