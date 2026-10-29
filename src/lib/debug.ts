// Dev-only test hooks. Exposes `window.__moove` for scripts run via the
// MOOVE_TEST env var (see src-tauri/src/debug.rs). Tree-shaken from release.
import { invoke } from "@tauri-apps/api/core";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type State = { tabs: { id: number }[]; active: number | null };

export function installTestHooks(getState: () => State) {
  const hooks = {
    invoke,
    sleep,
    state: getState,
    /** The fixture server (e2e/fixtures). */
    F: "http://localhost:8765",
    /** The active tab, as the sidebar sees it. */
    tab: () => getState().tabs.find((t) => t.id === getState().active),
    log: (message: string) => invoke("debug_log", { message }),
    eval: (label: string, js: string) => invoke("debug_eval", { label, js }),
    resize: (width: number, height: number) => invoke("debug_resize", { width, height }),
    /** Screenshots are artifacts, not assertions: a failed capture (screen
        locked, display asleep) warns instead of failing the test. */
    shot: (name: string) =>
      invoke<string>("debug_screenshot", { name }).catch((e) => hooks.log(`warn - screenshot ${name}: ${e}`)),
    /** A key press in the sidebar (explore mode, renames). */
    key: (key: string) => window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })),
    /** Click an element the way a pointer would (rows act on pointerup). */
    click(el: Element) {
      const r = el.getBoundingClientRect();
      for (const type of ["pointerdown", "pointerup"]) {
        const init = { bubbles: true, button: 0, pointerId: 1, clientX: r.left + 10, clientY: r.top + r.height / 2 };
        el.dispatchEvent(new PointerEvent(type, init));
      }
    },
    /** Drive the command bar like a user would. */
    palette: {
      request: () => invoke("palette_request"),
      async open(action = "new-tab") {
        await invoke("menu_action", { id: action });
        await hooks.until(() => hooks.palette.request(), `${action} opens the command bar`);
        await sleep(200);
      },
      type: (text: string) =>
        hooks.eval(
          "palette",
          `(() => { const i = document.querySelector("input"); i.value = ${JSON.stringify(text)}; i.dispatchEvent(new Event("input", { bubbles: true })); })()`,
        ),
      key: (key: string) =>
        hooks.eval(
          "palette",
          `document.querySelector("input").dispatchEvent(new KeyboardEvent("keydown", { key: ${JSON.stringify(key)}, bubbles: true }))`,
        ),
      /** Result labels and section headers, in order. */
      async rows(): Promise<string[]> {
        const js = `JSON.stringify([...document.querySelectorAll("li")].map((li) => (li.querySelector(".label") ?? li).textContent.trim()))`;
        return JSON.parse(await hooks.read("palette", js));
      },
    },
    /** Text currently shown by a terminal tab. */
    termText: (id: number) => hooks.read("term", `window.__termText(${id})`),
    /** Evaluate `expr` in another webview and return its (string) result. */
    async read(label: string, expr: string): Promise<string> {
      const key = `read-${Math.random()}`;
      await invoke("debug_eval", {
        label,
        js: `Promise.resolve(${expr}).then((v) => window.__TAURI_INTERNALS__.invoke("debug_set", { key: ${JSON.stringify(key)}, value: String(v) }))`,
      });
      for (let i = 0; i < 100; i++) {
        const value = await invoke<string | null>("debug_get", { key });
        if (value != null) return value;
        await sleep(50);
      }
      throw new Error(`timed out reading ${expr} from ${label}`);
    },
    quit: async () => {
      await invoke("debug_log", { message: "TEST DONE" });
      await invoke("debug_quit");
    },
    assert(cond: unknown, message: string) {
      if (!cond) throw new Error(`assertion failed: ${message}`);
      return hooks.log(`ok - ${message}`);
    },
    /** Poll until `fn` returns truthy or time out. */
    async until<T>(fn: () => T | Promise<T>, message: string, timeout = 8000): Promise<T> {
      const start = performance.now();
      for (;;) {
        const value = await fn();
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
