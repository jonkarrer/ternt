// What the window shows, kept in step with the core by its events.
//
// A thread on screen is its checkpoints (`log`), in order with no gaps: everything shown is
// derived from them, so what is on screen is what is stored. The one exception is a reply
// being written (`live`), which the reply's own checkpoint replaces when it arrives.
import {
  core,
  type Checkpoint,
  type CoreEvent,
  type Doing,
  type Memory,
  type MemoryChange,
  type Status,
  type Submission,
  type ThreadSummary,
} from "./core";

/** What Tokidoki is doing on a thread right now. `writing` is a reply arriving. */
export type Presence = Doing | { kind: "writing" } | null;

export const app = $state({
  status: null as Status | null,
  /** What the main area shows. */
  view: "conversation" as "conversation" | "memory" | "keys",
  threads: [] as ThreadSummary[],
  /** Everything Tokidoki remembers, newest first. */
  memories: [] as Memory[],
  /** Memories learned or revised since the memory view was last open. */
  unseen: 0,
  /** The open thread; `null` is a new conversation, which the router places. */
  current: null as string | null,
  log: [] as Checkpoint[],
  /** Per thread: what it's doing, and the reply so far. */
  live: {} as Record<string, { doing: Presence; text: string }>,
  /** Inputs waiting for the answer under way. */
  queued: [] as Submission[],
  /** Per thread: why its last input got no answer. */
  unanswered: {} as Record<string, string>,
  /** Why the last send didn't go through. */
  sendError: null as string | null,
});

let listening = false;

/** Asks the core where things stand (it opens if it can), and follows it from then on. */
export async function start() {
  if (!listening) {
    listening = true;
    await core.onEvent(onEvent);
  }
  app.status = await core.status();
  if (app.status.state === "ready") await Promise.all([loadThreads(), loadMemories()]);
}

export async function loadMemories() {
  app.memories = await core.memories();
}

export function show(view: typeof app.view) {
  app.view = view;
  if (view === "memory") app.unseen = 0;
}

export async function loadThreads() {
  app.threads = await core.threads();
}

export async function openThread(thread: string | null) {
  app.view = "conversation";
  app.current = thread;
  app.log = [];
  app.sendError = null;
  if (!thread) return;
  const log = await core.history(thread, 0);
  if (app.current !== thread) return;
  app.log = log;
  // A checkpoint that arrived while that was read may not be in it.
  await catchUp();
}

/** Reads the checkpoints the open thread is missing. */
async function catchUp() {
  const thread = app.current;
  if (!thread) return;
  const more = await core.history(thread, app.log.length);
  // Events kept arriving meanwhile: take only what still comes next.
  if (app.current !== thread) return;
  for (const checkpoint of more) if (checkpoint.seq === app.log.length) app.log.push(checkpoint);
}

export async function send(text: string) {
  app.sendError = null;
  try {
    const sent = await core.send(text, app.current);
    delete app.unanswered[sent.thread];
    if (sent.thread !== app.current) await openThread(sent.thread);
    await loadThreads();
    return true;
  } catch (error) {
    app.sendError = String(error);
    return false;
  }
}

export function stop() {
  if (app.current) core.abort(app.current);
}

function live(thread: string) {
  return (app.live[thread] ??= { doing: null, text: "" });
}

function onEvent(event: CoreEvent) {
  switch (event.type) {
    case "checkpoint": {
      const { thread, change } = event;
      if (change.kind === "message" && change.role === "assistant") live(thread).text = "";
      if (change.kind === "turn_ended") {
        app.live[thread] = { doing: null, text: "" };
        loadThreads();
      }
      if (thread !== app.current) break;
      if (event.seq === app.log.length) app.log.push(event);
      else if (event.seq > app.log.length) catchUp();
      break;
    }
    case "activity": {
      const now = live(event.thread);
      now.doing = event.doing;
      // A new request: whatever an earlier attempt wrote is gone.
      if (event.doing.kind === "thinking") now.text = "";
      break;
    }
    case "delta": {
      if (event.delta.kind !== "text") break;
      const now = live(event.thread);
      now.doing = { kind: "writing" };
      now.text += event.delta.text;
      break;
    }
    case "submission": {
      const { submission } = event;
      app.queued = app.queued.filter((queued) => queued.id !== submission.id);
      if (submission.status === "queued") app.queued.push(submission);
      // Stopping an answer is the user's doing, not a failure to report.
      if (submission.status === "unanswered" && submission.reason !== "aborted")
        app.unanswered[submission.thread] = submission.reason ?? "";
      break;
    }
    case "memory":
      onMemory(event.change);
      break;
    case "lagged":
      catchUp();
      loadThreads();
      loadMemories();
      break;
  }
}

/** Keeps the list in step with the store: each memory once, newest first. */
function onMemory(change: MemoryChange) {
  switch (change.kind) {
    case "learned":
    case "revised": {
      const gone = change.kind === "revised" ? change.replaces : change.memory.id;
      app.memories = [change.memory, ...app.memories.filter((m) => m.id !== gone && m.id !== change.memory.id)];
      if (app.view !== "memory") app.unseen += 1;
      break;
    }
    case "forgotten":
      app.memories = app.memories.filter((m) => !change.ids.includes(m.id));
      break;
    case "described": {
      const thread = app.threads.find((t) => t.id === change.thread);
      if (thread) thread.about = change.about;
      break;
    }
  }
}

/** A line of the conversation, as shown. */
export type Item =
  | { kind: "user"; key: string; text: string }
  | { kind: "reply"; key: string; text: string }
  | { kind: "tool"; key: string; name: string }
  | { kind: "recalled"; key: string; memories: string[] }
  | { kind: "learned"; key: string; memory: Memory }
  | { kind: "compacted"; key: string };

/** The conversation a thread's checkpoints make, with what was learned in it (`learned`:
    memories that came from this thread) where it was learned. */
export function items(log: Checkpoint[], learned: Memory[]): Item[] {
  const out: Item[] = [];
  const at = (seq: number) => learned.filter((memory) => memory.source?.seq === seq);
  for (const { seq: n, change } of log) {
    const seq = String(n);
    if (change.kind === "recalled" && change.memories.length) {
      out.push({ kind: "recalled", key: seq, memories: change.memories });
    } else if (change.kind === "compacted") {
      out.push({ kind: "compacted", key: seq });
    } else if (change.kind === "message") {
      const text = change.content
        .flatMap((part) => (part.type === "text" ? [part.text] : []))
        .join("\n\n")
        .trim();
      if (text) out.push({ kind: change.role === "user" ? "user" : "reply", key: seq, text });
      for (const part of change.content)
        if (part.type === "tool_call") out.push({ kind: "tool", key: `${seq}-${part.id}`, name: part.name });
    }
    for (const memory of at(n)) out.push({ kind: "learned", key: `m${memory.id}`, memory });
  }
  return out;
}

/** Whether an answer is under way on the open thread: its last turn hasn't ended. */
export function busy(log: Checkpoint[]): boolean {
  for (let i = log.length - 1; i >= 0; i--) {
    const kind = log[i].change.kind;
    if (kind === "turn_ended") return false;
    if (kind === "turn_started") return true;
  }
  return false;
}
