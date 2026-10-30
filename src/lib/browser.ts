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
  /** An emoji shown instead of the color dot. */
  icon: string | null;
}

export interface Folder {
  id: number;
  space: number;
  name: string;
  open: boolean;
}

/** A cookie jar and the spaces using it (see `profile_groups` in model.rs). */
export interface Profile {
  /** `null` for the main "Ternt" profile. */
  profile: string | null;
  /** How `spaceProfile` selects it: `shared` or `space-<id>`. */
  choice: string;
  /** "Ternt" for the main profile; others are named after their spaces unless renamed. */
  name: string;
  /** Space ids, in space order. */
  spaces: number[];
}

/** Two tabs side by side (see `Split` in model.rs). */
export interface Split {
  left: number;
  right: number;
  /** The left pane's share of the width. */
  ratio: number;
}

/** A closed Today tab that can be reopened (newest last). */
export interface ClosedTab {
  id: number;
  kind: "web" | "terminal";
  url: string;
  title: string;
  space: number;
}

export interface BrowserState {
  /** Within each group (zone, space, folder), in sidebar order. */
  tabs: Tab[];
  spaces: Space[];
  folders: Folder[];
  /** The main profile first, then each separate one. */
  profiles: Profile[];
  /** Oldest first. */
  closed: ClosedTab[];
  splits: Split[];
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
/** Mirror INSET in src-tauri/src/browser.rs (also the gap between split panes). */
export const INSET = 8;

export const browser = {
  getState: () => invoke<BrowserState>("get_state"),
  newTab: (url: string) => invoke<number>("new_tab", { url }),
  /** `command` runs in the new terminal's shell. */
  newTerminal: (command?: string) => invoke<number>("new_terminal", { command: command ?? null }),
  closeTab: (id: number) => invoke("close_tab", { id }),
  activateTab: (id: number) => invoke("activate_tab", { id }),
  /** Show a tab beside the active one. */
  splitView: (id: number) => invoke("split_view", { id }),
  /** Returns the ratio after Rust clamps it (null: no split on screen). */
  setSplitRatio: (ratio: number) => invoke<number | null>("set_split_ratio", { ratio }),
  /** Reopen one closed tab, in the space it was closed from. */
  reopenClosed: (id: number) => invoke("reopen_closed", { id }),
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
  /** `choice`: `shared`, `new` or `space-<id>` (share that space's profile). */
  spaceProfile: (id: number, choice: string) => invoke("space_profile", { id, profile: choice }),
  setSpaceColor: (id: number, color: string) => invoke("space_color", { id, color }),
  /** `null` or blank removes it. */
  setSpaceIcon: (id: number, icon: string | null) => invoke("space_icon", { id, icon }),
  /** An empty name goes back to the names of its spaces. */
  renameProfile: (profile: string, name: string) => invoke("rename_profile", { profile, name }),
  clearProfileHistory: (profile: string | null) => invoke("clear_profile_history", { profile }),
  appVersion: () => invoke<string>("app_version"),
  revealDataDir: () => invoke("reveal_data_dir"),
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
  /** Rust asks the sidebar to show the icon picker for a space. */
  onIconPicker: (cb: (space: number) => void): Promise<UnlistenFn> =>
    listen<number>("browser:icon-picker", (e) => cb(e.payload)),
  /** Rust asks the sidebar to start inline-renaming a space or folder. */
  onRename: (cb: (target: { kind: "space" | "folder"; id: number }) => void): Promise<UnlistenFn> =>
    listen<{ kind: "space" | "folder"; id: number }>("browser:rename", (e) => cb(e.payload)),
};

