// Terminal webview: hosts one xterm.js instance per terminal pane (a tab
// starts with one; ⌘D splits it) and lays out the active tab's panes.
// Sessions are reconciled against the browser tab state, so Rust stays the
// single source of truth for which terminals exist and how they're split.
import { Channel, invoke } from "@tauri-apps/api/core";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import "../theme.css";
import "./term.css";
import { browser, type BrowserState, type PaneNode, type Tab } from "../lib/browser";

interface Session {
  /** Its tab. */
  id: number;
  pane: number;
  term: Terminal;
  fit: FitAddon;
  el: HTMLDivElement;
  /** The shell is started the first time the terminal is shown, at its real size. */
  started: boolean;
  webgl?: WebglAddon;
  /** The latest title its shell set (the tab shows the focused pane's). */
  title?: string;
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
const sessions = new Map<string, Session>();
const dark = matchMedia("(prefers-color-scheme: dark)");
const keyOf = (id: number, pane: number) => `${id}:${pane}`;
let latest: BrowserState | null = null;
/** The terminal tab on screen. */
let shown: Tab | null = null;
/** Its sessions, and the one with the keyboard. */
let visible: Session[] = [];
let focused: Session | null = null;
/** A pane just clicked into, until the state says it's focused (so a
    state update already on its way doesn't pull the keyboard back). */
let pendingFocus: Session | null = null;

const theme = () => (dark.matches ? DARK : LIGHT);

function create(id: number, pane: number): Session {
  const el = document.createElement("div");
  el.className = "pane";
  el.dataset.tab = String(id);
  el.dataset.pane = String(pane);
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

  const session: Session = { id, pane, term, fit, el, started: false };
  sessions.set(keyOf(id, pane), session);
  // Clicking into a pane focuses it.
  el.addEventListener("focusin", () => {
    if (focused === session) return;
    pendingFocus = session;
    setFocused(session);
    invoke("focus_pane", { id, pane });
  });
  return session;
}

async function start(s: Session) {
  s.started = true;
  const { id, pane } = s;
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (data) => {
    const bytes = new Uint8Array(data);
    // Ack once xterm has parsed the chunk; Rust pauses output when too much
    // is unacked, so a flood of output can't outrun rendering.
    s.term.write(bytes, () => invoke("pty_ack", { id, pane, bytes: bytes.length }));
  };
  s.term.onData((data) => invoke("pty_write", { id, pane, data }));
  s.term.onTitleChange((title) => {
    s.title = title;
    if (focused === s) nameTab(s);
  });
  await invoke("pty_spawn", { id, pane, cols: s.term.cols, rows: s.term.rows, channel });
  s.term.onResize(({ cols, rows }) => invoke("pty_resize", { id, pane, cols, rows }));
}

/** The tab is named after its focused pane. */
function nameTab(s: Session) {
  const tab = latest?.tabs.find((t) => t.id === s.id);
  if (s.title && tab && tab.title !== s.title) invoke("set_tab_title", { id: s.id, title: s.title });
}

/** Focus the terminal only if this webview has the keyboard: Rust decides
    which view gets it (explore mode keeps it in the sidebar), and a script
    focusing an element would make WebKit grab it. */
function focus(s: Session) {
  if (document.hasFocus()) s.term.focus();
}
window.addEventListener("focus", () => focused?.term.focus());

function setFocused(s: Session) {
  const changed = focused !== s;
  focused = s;
  for (const v of visible) v.el.classList.toggle("dimmed", visible.length > 1 && v !== s);
  if (changed) nameTab(s);
}

function reveal(s: Session) {
  if (!s.el.hidden) return;
  s.el.hidden = false;
  // Only visible terminals hold a WebGL context (browsers cap them).
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
}

function conceal(s: Session) {
  s.el.hidden = true;
  s.webgl?.dispose();
  s.webgl = undefined;
}

function dispose(s: Session) {
  s.webgl?.dispose();
  s.term.dispose();
  s.el.remove();
  sessions.delete(keyOf(s.id, s.pane));
  visible = visible.filter((v) => v !== s);
  if (focused === s) focused = null;
  if (pendingFocus === s) pendingFocus = null;
}

// --- Layout --------------------------------------------------------------------

/** Room between panes; a divider line runs down its middle. */
const GAP = 13;

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Ratios of dividers being dragged, by split id (sent to Rust on release). */
const dragging = new Map<number, number>();
const dividers = new Map<number, HTMLDivElement>();
/** Each divider's split: the room it shares out, in page coordinates. */
const rooms = new Map<number, { room: Rect; across: boolean }>();

function place(el: HTMLElement, r: Rect) {
  Object.assign(el.style, { left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${r.h}px` });
}

/** Lay out `node` in `r`: panes get their rects, splits a divider. */
function layoutNode(node: PaneNode, r: Rect, used: Set<number>) {
  if (node.kind === "pane") {
    const s = sessions.get(keyOf(shown!.id, node.id));
    if (s) place(s.el, r);
    return;
  }
  const across = node.direction === "right";
  const room = Math.max(0, (across ? r.w : r.h) - GAP);
  const first = Math.round(room * (dragging.get(node.id) ?? node.ratio));
  const [a, gap, b]: Rect[] = across
    ? [
        { ...r, w: first },
        { ...r, x: r.x + first, w: GAP },
        { ...r, x: r.x + first + GAP, w: room - first },
      ]
    : [
        { ...r, h: first },
        { ...r, y: r.y + first, h: GAP },
        { ...r, y: r.y + first + GAP, h: room - first },
      ];
  used.add(node.id);
  place(divider(node, r), gap);
  layoutNode(node.first, a, used);
  layoutNode(node.second, b, used);
}

/** A split's divider, made once and kept (it holds the pointer while dragged). */
function divider(node: Extract<PaneNode, { kind: "split" }>, room: Rect): HTMLDivElement {
  let el = dividers.get(node.id);
  if (!el) {
    el = document.createElement("div");
    el.dataset.split = String(node.id);
    root.append(el);
    dividers.set(node.id, el);
    const handle = el;
    handle.addEventListener("pointerdown", (event) => {
      event.preventDefault();
      handle.setPointerCapture(event.pointerId);
      const tab = shown!.id;
      const split = node.id;
      let ratio: number | null = null;
      const move = (e: PointerEvent) => {
        const { room: r, across } = rooms.get(split)!;
        const offset = across ? e.clientX - r.x : e.clientY - r.y;
        ratio = Math.min(0.9, Math.max(0.1, (offset - GAP / 2) / ((across ? r.w : r.h) - GAP)));
        dragging.set(split, ratio);
        layout();
      };
      const up = async () => {
        handle.removeEventListener("pointermove", move);
        handle.removeEventListener("pointerup", up);
        handle.removeEventListener("pointercancel", up);
        if (ratio != null) await invoke("set_pane_ratio", { id: tab, split, ratio });
        dragging.delete(split);
      };
      handle.addEventListener("pointermove", move);
      handle.addEventListener("pointerup", up);
      handle.addEventListener("pointercancel", up);
    });
  }
  const across = node.direction === "right";
  el.className = `divider ${across ? "across" : "stacked"}`;
  // The split's whole room, in page coordinates, for dragging.
  const origin = root.getBoundingClientRect();
  rooms.set(node.id, { room: { ...room, x: room.x + origin.x, y: room.y + origin.y }, across });
  return el;
}

/** Place the shown tab's panes and dividers, and fit each terminal. */
function layout() {
  const used = new Set<number>();
  if (shown?.panes) {
    const style = getComputedStyle(root);
    const r: Rect = {
      x: parseFloat(style.paddingLeft),
      y: parseFloat(style.paddingTop),
      w: root.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight),
      h: root.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom),
    };
    layoutNode(shown.panes.root, r, used);
  }
  for (const [id, el] of dividers) {
    if (!used.has(id)) {
      el.remove();
      dividers.delete(id);
      rooms.delete(id);
    }
  }
  for (const s of visible) s.fit.fit();
}

// --- State -----------------------------------------------------------------------

function reconcile(state: BrowserState) {
  latest = state;
  const terminals = state.tabs.filter((t) => t.kind === "terminal");
  const wanted = new Set(terminals.flatMap((t) => paneIds(t).map((p) => keyOf(t.id, p))));
  for (const s of [...sessions.values()]) if (!wanted.has(keyOf(s.id, s.pane))) dispose(s);
  for (const t of terminals) for (const p of paneIds(t)) if (!sessions.has(keyOf(t.id, p))) create(t.id, p);

  // The active tab, or its split partner when that's the terminal on screen.
  const split = state.splits.find((s) => s.left === state.active || s.right === state.active);
  const onScreen = split ? [state.active, split.left === state.active ? split.right : split.left] : [state.active];
  const tab = onScreen.map((id) => terminals.find((t) => t.id === id)).find((t) => t);
  if (!tab) return;

  shown = tab;
  const next = paneIds(tab).map((p) => sessions.get(keyOf(tab.id, p))!);
  for (const s of visible) if (!next.includes(s)) conceal(s);
  visible = next;
  for (const s of visible) reveal(s);
  layout();
  for (const s of visible) if (!s.started) start(s);

  const target = tab.panes?.focused ?? 0;
  if (pendingFocus && (pendingFocus.id !== tab.id || pendingFocus.pane === target)) pendingFocus = null;
  const keyboard = pendingFocus ?? sessions.get(keyOf(tab.id, target)) ?? visible[0];
  setFocused(keyboard);
  focus(keyboard);
}

function paneIds(tab: Tab): number[] {
  const ids: number[] = [];
  const walk = (n: PaneNode): void => {
    if (n.kind === "pane") ids.push(n.id);
    else (walk(n.first), walk(n.second));
  };
  if (tab.panes) walk(tab.panes.root);
  return ids.length ? ids : [0];
}

let fitQueued = false;
new ResizeObserver(() => {
  if (fitQueued) return;
  fitQueued = true;
  requestAnimationFrame(() => {
    fitQueued = false;
    layout();
  });
}).observe(root);

dark.addEventListener("change", () => {
  for (const s of sessions.values()) s.term.options.theme = theme();
});

// Subscribe before reading state, so no update can fall in between.
browser.onState(reconcile).then(() => browser.getState().then(reconcile));

if (import.meta.env.DEV) {
  // Lets e2e tests read what a terminal is showing (its focused pane by default).
  (window as any).__termText = (id: number, pane?: number) => {
    pane ??= latest?.tabs.find((t) => t.id === id)?.panes?.focused ?? 0;
    const buffer = sessions.get(keyOf(id, pane))?.term.buffer.active;
    if (!buffer) return "";
    const lines = [];
    for (let i = 0; i < buffer.length; i++) lines.push(buffer.getLine(i)?.translateToString(true) ?? "");
    return lines.join("\n").trimEnd();
  };
}
