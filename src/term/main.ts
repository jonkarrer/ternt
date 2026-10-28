// Terminal webview: hosts one xterm.js instance per terminal tab and shows
// the active one. Sessions are reconciled against the browser tab state, so
// Rust stays the single source of truth for which terminals exist.
import { Channel, invoke } from "@tauri-apps/api/core";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import "../theme.css";
import "./term.css";
import { browser, type BrowserState } from "../lib/browser";

interface Session {
  id: number;
  term: Terminal;
  fit: FitAddon;
  el: HTMLDivElement;
  /** The shell is started the first time the terminal is shown, at its real size. */
  started: boolean;
  webgl?: WebglAddon;
}

const DARK: ITheme = {
  background: "#232325",
  foreground: "#e4e4e6",
  cursor: "#f5f5f7",
  cursorAccent: "#232325",
  selectionBackground: "rgba(255, 255, 255, 0.22)",
  black: "#2e2e31",
  red: "#ff6b6b",
  green: "#7ad67a",
  yellow: "#f2cc60",
  blue: "#6aa7ff",
  magenta: "#d38cff",
  cyan: "#5fd4e0",
  white: "#d8d8dc",
  brightBlack: "#6e6e74",
  brightRed: "#ff8787",
  brightGreen: "#98e598",
  brightYellow: "#f7dc8a",
  brightBlue: "#8dbcff",
  brightMagenta: "#e0a8ff",
  brightCyan: "#86e2ea",
  brightWhite: "#ffffff",
};

const LIGHT: ITheme = {
  background: "#ffffff",
  foreground: "#1d1d1f",
  cursor: "#1d1d1f",
  cursorAccent: "#ffffff",
  selectionBackground: "rgba(0, 0, 0, 0.14)",
  black: "#1d1d1f",
  red: "#c4262e",
  green: "#2a8a3a",
  yellow: "#a06a00",
  blue: "#1f5fd1",
  magenta: "#8a3fc4",
  cyan: "#0f7d8c",
  white: "#c8c8cc",
  brightBlack: "#6e6e74",
  brightRed: "#e0444b",
  brightGreen: "#38a64b",
  brightYellow: "#c08400",
  brightBlue: "#3478f6",
  brightMagenta: "#a25ad8",
  brightCyan: "#1b97a8",
  brightWhite: "#ffffff",
};

const root = document.getElementById("terminals")!;
const sessions = new Map<number, Session>();
const dark = matchMedia("(prefers-color-scheme: dark)");
let active: Session | null = null;

const theme = () => (dark.matches ? DARK : LIGHT);

function create(id: number): Session {
  const el = document.createElement("div");
  el.className = "terminal";
  el.hidden = true;
  root.append(el);

  const term = new Terminal({
    fontFamily: '"SF Mono", Menlo, monospace',
    fontSize: 13,
    lineHeight: 1.2,
    cursorBlink: true,
    macOptionIsMeta: true,
    scrollback: 10_000,
    theme: theme(),
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  // ⌘-click (or click) a URL to open it as a browser tab.
  term.loadAddon(new WebLinksAddon((_event, uri) => browser.newTab(uri)));
  term.open(el);

  const session: Session = { id, term, fit, el, started: false };
  sessions.set(id, session);
  return session;
}

async function start(s: Session) {
  s.started = true;
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (data) => {
    const bytes = new Uint8Array(data);
    // Ack once xterm has parsed the chunk; Rust pauses output when too much
    // is unacked, so a flood of output can't outrun rendering.
    s.term.write(bytes, () => invoke("pty_ack", { id: s.id, bytes: bytes.length }));
  };
  s.term.onData((data) => invoke("pty_write", { id: s.id, data }));
  s.term.onTitleChange((title) => invoke("set_tab_title", { id: s.id, title }));
  await invoke("pty_spawn", { id: s.id, cols: s.term.cols, rows: s.term.rows, channel });
  s.term.onResize(({ cols, rows }) => invoke("pty_resize", { id: s.id, cols, rows }));
}

function show(s: Session) {
  if (active === s) {
    s.term.focus();
    return;
  }
  // Only the visible terminal holds a WebGL context (browsers cap them).
  if (active) {
    active.el.hidden = true;
    active.webgl?.dispose();
    active.webgl = undefined;
  }
  active = s;
  s.el.hidden = false;
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => {
      webgl.dispose();
      if (s.webgl === webgl) s.webgl = undefined;
    });
    s.term.loadAddon(webgl);
    s.webgl = webgl;
  } catch {
    // Falls back to xterm's DOM renderer.
  }
  s.fit.fit();
  if (!s.started) start(s);
  s.term.focus();
}

function dispose(s: Session) {
  s.webgl?.dispose();
  s.term.dispose();
  s.el.remove();
  sessions.delete(s.id);
  if (active === s) active = null;
}

function reconcile(state: BrowserState) {
  const ids = new Set(state.tabs.filter((t) => t.kind === "terminal").map((t) => t.id));
  for (const s of [...sessions.values()]) if (!ids.has(s.id)) dispose(s);
  for (const id of ids) if (!sessions.has(id)) create(id);
  const next = state.active != null ? sessions.get(state.active) : undefined;
  if (next) show(next);
}

let fitQueued = false;
new ResizeObserver(() => {
  if (fitQueued) return;
  fitQueued = true;
  requestAnimationFrame(() => {
    fitQueued = false;
    active?.fit.fit();
  });
}).observe(root);

dark.addEventListener("change", () => {
  for (const s of sessions.values()) s.term.options.theme = theme();
});

browser.onState(reconcile);
browser.getState().then(reconcile);

if (import.meta.env.DEV) {
  // Lets e2e tests read what a terminal is showing.
  (window as any).__termText = (id: number) => {
    const buffer = sessions.get(id)?.term.buffer.active;
    if (!buffer) return "";
    const lines = [];
    for (let i = 0; i < buffer.length; i++) lines.push(buffer.getLine(i)?.translateToString(true) ?? "");
    return lines.join("\n").trimEnd();
  };
}
