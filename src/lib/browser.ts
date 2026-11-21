import { invoke } from "@tauri-apps/api/core";
import type { EventCallback, UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Event as AgentEvent, Sent, ThreadSummary } from "../agent/protocol";

/** Events sent to this webview. (The global `listen` also gets every event
 *  sent to any other webview: each state update arrived once per target.) */
const listen = <T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> =>
  getCurrentWebview().listen<T>(event, handler);

export { hostOf, parseInput, tabLabel, type ParsedInput } from "./url";

export type Zone = "favorite" | "pinned" | "today";

export interface Tab {
  id: number;
  kind: "web" | "terminal";
  url: string;
  /** The page's (or shell's) own title. */
  title: string;
  /** A name the user gave it, shown instead (see `tabLabel`). */
  name: string | null;
  zone: Zone;
  /** Owning space (ignored for favorites, which every space shares). */
  space: number;
  /** Pinned tabs only. */
  folder: number | null;
  /** Web tabs: whether the page is in memory (tabs load lazily). */
  loaded: boolean;
  loading: boolean;
  /** Web tabs: the icon the page names, once read (else its /favicon.ico). */
  favicon: string | null;
  canGoBack: boolean;
  canGoForward: boolean;
  /** Terminals: the shells it's split into (see panes.rs); `null` for web tabs. */
  panes: Panes | null;
}

/** A terminal split into panes: side by side (`right`) or stacked (`down`). */
export type PaneNode =
  | { kind: "pane"; id: number }
  | { kind: "split"; id: number; direction: "right" | "down"; ratio: number; first: PaneNode; second: PaneNode };

export interface Panes {
  root: PaneNode;
  /** The pane with the keyboard; it names the tab. */
  focused: number;
}

export interface Space {
  id: number;
  name: string;
  color: string;
  /** Set when the space has its own cookie jar. */
  profile: string | null;
  /** An emoji or a `sym:` line symbol (src/lib/spaceIcons.ts); `null`
   *  shows the space's initial. */
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
  /** `null` for the main "Default" profile. */
  profile: string | null;
  /** How `spaceProfile` selects it: `shared` or `space-<id>`. */
  choice: string;
  /** "Default" for the main profile; others are named after their spaces unless renamed. */
  name: string;
  /** Space ids, in space order. */
  spaces: number[];
}

/** Up to four tabs shown together (see src-tauri/src/group.rs). */
export interface Group {
  id: number;
  name: string;
  /** Left to right; each column's tabs top to bottom. */
  columns: number[][];
  /** The first column's share of the width. */
  ratio: number;
  /** Each column's top tab's share of its height. */
  rows: number[];
}

