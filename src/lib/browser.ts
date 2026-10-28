import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Tab {
  id: number;
  url: string;
  title: string;
  loading: boolean;
}

export interface BrowserState {
  tabs: Tab[];
  active: number | null;
}

/** Mirror the sidebar constants in src-tauri/src/browser.rs. */
export const DEFAULT_SIDEBAR_WIDTH = 240;
export const MIN_SIDEBAR_WIDTH = 180;
export const MAX_SIDEBAR_WIDTH = 480;

const SEARCH_URL = "https://www.google.com/search?q=";

export const browser = {
  getState: () => invoke<BrowserState>("get_state"),
  newTab: (url: string) => invoke<number>("new_tab", { url }),
  closeTab: (id: number) => invoke("close_tab", { id }),
  activateTab: (id: number) => invoke("activate_tab", { id }),
  navigate: (url: string) => invoke("navigate", { url }),
  goBack: () => invoke("go_back"),
  goForward: () => invoke("go_forward"),
  reload: () => invoke("reload"),
  /** Returns the width after Rust clamps it. */
  setSidebarWidth: (width: number) => invoke<number>("set_sidebar_width", { width }),

  onState: (cb: (state: BrowserState) => void): Promise<UnlistenFn> =>
    listen<BrowserState>("browser:state", (e) => cb(e.payload)),
  /** Payload is true for "new tab", false for "edit current URL". */
  onOmnibox: (cb: (newTab: boolean) => void): Promise<UnlistenFn> =>
    listen<boolean>("browser:omnibox", (e) => cb(e.payload)),
};

/** Turn omnibox input into a URL: explicit URL, bare host, or search. */
export function toUrl(raw: string): string | null {
  const input = raw.trim();
  if (!input) return null;
  if (/^(https?|file|about|data):/i.test(input)) return input;
  if (/^(localhost|\d{1,3}(\.\d{1,3}){3})(:\d+)?(\/\S*)?$/i.test(input)) return `http://${input}`;
  if (/^[^\s/]+\.[a-z]{2,}(:\d+)?(\/\S*)?$/i.test(input)) return `https://${input}`;
  return SEARCH_URL + encodeURIComponent(input);
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return url;
  }
}
