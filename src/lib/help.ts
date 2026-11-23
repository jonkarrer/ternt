// What ⌃H shows (the help sheet in the command bar overlay) and Settings →
// Shortcuts lists. Mirrors the menu accelerators in src-tauri/src/menu.rs.

export type Shortcut = { keys: string; action: string };
export type ShortcutGroup = { title: string; items: Shortcut[] };
export type HowTo = { title: string; text: string };

const group = (title: string, items: [keys: string, action: string][]): ShortcutGroup => ({
  title,
  items: items.map(([keys, action]) => ({ keys, action })),
});

export const SHORTCUTS: ShortcutGroup[] = [
  group("Tabs", [
    ["⌘T", "Command bar: open a URL, search, switch tabs or spaces, run commands"],
    ["⌥⌘T", "New terminal tab"],
    ["⌘L", "Edit the current URL"],
    ["⌘W", "Close tab (unloads favorites and pinned tabs)"],
    ["⇧⌘T", "Reopen closed tab"],
    ["⌃Tab  ⌃⇧Tab", "Next / previous tab"],
    ["⌘1 – ⌘9", "Select tab (⌘9: the last one)"],
  ]),
  group("Pages", [
    ["⌘[  ⌘]", "Back / forward"],
    ["⌘R", "Reload"],
    ["⌥⌘I", "Developer Tools (also right-click → Inspect Element)"],
    ["⌥⌘\\", "Split view: a new terminal beside the page"],
  ]),
  group("Terminals", [
    ["⌘D", "Split Right: a new terminal pane beside this one"],
    ["⇧⌘D", "Split Down: a new terminal pane below this one"],
    ["⌘+ / ⌘− / ⌘0", "Terminal text bigger / smaller / back to normal"],
    ["⌘[  ⌘]", "Previous / next pane"],
    ["⌘W", "Close the focused pane"],
  ]),
  group("Sidebar and spaces", [
    ["⌘S", "Toggle sidebar"],
    ["⌃F", "Explore tabs: j/k move, h/l switch space, Space peeks, Enter opens"],
    ["⌃1 – ⌃9", "Select space"],
    ["⌥⌘←  ⌥⌘→", "Previous / next space"],
  ]),
  group("Agent", [
    ["⌃A", "Ask Tokidoki: this space's agent, in a panel over the page. In the command bar, tok ⇥ and what you type starts a conversation with it"],
  ]),
  group("App", [
    ["⌃H", "Shortcuts and how-tos (this sheet)"],
    ["⌘H", "Hide Ternt"],
    ["⌘,", "Settings"],
    ["⌥⌘H", "Hide other apps"],
    ["⌘Q", "Quit"],
  ]),
];

export const HOW_TOS: HowTo[] = [
  {
    title: "Ask your space's agent",
    text: "Press ⌃A. Or press ⌘T, type tok, press Tab and type what you want to ask: Enter starts a new conversation with it. Each space has its own agent with its own memory, so a dev space and a marketing space never mix. Enter sends, Shift+Enter adds a line, Esc closes; the + button starts a new conversation. Pick the provider and paste your own API key in Settings → Agent (⌘,): Fireworks AI or Anthropic. The key is kept in your Keychain and goes only to the agent process. In a build with the stand-in agent (marked stub) no key is needed.",
  },
  {
    title: "Search a site",
    text: "Press ⌘T, type the start of a site's name and press Tab: you ⇥ YouTube, goo ⇥ Google, perp ⇥ Perplexity (also ChatGPT, Claude, GitHub, Wikipedia, Maps, Amazon, Reddit). What you type next is searched there. Backspace or Esc removes it.",
  },
  {
    title: "Run a command",
    text: "Press ⌘T and type > followed by the command: it runs in a new terminal.",
  },
  {
    title: "Rename a tab",
    text: "Double-click its title in the sidebar (or right-click → Rename Tab…). Clear the name to go back to the page's title.",
  },
  {
    title: "Keep a tab",
    text: "Drag it to the top of the sidebar to make it a favorite, or above the divider to pin it. Closing a kept tab only unloads it.",
  },
  {
    title: "Organize with folders",
    text: "Use the folder button by the space name. Drag tabs onto a folder or between its tabs, and drag a folder by its header to reorder.",
  },
  {
    title: "Group tabs side by side",
    text: "Drag a tab from the sidebar onto the page: drop it on the half of a tab where it should go (up to four tabs, pages or terminals). Click the group's row to show it, one of its tabs to show just that one. Drag the gaps to resize.",
  },
  {
    title: "Split a terminal",
    text: "⌘D splits right, ⇧⌘D splits down, or use the buttons on the terminal's row. New panes start in the same folder; drag the line between panes to resize.",
  },
  {
    title: "Show a hidden sidebar",
    text: "With the sidebar hidden (⌘S), move the pointer to the window's left edge: it slides out over the page.",
  },
  {
    title: "Switch spaces",
    text: "Swipe two fingers left across the sidebar for the next space, right for the previous one; scroll sideways (or Shift-scroll) with a mouse, or press ⌃1–⌃9.",
  },
  {
    title: "Separate logins",
    text: "Each new space can have its own profile (cookies and logins). Right-click a space → Profile to share one instead.",
  },
  {
    title: "Links from other apps",
    text: "Make Ternt your default browser (Ternt menu → Make Default Browser, or the button in Settings): links open in a small window, with a button to move them into Ternt.",
  },
];

/** The shortcuts and how-tos matching every word of `query` (all of them for none). */
export function filterHelp(query: string): { shortcuts: ShortcutGroup[]; howTos: HowTo[] } {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const matches = (...texts: string[]) => {
    const haystack = texts.join(" ").toLowerCase();
    return words.every((w) => haystack.includes(w));
  };
  return {
    shortcuts: SHORTCUTS.map((g) => ({
      ...g,
      // A group's title matches all of its shortcuts.
      items: g.items.filter((s) => matches(g.title, s.keys, s.action)),
    })).filter((g) => g.items.length),
    howTos: HOW_TOS.filter((h) => matches(h.title, h.text)),
  };
}