/** A tab on screen, in window coordinates (logical px). */
export interface ScreenFrame {
  tab: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A closed Today tab that can be reopened (newest last). */
export interface ClosedTab {
  id: number;
  kind: "web" | "terminal";
  url: string;
  title: string;
  name: string | null;
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
  /** Their tabs sit together in Today, in group order. */
  groups: Group[];
  /** The group on screen (the active tab's, showing rather than the tab alone). */
  shownGroup: number | null;
  /** The tabs on screen and where. */
  screen: ScreenFrame[];
  active: number | null;
  activeSpace: number;
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  /** The collapsed sidebar is showing over the page (pointer on it). */
  sidebarPeeking: boolean;
}

export interface HistoryHit {
  url: string;
  title: string;
}

/** Self-update state (see src-tauri/src/updater.rs). */
export type UpdateStatus =
  | { state: "idle" }
  | { state: "upToDate" }
  | { state: "ready"; version: string }
  | { state: "downloading"; version: string; progress: number | null }
  | { state: "installing"; version: string }
  | { state: "failed"; message: string };

/** A file downloaded from a page (src-tauri/src/downloads.rs). */
export interface Download {
  id: number;
  url: string;
  path: string;
  name: string;
  state: "active" | "done" | "failed" | "canceled";
  received: number;
  /** The size, when the server said (and once it's done). */
  total: number | null;
  /** When it started (ms since 1970). */
  started: number;
  /** Moved or deleted since. */
  missing: boolean;
}

/** App preferences (src-tauri/src/prefs.rs). */
export interface Prefs {
  /** Where links opened from other apps go. */
  externalLinks: "little" | "tab";
  /** Who uses this Ternt (Settings → Account). */
  account: { name: string; email: string };
  /** Terminal text size in px (⌘+ / ⌘− / ⌘0 in a terminal). */
  terminalFontSize: number;
  /** Unload tabs off screen for 30 minutes, to save memory. */
  unloadTabs: boolean;
  /** Air Traffic Control: links from other apps matching a rule open in its
      space (first match wins; src-tauri/src/routing.rs). */
  linkRules: LinkRule[];
  /** Sites allowed the camera and microphone (origins); the rest ask. */
  mediaSites: string[];
  /** Who answers in the agent panel (Settings → Agent); the agent gets that provider's key. */
  agentProvider: "fireworks" | "anthropic";
}

export interface LinkRule {
  /** `host`: that host and its subdomains (with a path: URLs under it).
      `regex`: tried against the whole URL. */
  kind: "host" | "regex";
  pattern: string;
  space: number;
}

export type PaletteMode = "newTab" | "edit" | "help";

export interface PaletteRequest {
  mode: PaletteMode;
  url: string | null;
  /** Left edge of the content pane (logical px). */
  contentLeft: number;
}

/** The agent panel (⌘J): src-tauri/src/agent. */
export interface AgentSpace {
  id: number;
  name: string;
  color: string;
  icon: string | null;
}

export interface AgentRequest {
  space: AgentSpace;
  /** Left edge of the content pane (logical px). */
  contentLeft: number;
  /** `stub` until tokidoki is embedded. */
  backend: string;
  /** The agent's data folder under the app data dir. */
  folder: string;
}

/** An agent event with the space it came from. */
export interface AgentTagged {
  space: number;
  event: AgentEvent;
}

/** The agent process (src-tauri/src/agent/sidecar.rs): a crash there restarts it, not Ternt. */
/** Which keys are set (never their values). Fireworks or Anthropic answers; Baseten is the router's fallback. */
export type AgentKeys = { fireworks: boolean; anthropic: boolean; baseten: boolean };
/** A key change: a missing field leaves that key as it is, "" removes it. */
export type AgentKeyChange = Partial<Record<keyof AgentKeys, string>>;

export type AgentStatus =
  | { state: "starting" }
  /** `provider` answers (Settings → Agent); `hasKey`: its key was given (the stub needs none). */
  | { state: "running"; backend: string; restarted: boolean; provider: string; hasKey: boolean }
  /** `asked`: restarted on purpose (Restart, a key change), not a crash. */
  | { state: "restarting"; attempt: number; asked: boolean }
  | { state: "down"; reason: string };

/** Mirror the sidebar constants in src-tauri/src/browser.rs. */
export const DEFAULT_SIDEBAR_WIDTH = 240;
export const MIN_SIDEBAR_WIDTH = 180;
export const MAX_SIDEBAR_WIDTH = 480;
/** Mirror INSET in src-tauri/src/browser.rs (also the gap between a group's panes). */
export const INSET = 8;

export const browser = {
  getState: () => invoke<BrowserState>("get_state"),
  newTab: (url: string) => invoke<number>("new_tab", { url }),
  /** `command` runs in the new terminal's shell. */
  newTerminal: (command?: string) => invoke<number>("new_terminal", { command: command ?? null }),
  closeTab: (id: number) => invoke("close_tab", { id }),
  /** Remove a tab for good, pinned or not. */
  removeTab: (id: number) => invoke("remove_tab", { id }),
  activateTab: (id: number) => invoke("activate_tab", { id }),
  /** Show a tab beside the active one (joining its group). */
  splitView: (id: number) => invoke("split_view", { id }),
  /** Resize the group on screen: its columns (`column` null) or a column's
   *  rows. Returns the ratio after Rust clamps it (null: no such divider). */
  setGroupRatio: (column: number | null, ratio: number) => invoke<number | null>("set_group_ratio", { column, ratio }),
  showGroup: (id: number) => invoke("show_group", { id }),
  renameGroup: (id: number, name: string) => invoke("rename_group", { id, name }),
  ungroup: (id: number) => invoke("ungroup", { id }),
  /** While dragging a tab over the content area: show where it would drop
   *  (`point` null hides that). Returns whether it would. */
  previewGroupDrop: (tab: number, point: [number, number] | null) => invoke<boolean>("preview_group_drop", { tab, point }),
  /** Drop a dragged tab on the content area, beside the tab under it. */
  dropOnContent: (tab: number, x: number, y: number) => invoke<boolean>("drop_on_content", { tab, x, y }),
  /** Reopen one closed tab, in the space it was closed from. */
  reopenClosed: (id: number) => invoke("reopen_closed", { id }),
  /** `index` is the position in the full tab list after removing the tab. */
  moveTab: (id: number, zone: Zone, folder: number | null, index: number | null) =>
    invoke("move_tab", { id, zone, folder, index }),
  tabContextMenu: (id: number) => invoke("tab_context_menu", { id }),
  /** Explore mode (⌃F): show a tab without leaving the mode. */
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
  getPrefs: () => invoke<Prefs>("get_prefs"),
  /** Change some prefs, keeping the rest as they are now. */
  updatePrefs: async (change: Partial<Prefs>) => {
    const prefs = { ...(await invoke<Prefs>("get_prefs")), ...change };
    await invoke("set_prefs", { prefs });
    return prefs;
  },
  /** Where a link from another app would go: the matching rule and its space. */
  routeLink: (url: string) => invoke<{ rule: number; space: number } | null>("route_link", { url }),
  /** Why a rule's pattern is no good (a regex that won't compile). */
  checkLinkRule: (kind: LinkRule["kind"], pattern: string) =>
    invoke("check_link_rule", { kind, pattern }).then(
      () => null,
      (e) => String(e),
    ),
  /** A site's app icon as a data URL (src-tauri/src/icons.rs). */
  siteIcon: (url: string, tab?: number) => invoke<string | null>("site_icon", { url, tab: tab ?? null }),
  defaultBrowser: () => invoke<{ isDefault: boolean; canAsk: boolean }>("default_browser"),
  /** macOS asks the user to confirm. */
  makeDefaultBrowser: () => invoke("make_default_browser"),
  createFolder: () => invoke<number | null>("create_folder"),
  renameFolder: (id: number, name: string) => invoke("rename_folder", { id, name }),
  setFolderOpen: (id: number, open: boolean) => invoke("set_folder_open", { id, open }),
  /** `index` among the space's folders, counted without the moved one. */
  moveFolder: (id: number, index: number) => invoke("move_folder", { id, index }),
  /** An empty name goes back to the tab's own title. */
  renameTab: (id: number, name: string) => invoke("rename_tab", { id, name }),
  peekSidebar: (show: boolean) => invoke("peek_sidebar", { show }),
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
  /** The Settings window: prefs changed (by any window). */
  onPrefsChanged: (cb: (prefs: Prefs) => void): Promise<UnlistenFn> =>
    listen<Prefs>("prefs:changed", (e) => cb(e.payload)),

  // The agent panel; `space` scopes every call to one space's agent.
  agentRequest: () => invoke<AgentRequest | null>("agent_request"),
  openAgent: () => invoke("open_agent"),
  closeAgent: () => invoke("close_agent"),
  /** Send `text`; `thread` continues a conversation, `null` starts one. */
  agentSend: (space: number, text: string, thread: string | null) => invoke<Sent>("agent_send", { space, text, thread }),
  agentThreads: (space: number) => invoke<ThreadSummary[]>("agent_threads", { space }),
  /** A thread's checkpoints from `since` on. */
  agentHistory: (space: number, thread: string, since: number) => invoke<AgentEvent[]>("agent_history", { space, thread, since }),
  agentAbort: (space: number, thread: string) => invoke<boolean>("agent_abort", { space, thread }),
  onAgentOpen: (cb: (request: AgentRequest) => void): Promise<UnlistenFn> =>
    listen<AgentRequest>("agent:open", (e) => cb(e.payload)),
  onAgentEvent: (cb: (tagged: AgentTagged) => void): Promise<UnlistenFn> =>
    listen<AgentTagged>("agent:event", (e) => cb(e.payload)),
  agentStatus: () => invoke<AgentStatus>("agent_status"),
  /** Start the agent process over (after it was left down, or to recover it). */
  agentRestart: () => invoke("agent_restart"),
  onAgentStatus: (cb: (status: AgentStatus) => void): Promise<UnlistenFn> =>
    listen<AgentStatus>("agent:status", (e) => cb(e.payload)),
  /** Which API keys the agents have (never the values); they live in the Keychain. */
  agentKeys: () => invoke<AgentKeys>("agent_keys"),
  /** Store keys and restart the agent with them. */
  agentSetKeys: (change: AgentKeyChange) => invoke("agent_set_keys", change),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  /** Install the update (downloading it first if needed) and relaunch. */
  installUpdate: () => invoke("install_update"),
  checkForUpdates: () => invoke("check_for_updates"),
  onUpdate: (cb: (status: UpdateStatus) => void): Promise<UnlistenFn> =>
    listen<UpdateStatus>("update:status", (e) => cb(e.payload)),
  /** Newest first. */
  downloads: () => invoke<Download[]>("downloads"),
  onDownloads: (cb: (list: Download[]) => void): Promise<UnlistenFn> =>
    listen<Download[]>("downloads", (e) => cb(e.payload)),
  openDownload: (id: number) => invoke("open_download", { id }),
  revealDownload: (id: number) => invoke("reveal_download", { id }),
  cancelDownload: (id: number) => invoke("cancel_download", { id }),
  /** Take one finished download (or all of them) off the list; the files stay. */
  clearDownloads: (id?: number) => invoke("clear_downloads", { id: id ?? null }),
  /** Explore mode (⌃F) started or ended. */
  onExplore: (cb: (on: boolean) => void): Promise<UnlistenFn> => listen<boolean>("browser:explore", (e) => cb(e.payload)),
  /** Rust asks the sidebar to show the icon picker for a space. */
  onIconPicker: (cb: (space: number) => void): Promise<UnlistenFn> =>
    listen<number>("browser:icon-picker", (e) => cb(e.payload)),
  /** Rust asks the sidebar to start inline-renaming a space or folder. */
  /** The context menu's Rename… (Rust sends spaces, folders and tabs). */
  onRename: (cb: (target: { kind: "space" | "folder" | "tab"; id: number }) => void): Promise<UnlistenFn> =>
    listen<{ kind: "space" | "folder" | "tab"; id: number }>("browser:rename", (e) => cb(e.payload)),
};

