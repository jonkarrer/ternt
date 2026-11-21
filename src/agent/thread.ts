// A conversation on screen, kept in step with the agent by its events. The
// same model as tokidoki's own page (app/src/lib/state.svelte.ts): a thread
// is its checkpoints in order with no gaps, plus the reply being written.
import type { Doing, Event, JsonValue, Submission } from "./protocol";

export type Checkpoint = Extract<Event, { type: "checkpoint" }>;

/** What the agent is doing on the thread right now; `writing` is a reply arriving. */
export type Presence = Doing | { kind: "writing" } | null;

export interface ThreadView {
  /** `null`: a new conversation, placed by the agent on the first send. */
  thread: string | null;
  log: Checkpoint[];
  live: { doing: Presence; text: string };
  /** Why the last input got no answer. */
  unanswered: string | null;
}

export function emptyView(thread: string | null = null): ThreadView {
  return { thread, log: [], live: { doing: null, text: "" }, unanswered: null };
}

/**
 * Apply one event to the view (events for other threads are ignored).
 * Returns `true` when the log has a gap and `history` must be read from
 * `log.length` (also on `lagged`).
 */
export function apply(view: ThreadView, event: Event): boolean {
  switch (event.type) {
    case "checkpoint": {
      if (event.thread !== view.thread) return false;
      const { change } = event;
      if (change.kind === "message" && change.role === "assistant") view.live.text = "";
      if (change.kind === "turn_ended") view.live = { doing: null, text: "" };
      if (event.seq === view.log.length) {
        view.log.push(event);
        return false;
      }
      return event.seq > view.log.length;
    }
    case "activity": {
      if (event.thread !== view.thread) return false;
      view.live.doing = event.doing;
      // A new attempt: whatever an earlier one wrote is gone.
      if (event.doing.kind === "thinking") view.live.text = "";
      return false;
    }
    case "delta": {
      if (event.thread !== view.thread || event.delta.kind !== "text") return false;
      view.live.doing = { kind: "writing" };
      view.live.text += event.delta.text;
      return false;
    }
    case "submission": {
      const s: Submission = event.submission;
      if (s.thread !== view.thread) return false;
      view.unanswered = s.status === "unanswered" ? (s.reason ?? "No answer.") : null;
      return false;
    }
    case "lagged":
      return true;
    default:
      return false;
  }
}

export type Row =
  | { kind: "user"; key: string; text: string }
  | { kind: "assistant"; key: string; text: string }
  | { kind: "tool"; key: string; name: string; input: JsonValue; result: string | null; isError: boolean }
  | { kind: "note"; key: string; text: string };

/** The log as rows to draw: messages, tool calls with their results, notes. */
export function rows(log: Checkpoint[]): Row[] {
  const out: Row[] = [];
  const tools = new Map<string, Extract<Row, { kind: "tool" }>>();
  for (const cp of log) {
    const { change } = cp;
    const key = `${cp.thread}/${cp.seq}`;
    switch (change.kind) {
      case "message":
        change.content.forEach((part, i) => {
          const k = `${key}/${i}`;
          switch (part.type) {
            case "text":
              if (part.text.trim()) out.push({ kind: change.role, key: k, text: part.text });
              break;
            case "tool_call": {
              const row: Extract<Row, { kind: "tool" }> = { kind: "tool", key: k, name: part.name, input: part.input, result: null, isError: false };
              tools.set(part.id, row);
              out.push(row);
              break;
            }
            case "tool_result": {
              const row = tools.get(part.call_id);
              if (row) {
                row.result = part.content;
                row.isError = part.is_error;
              } else {
                out.push({ kind: "tool", key: k, name: "tool", input: null, result: part.content, isError: part.is_error });
              }
              break;
            }
            // Thinking stays private to the transcript's presence line.
          }
        });
        break;
      case "recalled":
        if (change.memories.length) {
          out.push({ kind: "note", key, text: `Recalled ${change.memories.length} ${change.memories.length === 1 ? "memory" : "memories"}` });
        }
        break;
      case "compacted":
        out.push({ kind: "note", key, text: `Summarized ${change.replaced} earlier messages` });
        break;
      case "system":
        if (change.text) out.push({ kind: "note", key, text: change.text });
        break;
    }
  }
  return out;
}

/** What the presence line says. */
export function presenceText(doing: Presence): string {
  switch (doing?.kind) {
    case "remembering":
      return "Remembering…";
    case "thinking":
      return "Thinking…";
    case "tool":
      return `Using ${doing.name}…`;
    case "writing":
      return "Writing…";
    default:
      return "";
  }
}
