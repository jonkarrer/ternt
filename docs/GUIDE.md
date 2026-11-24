# Using Ternt

Everything the app does, for the person using it. For how it is built, see
[ARCHITECTURE.md](ARCHITECTURE.md).

## Install and updates

Download the `.dmg` from the [latest release](https://github.com/jonkarrer/ternt/releases/latest),
open it and drag Ternt to Applications. Needs macOS 14 or later.

The first launch opens a welcome page with the shortcuts (later: *Help → Welcome to Ternt*), and
over it a short setup window (later: *Help → Set Up Ternt…*): whether to use the AI agent and, if
so, your API keys for it. A Fireworks AI or Anthropic key is required to go on with the agent on;
a Jev key is highly recommended; with the agent off, Ternt is a browser and a terminal.
No tabs are created for you: press ⌘T to open the first one.

After that Ternt updates itself. It checks at launch, every 15 minutes, and when you come back to it
(at most every 2 minutes). A new version downloads in the background and installs when you quit;
meanwhile a card at the bottom of the sidebar says it's ready, and *Restart Now* swaps it in right
away (your tabs come back). *Later* hides the card for that version. If Ternt didn't quit normally
(a crash), the next launch installs the downloaded update and relaunches. A release published as
required (`just upload patch --required`) installs and relaunches as soon as Ternt sees it, no
click needed. *Ternt → Check for Updates…* checks right away and answers on the same card.

## Shortcuts

| Shortcut | Action |
|---|---|
| ⌘T | Command bar: open a URL, search Google, switch tabs or spaces, find pages from history or recently closed tabs, run commands (initials work: "nt" = New Terminal); `> cmd` runs `cmd` in a new terminal; the start of a site's name then Tab searches that site (`you` ⇥ YouTube, `goo` ⇥ Google, `perp` ⇥ Perplexity, `yt`, `gh`…; list in `src/lib/engines.ts`) |
| ⌘L | Edit the current URL |
| ⌥⌘T | New terminal tab |
| ⌘W / ⇧⌘T | Close tab (unloads pinned tabs; closes the focused pane of a split terminal) / reopen closed tab |
| ⌃Tab / ⌃⇧Tab, ⌘1–⌘9 | Switch tabs |
| ⌘[ / ⌘] / ⌘R | Back / forward / reload (in a split terminal, ⌘[ / ⌘] move between panes) |
| ⌥⌘I | Developer Tools: the Web Inspector for the page, in its own window (right-click → Inspect Element works too) |
| ⌘D / ⇧⌘D | Split the terminal: a new pane to the right / below |
| ⌘+ / ⌘− / ⌘0 | Terminal text bigger / smaller / back to normal (every terminal; remembered) |
| ⌘S | Toggle sidebar (hidden, it slides out over the page when the pointer touches the window's left edge) |
| ⌃F | Explore tabs: j/k or ↑/↓ move, h/l or ←/→ switch space, Space peeks, Enter opens, Esc goes back (in a terminal, ⌃F goes to the shell) |
| ⌃1–⌃9, ⌘⌥← / ⌘⌥→ | Switch spaces, stopping at the first and last (or swipe two fingers across the sidebar: left for the next space, right for the previous; with a mouse, scroll sideways or Shift-scroll over it) |
| ⌥⌘\ | A new terminal beside the current tab (in a group) |
| ⌘, | Settings (⌘1–⌘6 switch its pages, ⌘W closes it) |
| ⌃H | Shortcuts and how-tos, searchable (also in the Help menu). In a terminal, ⌃H goes to the shell (backspace) |
| ⌘H | Hide Ternt |
| ⌃A | Ask Tokidoki: this space's AI agent in a panel over the page. In the command bar (⌘T), `tok` ⇥ and what you type starts a conversation with it. One agent per space, each with its own memory (see [The agent](#the-agent)) |

## Sidebar

Arc-style, top to bottom:

- **Favorites**: tiles showing each site's app icon (its `apple-touch-icon`, else its largest icon,
  read from the open page so signed-in sites and single-page apps get theirs), shared by every
  space with the same profile.
- **Pinned**: per space, optionally grouped in folders. An open folder is a tinted block with its
  tabs indented under it; a closed one shows how many tabs it holds. Drag tabs to reorder them
  (within a folder, or in and out of one), and drag a folder by its header to reorder folders.
  No folder is needed: while dragging, a spot under the folders pins a tab on its own.
- **Today**: per space, newest first, below a divider. Hover the divider for *Clear*, which
  closes the space's Today web tabs (terminals stay; ⇧⌘T reopens).
- **Terminals**: per space, at the bottom of the sidebar under *New Terminal*. A long Today list
  pushes them down, and they scroll with it.

**Idle tabs unload**: a web tab off screen for 30 minutes lets go of its page (each loaded page
keeps a WebKit process, often 50–150 MB). It stays in the sidebar and reloads when you open it.
Tabs playing sound or using the camera or microphone stay loaded; *Settings → General* turns this
off.

**Renaming tabs**: double-click a tab's title (or right-click → *Rename Tab…*) to give it a name;
Enter saves, Escape cancels. The name replaces the page's or shell's own title in the sidebar, the
top bar and the command bar (which still finds the tab by its title), is saved with the session and
comes back with ⇧⌘T. Clear it to go back to the title.

**Groups**: show up to four tabs together, pages and terminals alike. Drag a tab from the sidebar
onto the page area: a highlight shows where it will land (the left, right, top or bottom half of the
tab under the pointer), and dropping it there makes a group, or adds to the one on screen. That
gives two side by side or stacked, one half and two quarters (50/25/25, 25/25/50), or quarters.
Right-click a tab → *Open in Split View* adds it beside the current tab; ⌥⌘\ opens a terminal
beside it. Drag the gaps between panes to resize (double-click: back to half and half), and click
into a pane to make its tab active.

In the sidebar a group is a row (named *Group 1*, *Group 2*…; double-click to rename) with its tabs
under it, all in the group's color. Click the row to show the group; click one of its tabs to show
just that tab, full size. Closing a tab, dragging it out of the group in the sidebar, or *Remove
from Group* takes it out, and the others fill its space; the × on the group's row (or *Ungroup
Tabs*) ends the group and keeps its tabs. Groups live in Today, in one space, and are saved with
the session.

