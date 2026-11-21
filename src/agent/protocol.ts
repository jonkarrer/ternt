// Tokidoki's protocol, copied from crates/interface/protocol.ts in the tokidoki repo
// (commit 7f34352; generated there by `just types`) by scripts/sync-protocol.sh; don't
// edit it here. src-tauri/src/agent/protocol.rs is the same set of types in Rust.
//
// The tokidoki protocol (crates/interface/src/protocol.rs), as TypeScript.
// Generated: change the Rust types, then run `just types`.

export type JsonValue = number | string | boolean | JsonValue[] | { [key: string]: JsonValue } | null;

/**
 * Who an input came from. Their memory scope follows from it, so the core decides who may
 * speak. The ID is whatever the code passing inputs in uses to tell people apart.
 */
export type Speaker = { id: string, name?: string | null, };

/**
 * Bytes, inline or where to fetch them.
 */
export type Media = { "base64": string } | { "url": string };

/**
 * One piece of an input. The core takes text today; the other kinds are in the protocol so
 * they can be passed in, and are refused until the core can use them.
 */
export type Part = { "type": "text", text: string, } | { "type": "image", media_type: string, data: Media, } | { "type": "audio", media_type: string, data: Media, } | { "type": "file", name: string, media_type: string, data: Media, };

/**
 * What to do with an input to a thread that is busy answering another.
 */
export type WhenBusy = "follow_up" | "steer";

export type Input = { from: Speaker, parts: Array<Part>, 
/**
 * The thread to send it to; `None` lets the router choose.
 */
thread?: string | null, 
/**
 * The same ID finds the same submission instead of submitting twice (a retried send).
 */
request_id?: string | null, when_busy: WhenBusy, };

/**
 * Why an input went where it did.
 */
export type Route = { 
/**
 * `rule`, `search`, `classifier` or `model`.
 */
by: string, reason: string, 
/**
 * A new thread was started for it.
 */
new: boolean, };

export type Sent = { thread: string, submission: string, 
/**
 * `None` when the input named its thread, or its request ID was already submitted.
 */
route: Route | null, };

export type Status = "queued" | "placed" | "done" | "unanswered";

/**
 * An input and what became of it.
 */
export type Submission = { id: string, thread: string, text: string, status: Status, answer: string | null, 
/**
 * Why it went unanswered.
 */
reason: string | null, };

export type ThreadSummary = { id: string, 
/**
 * The start of its first message.
 */
title: string, turns: number, 
/**
 * What it is about, in tokidoki's words: written while the thread is idle, so a new
 * thread has none yet.
 */
about?: string | null, 
/**
 * Ms since the epoch.
 */
created_at: number, 
/**
 * The last message, ms since the epoch.
 */
active_at: number, };

export type Role = "user" | "assistant";

export type Content = { "type": "text", text: string, } | { "type": "tool_call", id: string, name: string, input: JsonValue, } | { "type": "tool_result", call_id: string, content: string, is_error: boolean, } | { "type": "thinking", text: string, };

/**
 * A change to a thread, as recorded: every checkpoint is one.
 */
export type Change = { "kind": "turn_started" } | { "kind": "step_started" } | { "kind": "message", role: Role, content: Array<Content>, } | { "kind": "turn_ended" } | { "kind": "routed", by: string, reason: string, } | { "kind": "recalled", memories: Array<string>, } | { "kind": "compacted", replaced: number, with: number, } | { "kind": "system", text: string | null, } | { "kind": "tool_added", name: string, } | { "kind": "tool_removed", name: string, };

export type Event = { "type": "checkpoint", thread: string, seq: number, 
/**
 * Ms since the epoch.
 */
at: number, turn: number, change: Change, } | { "type": "submission", submission: Submission, } | { "type": "activity", thread: string, doing: Doing, } | { "type": "delta", thread: string, delta: Delta, } | { "type": "memory", change: MemoryChange, } | { "type": "lagged" };

/**
 * Something tokidoki remembers, across every thread.
 */
export type Memory = { 
/**
 * Names this version: a memory that is revised gets a new ID.
 */
id: number, kind: MemoryKind, 
/**
 * The memory, as one line.
 */
text: string, 
/**
 * 1 when first learned; each revision adds one.
 */
version: number, 
/**
 * When this version was learned, ms since the epoch.
 */
at: number, 
/**
 * Where it was learned, when it came out of a conversation.
 */
source?: MemorySource | null, };

export type MemoryKind = "fact" | "procedure";

/**
 * The checkpoint a memory came from (`Event::Checkpoint::seq`).
 */
export type MemorySource = { thread: string, seq: number, };

export type MemoryChange = { "kind": "learned", memory: Memory, } | { "kind": "revised", memory: Memory, replaces: number, was: string, } | { "kind": "forgotten", ids: Array<number>, } | { "kind": "described", thread: string, about: string, };

/**
 * What a reply grew by.
 */
export type Delta = { "kind": "text", text: string, } | { "kind": "thinking", text: string, };

/**
 * What tokidoki started doing on a thread.
 */
export type Doing = { "kind": "remembering" } | { "kind": "thinking" } | { "kind": "tool", name: string, };

/**
 * "That was wrong": recorded with the thread (and turn), for investigating later.
 */
export type Feedback = { from: Speaker, thread: string, 
/**
 * The turn it is about; `None` for the latest.
 */
turn: number | null, text: string, };
