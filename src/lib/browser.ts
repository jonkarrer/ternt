import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Tab {
  id: number;
  kind: "web" | "terminal";
  url: string;
  title: string;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
}

export interface BrowserState {
  tabs: Tab[];
  active: number | null;
}

export type PaletteMode = "newTab" | "edit";

export interface PaletteRequest {
  mode: PaletteMode;
  url: string | null;
}

/** Mirror the sidebar constants in src-tauri/src/browser.rs. */
export const DEFAULT_SIDEBAR_WIDTH = 240;
export const MIN_SIDEBAR_WIDTH = 180;
export const MAX_SIDEBAR_WIDTH = 480;

const SEARCH_URL = "https://www.google.com/search?q=";

export const browser = {
  getState: () => invoke<BrowserState>("get_state"),
  newTab: (url: string) => invoke<number>("new_tab", { url }),
  newTerminal: () => invoke<number>("new_terminal"),
  closeTab: (id: number) => invoke("close_tab", { id }),
  activateTab: (id: number) => invoke("activate_tab", { id }),
  navigate: (url: string) => invoke("navigate", { url }),
  goBack: () => invoke("go_back"),
  goForward: () => invoke("go_forward"),
  reload: () => invoke("reload"),
  /** Returns the width after Rust clamps it. */
  setSidebarWidth: (width: number) => invoke<number>("set_sidebar_width", { width }),
  /** Run a menu action by id (same code path as its keyboard shortcut). */
  menuAction: (id: string) => invoke("menu_action", { id }),

  openPalette: (mode: PaletteMode) => invoke("open_palette", { mode }),
  closePalette: () => invoke("close_palette"),
  paletteRequest: () => invoke<PaletteRequest | null>("palette_request"),

  onState: (cb: (state: BrowserState) => void): Promise<UnlistenFn> =>
    listen<BrowserState>("browser:state", (e) => cb(e.payload)),
  onPaletteOpen: (cb: (request: PaletteRequest) => void): Promise<UnlistenFn> =>
    listen<PaletteRequest>("palette:open", (e) => cb(e.payload)),
};

export type ParsedInput = { kind: "url" | "search"; url: string };

/** Interpret address-bar input: explicit URL, bare host, or a search. */
export function parseInput(raw: string): ParsedInput | null {
  const input = raw.trim();
  if (!input) return null;
  if (/^(https?|file|about|data):/i.test(input)) return { kind: "url", url: input };
  if (/^(localhost|\d{1,3}(\.\d{1,3}){3})(:\d+)?(\/\S*)?$/i.test(input)) {
    return { kind: "url", url: `http://${input}` };
  }
  if (/^[^\s/]+\.[a-z]{2,}(:\d+)?(\/\S*)?$/i.test(input)) {
    return { kind: "url", url: `https://${input}` };
  }
  return { kind: "search", url: SEARCH_URL + encodeURIComponent(input) };
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return url;
  }
}