**Terminal panes**: ⌘D splits a terminal tab side by side, ⇧⌘D stacks; panes nest, and the tab
stays one row in the sidebar, named after the focused pane. Each pane runs its own shell, starting
in the folder of the pane it split from. Click a pane (or ⌘[ / ⌘]) to focus it; the others fade a
little. Drag the line between panes to resize. ⌘W or `exit` closes a pane; the last one closes the
tab. The layout is saved with the session (with fresh shells on relaunch).

**Copying from a terminal**: select and ⌘C (with nothing selected, ⌘C leaves the clipboard
alone). When a program has the mouse (tmux, vim…), ⌥-drag selects. Programs that copy for you
(tmux, vim, Claude Code, also over ssh) reach the clipboard too (OSC 52); they can't read it.

**Shift+Enter** types a new line rather than submitting (in Claude Code, for one), like ⌥Enter.

**Spaces**: hover the space name for **…** (rename, icon, color, profile, delete; also a
right-click on the name or on a space in the switcher). Every space has an icon, shown next to its
name, in the switcher and in the command bar: new spaces start with a line symbol in the space's
color; *Change Icon…* picks another symbol or an emoji (or type any emoji). *Remove* shows the
space's initial.

Drag tabs between sections, into folders, or to reorder. Closing a favorite or pinned tab
unloads it (frees memory, keeps it in the sidebar). Right-click a tab, folder or space icon
for more. Double-click a space or folder name to rename it.

