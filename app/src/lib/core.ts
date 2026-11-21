// The bridge to the core (src-tauri/src/lib.rs): one function per command, taking and
// returning the `interface` crate's types.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Event, Memory, MemoryChange, Sent, Submission, ThreadSummary } from "../../../crates/interface/protocol";

// The protocol's types are generated from the Rust (`just types`), never written here.
export type {
  Change,
  Content,
  Doing,
  Memory,
  MemoryChange,
  Sent,
  Submission,
  ThreadSummary,
} from "../../../crates/interface/protocol";

export type CoreEvent = Event;
export type Checkpoint = Extract<Event, { type: "checkpoint" }>;

/** Where the app is (src-tauri/src/lib.rs). */
export type Status = { state: "needs_keys" } | { state: "ready" } | { state: "failed"; message: string };

export type UpdateStatus =
  | { state: "idle" | "up_to_date" }
  | { state: "ready" | "installing"; version: string }
  | { state: "failed"; message: string };

export const core = {
  status: () => invoke<Status>("status"),
  keysSet: () => invoke<{ anthropic: boolean; baseten: boolean }>("keys_set"),
  /** `null` keeps the key there is; an empty Baseten key removes it. */
  setKeys: (anthropic: string | null, baseten: string | null) => invoke<void>("set_keys", { anthropic, baseten }),
  send: (text: string, thread: string | null) =>
    invoke<Sent>("send", { text, thread, steer: false, requestId: crypto.randomUUID() }),
  threads: () => invoke<ThreadSummary[]>("threads"),
  history: (thread: string, since: number) => invoke<Checkpoint[]>("history", { thread, since }),
  abort: (thread: string) => invoke<boolean>("abort", { thread }),
  memories: () => invoke<Memory[]>("memories"),
  remember: (text: string) => invoke<Memory>("remember", { text }),
  reviseMemory: (id: number, text: string) => invoke<Memory>("revise_memory", { id, text }),
  forgetMemory: (id: number) => invoke<void>("forget_memory", { id }),
  onEvent: (handler: (event: CoreEvent) => void) => listen<CoreEvent>("tokidoki:event", (e) => handler(e.payload)),

  version: () => invoke<string>("app_version"),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  checkForUpdates: () => invoke<void>("check_for_updates"),
  installUpdate: () => invoke<void>("install_update"),
  onUpdate: (handler: (status: UpdateStatus) => void) =>
    listen<UpdateStatus>("update:status", (e) => handler(e.payload)),
};
