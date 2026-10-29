import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export { hostOf, parseInput, type ParsedInput } from "./url";

export type Zone = "favorite" | "pinned" | "today";

export interface Tab {
  id: number;
  kind: "web" | "terminal";
  url: string;
  title: string;
  zone: Zone;
  /** Owning space (ignored for favorites, which every space shares). */
  space: number;
  /** Pinned tabs only. */
  folder: number | null;
  /** Web tabs: whether the page is in memory (tabs load lazily). */
  loaded: boolean;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
}

export interface Space {
  id: number;
  name: string;
  color: string;
  /** Set when the space has its own cookie jar. */
  profile: string | null;
}

export interface Folder {
  id: number;
  space: number;
  name: string;
  open: boolean;
}

export interface BrowserState {
  /** Within each group (zone, space, folder), in sidebar order. */
  tabs: Tab[];
  spaces: Space[];
  folders: Folder[];
  active: number | null;
  activeSpace: number;
  sidebarWidth: number;
  sidebarCollapsed: boolean;
}

export interface HistoryHit {
  url: string;
  title: string;
}

/** Self-update state (see src-tauri/src/updater.rs). */
export type UpdateStatus =
  | { state: "idle" }
  | { state: "upToDate" }
  | { state: "available"; version: string }
  | { state: "downloading"; version: string; progress: number | null }
  | { state: "installing"; version: string }
  | { state: "failed"; message: string };

export type PaletteMode = "newTab" | "edit";

export interface PaletteRequest {
  mode: PaletteMode;
  url: string | null;
  /** Left edge of the content pane (logical px). */
  contentLeft: number;
}

/** Mirror the sidebar constants in src-tauri/src/browser.rs. */
export const DEFAULT_SIDEBAR_WIDTH = 240;
export const MIN_SIDEBAR_WIDTH = 180;
export const MAX_SIDEBAR_WIDTH = 480;

export const browser = {
  getState: () => invoke<BrowserState>("get_state"),
  newTab: (url: string) => invoke<number>("new_tab", { url }),
  /** `command` runs in the new terminal's shell. */
  newTerminal: (command?: string) => invoke<number>("new_terminal", { command: command ?? null }),
  closeTab: (id: number) => invoke("close_tab", { id }),
  activateTab: (id: number) => invoke("activate_tab", { id }),
  /** `index` is the position in the full tab list after removing the tab. */
  moveTab: (id: number, zone: Zone, folder: number | null, index: number | null) =>
    invoke("move_tab", { id, zone, folder, index }),
  tabContextMenu: (id: number) => invoke("tab_context_menu", { id }),
  /** Explore mode (⌘F): show a tab without leaving the mode. */
  explorePreview: (id: number) => invoke("explore_preview", { id }),
  /** Leave explore mode: open a tab, cancel back to the start, or stay put. */
  exploreEnd: (open: number | null, cancel = false) => invoke("explore_end", { open, cancel }),

  createSpace: () => invoke<number>("create_space"),
  activateSpace: (id: number) => invoke("activate_space", { id }),
  renameSpace: (id: number, name: string) => invoke("rename_space", { id, name }),
  spaceContextMenu: (id: number) => invoke("space_context_menu", { id }),
  createFolder: () => invoke<number | null>("create_folder"),
  renameFolder: (id: number, name: string) => invoke("rename_folder", { id, name }),
  setFolderOpen: (id: number, open: boolean) => invoke("set_folder_open", { id, open }),
  folderContextMenu: (id: number) => invoke("folder_context_menu", { id }),
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
  /** Visited pages matching `query`, in the active space's profile. */
  searchHistory: (query: string, limit: number) => invoke<HistoryHit[]>("history_search", { query, limit }),

  onState: (cb: (state: BrowserState) => void): Promise<UnlistenFn> =>
    listen<BrowserState>("browser:state", (e) => cb(e.payload)),
  onPaletteOpen: (cb: (request: PaletteRequest) => void): Promise<UnlistenFn> =>
    listen<PaletteRequest>("palette:open", (e) => cb(e.payload)),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  /** Download, install and relaunch. */
  installUpdate: () => invoke("install_update"),
  checkForUpdates: () => invoke("check_for_updates"),
  onUpdate: (cb: (status: UpdateStatus) => void): Promise<UnlistenFn> =>
    listen<UpdateStatus>("update:status", (e) => cb(e.payload)),
  /** Explore mode (⌘F) started or ended. */
  onExplore: (cb: (on: boolean) => void): Promise<UnlistenFn> => listen<boolean>("browser:explore", (e) => cb(e.payload)),
  /** Rust asks the sidebar to start inline-renaming a space or folder. */
  onRename: (cb: (target: { kind: "space" | "folder"; id: number }) => void): Promise<UnlistenFn> =>
    listen<{ kind: "space" | "folder"; id: number }>("browser:rename", (e) => cb(e.payload)),
};