**Profiles** hold your sign-ins: cookies, logins (OAuth, Google, etc.) and history. The **Default**
profile is for every space without its own; each new space gets its own (macOS 14+), named after the
space, and several spaces can share one. Sign-in popups and favorites use their space's profile.
Manage them in *Settings → Profiles* (rename a profile, move spaces between profiles, change a
space's color, clear its history), or right-click a space icon → *Profile*. The sidebar just shows
spaces: a profile is the identity sites see while you're in one (say, one Gmail for work, another
for side projects).
A profile no space uses any more is deleted.

**Air Traffic Control** (Settings → Links) sends links from other apps to a space: a rule matches
by host (`github.com` takes its subdomains too; `github.com/acme` only the pages under it) or by a
regex tried against the whole URL, and the first match opens the link as a tab in its space. Links
no rule takes go to Little Ternt or a new tab, as chosen above it. A tester there shows where a
pasted link would go.

**Downloads** (download links, files a site sends to save) go to `~/Downloads`, numbered rather
than overwriting, and the Dock's Downloads stack bounces when one finishes. A box at the bottom of
the sidebar shows each one's progress (Cancel stops it); click a finished file to open it, or the
magnifier to show it in Finder. The button at the bottom left shows or hides the box, with a ring
of progress while something downloads. A link opened in a new tab that turns out to be a file
leaves no empty tab behind. Links for other apps
(`mailto:`, `tel:`, Zoom, Slack…) open in those apps.

Links and buttons that open a new window open a tab, which the page keeps as its window (it can
point it somewhere after opening it, or post a form into it); sign-in popups that ask for a size
get a window of their own. ⌘-click (or middle-click) opens a link in a tab behind the one you're
on, ⇧⌘-click shows it.

Video calls (Meet, Zoom in the browser, Cal Video…) work: the first time a site wants your camera
or microphone it asks, and remembers if you allow it (*Settings → General* lists those sites, to
remove). Sharing your screen asks through macOS's picker.

## Little Ternt

Make Ternt your default browser (*Ternt → Make Default Browser…*, the palette, or the button in *Settings → Account* or *Links*) and links you click in other
apps open in **Little Ternt**: a small window with the page's address on top, signed in with the
space you're in. Read it and close it (⌘W), or click *Open in Ternt* to move the page into a tab.
*Settings → Links* can send those links straight to a new tab instead.

## Settings

*Ternt → Settings…* (⌘,) opens an Arc-style window with pages along the top: **Account** (your
name and email, kept on this Mac, and a summary of your profiles), **General** (version, updates, sites allowed the camera and microphone),
**Profiles** (Default and each separate profile),
**Links** (default browser, where links from other apps open), **Shortcuts** and **Advanced**
(data folder).


## The agent

Every space gets its own AI agent, Tokidoki: an assistant with long-term memory. Press ⌃A, or
type `tok` ⇥ and your question in the command bar (⌘T): a panel opens over the page with that
space's conversations. Agents share nothing, so what the dev space's agent learns never reaches
the marketing space's.

**Bring your own key.** *Settings → Agent → Keys* takes your Fireworks AI and Anthropic keys and
keeps them in the macOS Keychain. They reach the agent's own process and nothing else.

**Each space picks its provider** (Fireworks AI by default) on the same page. The change reaches
the running agent at once and its conversations go on.

**Tools** are switched on or off per space there too: web search for now, through Exa, with your
Exa key under Keys.

**Jev** (an optional TypeSafe key) makes the agent's small decisions, fast and for very little:
which conversation a message belongs to, which memories are worth recalling for it, and whether
it says something worth remembering.

**Memory** on that page shows what the space's agent has learned, fact by fact, to correct or
forget, and a Wipe that forgets all of it (the conversations stay).

**Switching it off** (the switch at the top of *Settings → Agent*, or in the setup window): ⌃A
does nothing, the command bar stops offering Tokidoki and no agent process runs.

If the agent's process crashes, Ternt carries on and restarts it; the panel says so, and shows a
Restart button if it keeps failing.
