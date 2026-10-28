// Dev-only test hooks. Exposes `window.__moove` for scripts run via the
// MOOVE_TEST env var (see src-tauri/src/debug.rs). Tree-shaken from release.
import { invoke } from "@tauri-apps/api/core";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function installTestHooks(getState: () => unknown) {
  const hooks = {
    invoke,
    sleep,
    state: getState,
    log: (message: string) => invoke("debug_log", { message }),
    eval: (label: string, js: string) => invoke("debug_eval", { label, js }),
    resize: (width: number, height: number) => invoke("debug_resize", { width, height }),
    shot: (name: string) => invoke<string>("debug_screenshot", { name }),
    quit: async () => {
      await invoke("debug_log", { message: "TEST DONE" });
      await invoke("debug_quit");
    },
    assert(cond: unknown, message: string) {
      if (!cond) throw new Error(`assertion failed: ${message}`);
      return hooks.log(`ok - ${message}`);
    },
    /** Poll until `fn` returns truthy or time out. */
    async until<T>(fn: () => T, message: string, timeout = 8000): Promise<T> {
      const start = performance.now();
      for (;;) {
        const value = fn();
        if (value) {
          await hooks.log(`ok - ${message}`);
          return value;
        }
        if (performance.now() - start > timeout) throw new Error(`timed out: ${message}`);
        await sleep(50);
      }
    },
  };
  (window as any).__moove = hooks;
  invoke("debug_ready");
}
