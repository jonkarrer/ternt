import { describe, expect, test } from "bun:test";
import { apply, emptyView, presenceText, rows, type Checkpoint } from "./thread";
import type { Change, Event } from "./protocol";

const cp = (seq: number, change: Change, thread = "t1"): Checkpoint => ({ type: "checkpoint", thread, seq, at: seq, turn: 1, change });
const user = (text: string, seq: number) => cp(seq, { kind: "message", role: "user", content: [{ type: "text", text }] });
const assistant = (text: string, seq: number) => cp(seq, { kind: "message", role: "assistant", content: [{ type: "text", text }] });

describe("apply", () => {
  test("checkpoints in order build the log; a gap asks for history", () => {
    const view = emptyView("t1");
    expect(apply(view, cp(0, { kind: "turn_started" }))).toBe(false);
    expect(apply(view, user("hi", 1))).toBe(false);
    expect(view.log.length).toBe(2);
    // Already have it: ignored, no gap.
    expect(apply(view, user("hi", 1))).toBe(false);
    expect(view.log.length).toBe(2);
    expect(apply(view, cp(5, { kind: "turn_ended" }))).toBe(true);
    expect(view.log.length).toBe(2);
    expect(apply(view, { type: "lagged" })).toBe(true);
  });

  test("other threads are ignored", () => {
    const view = emptyView("t1");
    apply(view, cp(0, { kind: "turn_started" }, "t2"));
    apply(view, { type: "delta", thread: "t2", delta: { kind: "text", text: "x" } });
    expect(view.log).toEqual([]);
    expect(view.live.text).toBe("");
  });

  test("a reply streams, then its checkpoint replaces it", () => {
    const view = emptyView("t1");
    apply(view, { type: "activity", thread: "t1", doing: { kind: "thinking" } });
    expect(presenceText(view.live.doing)).toBe("Thinking…");
    apply(view, { type: "delta", thread: "t1", delta: { kind: "text", text: "Hel" } });
    apply(view, { type: "delta", thread: "t1", delta: { kind: "text", text: "lo" } });
    expect(view.live).toEqual({ doing: { kind: "writing" }, text: "Hello" });
    apply(view, assistant("Hello", 0));
    expect(view.live.text).toBe("");
    apply(view, cp(1, { kind: "turn_ended" }));
    expect(view.live.doing).toBeNull();
  });

  test("a new attempt clears what the last one wrote", () => {
    const view = emptyView("t1");
    apply(view, { type: "delta", thread: "t1", delta: { kind: "text", text: "half" } });
    apply(view, { type: "activity", thread: "t1", doing: { kind: "thinking" } });
    expect(view.live.text).toBe("");
  });

  test("submissions set and clear the unanswered reason", () => {
    const view = emptyView("t1");
    const sub = (status: "queued" | "unanswered", reason: string | null = null): Event => ({
      type: "submission",
      submission: { id: "s", thread: "t1", text: "x", status, answer: null, reason },
    });
    apply(view, sub("unanswered", "rate limited"));
    expect(view.unanswered).toBe("rate limited");
    apply(view, sub("queued"));
    expect(view.unanswered).toBeNull();
  });
});

describe("rows", () => {
  test("messages, tool calls paired with results, notes", () => {
    const log: Checkpoint[] = [
      cp(0, { kind: "turn_started" }),
      user("what's open?", 1),
      cp(2, { kind: "recalled", memories: ["a", "b"] }),
      cp(3, { kind: "message", role: "assistant", content: [{ type: "tool_call", id: "c1", name: "space_tabs", input: { space: "dev" } }] }),
      cp(4, { kind: "message", role: "user", content: [{ type: "tool_result", call_id: "c1", content: "3 tabs", is_error: false }] }),
      cp(5, { kind: "message", role: "assistant", content: [{ type: "thinking", text: "hmm" }, { type: "text", text: "Three tabs." }] }),
      cp(6, { kind: "turn_ended" }),
    ];
    expect(rows(log)).toEqual([
      { kind: "user", key: "t1/1/0", text: "what's open?" },
      { kind: "note", key: "t1/2", text: "Recalled 2 memories" },
      { kind: "tool", key: "t1/3/0", name: "space_tabs", input: { space: "dev" }, result: "3 tabs", isError: false },
      { kind: "assistant", key: "t1/5/1", text: "Three tabs." },
    ]);
  });

  test("a result without its call still shows", () => {
    const log = [cp(0, { kind: "message", role: "user", content: [{ type: "tool_result", call_id: "zz", content: "boom", is_error: true }] })];
    expect(rows(log)).toEqual([{ kind: "tool", key: "t1/0/0", name: "tool", input: null, result: "boom", isError: true }]);
  });
});
