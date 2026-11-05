//! The tab/space/folder model: pure data and ordering rules, no webviews.
//!
//! Arc-style layout, top to bottom:
//! - **Favorites**: icon tiles shared by every space with the same profile.
//! - **Pinned**: per space, optionally grouped into folders.
//! - **Today**: per space, newest first.
//!
//! All tabs live in one `Vec`; a tab's group is `(zone, space, folder)` and
//! its order within the group is its order in the `Vec`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::group::{self, Group, Place};
use crate::panes::{Node, Panes};

pub const DEFAULT_SIDEBAR_WIDTH: f64 = 240.0;
const MAX_CLOSED_TABS: usize = 25;
/// Space colors, offered in this order to new spaces. The UI maps each to a
/// sidebar tint (`src/lib/spaces.ts`).
pub const SPACE_COLORS: [&str; 9] = [
    "blue", "purple", "pink", "red", "orange", "yellow", "green", "teal", "gray",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TabKind {
    Web,
    Terminal,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Zone {
    Favorite,
    Pinned,
    Today,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub id: u32,
    pub kind: TabKind,
    pub url: String,
    /// The page's (or shell's) own title.
    pub title: String,
    /// A name the user gave it, shown instead of the title.
    pub name: Option<String>,
    pub zone: Zone,
    /// Owning space. For favorites, the space they were added from: they
    /// show in every space sharing its profile.
    pub space: u32,
    /// Pinned tabs only.
    pub folder: Option<u32>,
    /// Web tabs: whether a webview currently exists for this tab.
    pub loaded: bool,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Terminals: the shells it's split into (one to start with).
    pub panes: Option<Panes>,
}

impl Tab {
    pub fn new(id: u32, kind: TabKind, space: u32, url: String, title: String) -> Self {
        Self {
            id,
            kind,
            url,
            title,
            name: None,
            zone: Zone::Today,
            space,
            folder: None,
            loaded: false,
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            panes: (kind == TabKind::Terminal).then(Panes::default),
        }
    }

    /// Favorites and pinned tabs survive "close" (they unload instead).
    pub fn is_kept(&self) -> bool {
        self.zone != Zone::Today
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: u32,
    pub name: String,
    pub color: String,
    /// Hex id of a separate WebKit data store (cookies, storage), or `None`
    /// to share the default one.
    #[serde(default)]
    pub profile: Option<String>,
    /// An emoji, or a `sym:` line symbol (see `DEFAULT_SPACE_ICONS`). Without
    /// one the space shows its initial.
    #[serde(default)]
    pub icon: Option<String>,
}

/// Longest icon accepted, in bytes: room for one emoji with modifiers
/// (flags, skin tones, ZWJ sequences), not a word.
pub const MAX_ICON_BYTES: usize = 32;

/// Line symbols new spaces start with, in order, skipping ones in use
/// (`sym:` icons are drawn by `src/lib/spaceIcons.ts`, tinted with the
/// space's color).
pub const DEFAULT_SPACE_ICONS: [&str; 8] = [
    "sym:home",
    "sym:briefcase",
    "sym:code",
    "sym:book",
    "sym:star",
    "sym:leaf",
    "sym:bolt",
    "sym:globe",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: u32,
    pub space: u32,
    pub name: String,
    pub open: bool,
}

/// The main profile's name: the default data store, shared by every space
/// that doesn't have its own.
pub const MAIN_PROFILE_NAME: &str = "Default";

/// A profile and the spaces using it (see `State::profile_groups`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileGroup {
    /// `None` for the main profile.
    pub profile: Option<String>,
    /// How the space menu selects it: `shared` or `space-<id>`.
    pub choice: String,
    /// "Default" for the main profile; a separate one is named after its
    /// spaces unless renamed.
    pub name: String,
    /// Space ids, in space order.
    pub spaces: Vec<u32>,
}

/// A closed Today tab, kept for "reopen" (⇧⌘T, the command bar).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosedTab {
    /// Its own sequence (tab ids are reused by the reopened tab).
    pub id: u32,
    pub kind: TabKind,
    pub url: String,
    pub title: String,
    pub name: Option<String>,
    pub space: u32,
}

/// A tab on screen and where (see `Snapshot::screen`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenFrame {
    pub tab: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Longest tab name kept, in characters.
pub const MAX_TAB_NAME: usize = 200;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    tabs: Vec<Tab>,
    spaces: Vec<Space>,
    folders: Vec<Folder>,
    /// The main profile first, then each separate one in space order.
    profiles: Vec<ProfileGroup>,
    /// Oldest first.
    closed: Vec<ClosedTab>,
    groups: Vec<Group>,
    /// The group on screen, if the active tab's group is showing (rather
    /// than the tab alone).
    shown_group: Option<u32>,
    /// Frames of the tabs on screen, in window coordinates (logical px);
    /// filled in by `browser.rs`, which knows the window.
    pub screen: Vec<ScreenFrame>,
    active: Option<u32>,
    active_space: u32,
    sidebar_width: f64,
    sidebar_collapsed: bool,
    sidebar_peeking: bool,
}

pub struct State {
    pub tabs: Vec<Tab>,
    pub spaces: Vec<Space>,
    pub folders: Vec<Folder>,
    /// Names given to separate profiles, by profile id.
    pub profile_names: HashMap<String, String>,
    /// Tab groups. Their tabs sit together in Today, in group order.
    pub groups: Vec<Group>,
    /// The active tab's group is on screen, rather than the tab alone.
    pub group_shown: bool,
    /// The tab last active in each group, by group id (focused when the
    /// group shows again).
    group_last: HashMap<u32, u32>,
    pub active: Option<u32>,
    pub active_space: u32,
    /// The tab last shown in each space, restored when switching back.
    pub last_active: HashMap<u32, u32>,
    pub sidebar_width: f64,
    pub sidebar_collapsed: bool,
    /// The collapsed sidebar shown over the page while the pointer is on it
    /// (not saved).
    pub sidebar_peeking: bool,
    pub closed: Vec<ClosedTab>,
    next_tab_id: u32,
    next_closed_id: u32,
    /// Spaces and folders share one id sequence.
    next_group_id: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            spaces: vec![Space {
                id: 1,
                name: "Home".into(),
                color: SPACE_COLORS[0].into(),
                profile: None,
                icon: Some(DEFAULT_SPACE_ICONS[0].into()),
            }],
            folders: Vec::new(),
            profile_names: HashMap::new(),
            groups: Vec::new(),
            group_shown: false,
            group_last: HashMap::new(),
            active: None,
            active_space: 1,
            last_active: HashMap::new(),
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_collapsed: false,
            sidebar_peeking: false,
            closed: Vec::new(),
            next_tab_id: 1,
            next_closed_id: 1,
            next_group_id: 2,
        }
    }
}

impl State {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tabs: self.tabs.clone(),
            spaces: self.spaces.clone(),
            folders: self.folders.clone(),
            profiles: self.profile_groups(),
            closed: self.closed.clone(),
            groups: self.groups.clone(),
            shown_group: self.shown_group().map(|g| g.id),
            screen: Vec::new(),
            active: self.active,
            active_space: self.active_space,
            sidebar_width: self.sidebar_width,
            sidebar_collapsed: self.sidebar_collapsed,
            sidebar_peeking: self.sidebar_peeking,
        }
    }

    pub fn tab(&self, id: u32) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn tab_mut(&mut self, id: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn space(&self, id: u32) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    pub fn space_mut(&mut self, id: u32) -> Option<&mut Space> {
        self.spaces.iter_mut().find(|s| s.id == id)
    }

    pub fn folder_mut(&mut self, id: u32) -> Option<&mut Folder> {
        self.folders.iter_mut().find(|f| f.id == id)
    }

    pub fn effective_sidebar_width(&self) -> f64 {
        if self.sidebar_collapsed {
            0.0
        } else {
            self.sidebar_width
        }
    }

    /// A space's data store: `Some(id)` for a separate profile, `None` for
    /// the shared default.
    pub fn profile_of(&self, space: u32) -> Option<&str> {
        self.space(space)?.profile.as_deref()
    }

    /// The data store a tab's webview should use: its space's.
    pub fn profile_for(&self, tab: &Tab) -> Option<String> {
        self.profile_of(tab.space).map(String::from)
    }

    /// Whether any space still uses a profile (its data must be kept).
    pub fn profile_in_use(&self, profile: &str) -> bool {
        self.spaces
            .iter()
            .any(|s| s.profile.as_deref() == Some(profile))
    }

    /// Whether `tab` belongs to the group `(zone, space, folder)`. Favorites
    /// are grouped by profile rather than by space.
    fn in_group(&self, tab: &Tab, zone: Zone, space: u32, folder: Option<u32>) -> bool {
        tab.zone == zone
            && tab.folder == folder
            && if zone == Zone::Favorite {
                self.space(tab.space).is_some()
                    && self.profile_of(tab.space) == self.profile_of(space)
            } else {
                tab.space == space
            }
    }

    fn next_group_id(&mut self) -> u32 {
        let id = self.next_group_id;
        self.next_group_id += 1;
        id
    }

    // --- Tabs ------------------------------------------------------------

    /// Add a tab at the top of the active space's Today list.
    pub fn add_tab(&mut self, kind: TabKind, url: String, title: String) -> u32 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let tab = Tab::new(id, kind, self.active_space, url, title);
        let at = self
            .tabs
            .iter()
            .position(|t| self.in_group(t, Zone::Today, tab.space, None))
            .unwrap_or(self.tabs.len());
        self.tabs.insert(at, tab);
        id
    }

    /// Move a tab into a group at `index` (clamped). Terminals only live in
    /// Today. Returns whether anything changed.
    pub fn move_tab(&mut self, id: u32, zone: Zone, folder: Option<u32>, index: usize) -> bool {
        let Some(from) = self.tabs.iter().position(|t| t.id == id) else {
            return false;
        };
        if self.tabs[from].kind == TabKind::Terminal && zone != Zone::Today {
            return false;
        }
        let space = if zone == Zone::Favorite {
            self.tabs[from].space
        } else {
            self.active_space
        };
        // A folder must exist in this space; otherwise the tab is loose.
        let folder = folder.filter(|f| {
            zone == Zone::Pinned && self.folders.iter().any(|x| x.id == *f && x.space == space)
        });
        let before = (
            self.tabs[from].zone,
            self.tabs[from].space,
            self.tabs[from].folder,
            from,
        );

        let mut tab = self.tabs.remove(from);
        tab.zone = zone;
        tab.space = space;
        tab.folder = folder;
        let members: Vec<usize> = (0..self.tabs.len())
            .filter(|&i| self.in_group(&self.tabs[i], zone, space, folder))
            .collect();
        let at = match members.get(index) {
            Some(&i) => i,
            None => members.last().map_or(self.tabs.len(), |&i| i + 1),
        };
        self.tabs.insert(at, tab);
        // Dragged away from its group's tabs: it leaves the group.
        if let Some(group) = self.group_of(id).cloned() {
            let order: Vec<u32> = self
                .tabs
                .iter()
                .filter(|t| self.in_group(t, Zone::Today, space, None))
                .map(|t| t.id)
                .collect();
            let beside = order.iter().position(|&t| t == id).is_some_and(|i| {
                let near = |j: Option<usize>| j.and_then(|j| order.get(j)).is_some_and(|&t| group.has(t));
                near(i.checked_sub(1)) || near(Some(i + 1))
            });
            if !beside {
                self.ungroup_tab(id);
            }
        }
        self.prune_groups();
        before != (zone, space, folder, at)
    }

    /// Tabs in the order the sidebar shows them for `space`: favorites,
    /// pinned (open folders, then loose), Today's pages, then terminals (at
    /// the bottom of the sidebar).
    pub fn visible_order(&self, space: u32) -> Vec<u32> {
        let ids = |zone, folder| {
            self.tabs
                .iter()
                .filter(move |t| self.in_group(t, zone, space, folder))
                .map(|t| t.id)
        };
        let mut out: Vec<u32> = ids(Zone::Favorite, None).collect();
        for folder in self.folders.iter().filter(|f| f.space == space && f.open) {
            out.extend(ids(Zone::Pinned, Some(folder.id)));
        }
        out.extend(ids(Zone::Pinned, None));
        let (terminals, pages): (Vec<u32>, Vec<u32>) = ids(Zone::Today, None).partition(|&id| {
            self.tab(id).is_some_and(|t| t.kind == TabKind::Terminal) && self.group_of(id).is_none()
        });
        out.extend(pages);
        out.extend(terminals);
        out
    }

    /// Make `id` the active tab, switching to its space if needed. Returns
    /// the previously active tab, or `None` if `id` doesn't exist. A
    /// favorite keeps the space only if it shares the space's profile: one
    /// from another profile is signed in as that profile, so its space shows.
    /// Focus moving within the group on screen keeps it there; any other
    /// tab shows alone.
    pub fn focus(&mut self, id: u32) -> Option<Option<u32>> {
        self.group_shown = self.shown_group().is_some_and(|g| g.has(id));
        if let Some(group) = self.group_of(id).map(|g| g.id) {
            self.group_last.insert(group, id);
        }
        let tab = self.tab(id)?;
        let shared_here = tab.zone == Zone::Favorite
            && self.profile_of(tab.space) == self.profile_of(self.active_space);
        if !shared_here {
            self.active_space = tab.space;
        }
        self.last_active.insert(self.active_space, id);
        Some(self.active.replace(id))
    }

    /// The tab to show after `id` goes away: the next one down in the
    /// sidebar, else the one above.
    pub fn neighbor(&self, id: u32) -> Option<u32> {
        let order = self.visible_order(self.active_space);
        let index = order.iter().position(|&t| t == id)?;
        order
            .get(index + 1)
            .or_else(|| index.checked_sub(1).and_then(|i| order.get(i)))
            .copied()
    }

    /// Remove a Today tab (recording it for "reopen"). Kept tabs are left in
    /// place; the caller unloads them.
    pub fn remove_tab(&mut self, id: u32) -> Option<Tab> {
        let index = self.tabs.iter().position(|t| t.id == id)?;
        if self.tabs[index].is_kept() {
            return Some(self.tabs[index].clone());
        }
        let tab = self.tabs.remove(index);
        self.prune_groups();
        self.closed.push(ClosedTab {
            id: self.next_closed_id,
            kind: tab.kind,
            url: tab.url.clone(),
            title: tab.title.clone(),
            name: tab.name.clone(),
            space: tab.space,
        });
        self.next_closed_id += 1;
        if self.closed.len() > MAX_CLOSED_TABS {
            self.closed.remove(0);
        }
        Some(tab)
    }

    /// Name a tab (trimmed and capped); an empty name goes back to its own
    /// title.
    pub fn rename_tab(&mut self, id: u32, name: &str) -> bool {
        let Some(tab) = self.tab_mut(id) else {
            return false;
        };
        let name: String = name.trim().chars().take(MAX_TAB_NAME).collect();
        tab.name = (!name.is_empty()).then_some(name);
        true
    }

    /// Take a closed tab back out of the list (`None`: the most recent),
    /// switching to the space it was closed from.
    pub fn take_closed(&mut self, id: Option<u32>) -> Option<ClosedTab> {
        let index = match id {
            Some(id) => self.closed.iter().position(|c| c.id == id)?,
            None => self.closed.len().checked_sub(1)?,
        };
        let closed = self.closed.remove(index);
        if self.space(closed.space).is_some() {
            self.active_space = closed.space;
        }
        Some(closed)
    }

    // --- Groups ------------------------------------------------------------

    pub fn group_of(&self, tab: u32) -> Option<&Group> {
        self.groups.iter().find(|g| g.has(tab))
    }

    fn group_mut(&mut self, id: u32) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// The group on screen: the active tab's, while it's showing.
    pub fn shown_group(&self) -> Option<&Group> {
        self.group_of(self.active?).filter(|_| self.group_shown)
    }

    /// The tabs on screen: the shown group's, else the active tab.
    pub fn on_screen(&self) -> Vec<u32> {
        match self.shown_group() {
            Some(group) => group.tabs().collect(),
            None => self.active.into_iter().collect(),
        }
    }

    /// Whether `tab` can join `target` (and `target`'s group, if any) at
    /// `place`. Both must be in the same space and not favorites (those
    /// belong to every space of a profile). Pinned tabs move to Today.
    pub fn can_place(&self, tab: u32, target: u32, place: Place) -> bool {
        let (Some(t), Some(at)) = (self.tab(tab), self.tab(target)) else {
            return false;
        };
        if t.space != at.space || t.zone == Zone::Favorite || at.zone == Zone::Favorite {
            return false;
        }
        match self.group_of(target) {
            Some(group) => group.can_place(tab, target, place),
            None => Group::new(0, String::new(), target).can_place(tab, target, place),
        }
    }

    /// Put `tab` beside `target` at `place`: into `target`'s group, or a new
    /// group of the two. `tab` leaves any other group. The group shows, with
    /// `tab` active. Returns the group's id, or `None` if it doesn't fit.
    pub fn place_tab(&mut self, tab: u32, target: u32, place: Place) -> Option<u32> {
        if !self.can_place(tab, target, place) {
            return None;
        }
        let id = match self.group_of(target) {
            Some(group) => group.id,
            None => {
                let id = self.next_group_id();
                let name = self.next_group_name();
                self.groups.push(Group::new(id, name, target));
                id
            }
        };
        if let Some(other) = self.group_of(tab).map(|g| g.id) {
            self.group_mut(other)?.remove(tab);
        }
        // Groups live in Today.
        for id in [tab, target] {
            if let Some(t) = self.tab_mut(id) {
                t.zone = Zone::Today;
                t.folder = None;
            }
        }
        self.group_mut(id)?.place(tab, target, place);
        self.prune_groups();
        self.focus(tab);
        self.group_shown = true;
        Some(id)
    }

    /// "Group N", the lowest N not taken.
    fn next_group_name(&self) -> String {
        (1..)
            .map(|n| format!("Group {n}"))
            .find(|name| !self.groups.iter().any(|g| &g.name == name))
            .unwrap_or_default()
    }

    /// Take a tab out of its group; the rest fill its space (a group of one
    /// ends). Returns whether it was in one.
    pub fn ungroup_tab(&mut self, tab: u32) -> bool {
        let Some(id) = self.group_of(tab).map(|g| g.id) else {
            return false;
        };
        self.group_mut(id).map(|g| g.remove(tab));
        self.prune_groups();
        true
    }

    /// End a group: its tabs stay, each on its own.
    pub fn remove_group(&mut self, id: u32) -> Option<Group> {
        let index = self.groups.iter().position(|g| g.id == id)?;
        Some(self.groups.remove(index))
    }

    pub fn rename_group(&mut self, id: u32, name: &str) {
        let name = name.trim();
        if let (Some(group), false) = (self.group_mut(id), name.is_empty()) {
            group.name = name.chars().take(MAX_TAB_NAME).collect();
        }
    }

    /// Show a group: its tabs together, with the one last active focused.
    pub fn show_group(&mut self, id: u32) -> Option<u32> {
        let group = self.groups.iter().find(|g| g.id == id)?;
        let tab = self
            .group_last
            .get(&id)
            .copied()
            .filter(|t| group.has(*t))
            .or_else(|| group.tabs().next())?;
        self.focus(tab);
        self.group_shown = true;
        Some(tab)
    }

    /// Resize a group's columns (`column: None`) or a column's rows.
    pub fn set_group_ratio(&mut self, id: u32, column: Option<usize>, ratio: f64) -> Option<f64> {
        self.group_mut(id)?.set_ratio(column, ratio)
    }

    /// Keep groups to what can be shown together: tabs that still exist,
    /// in Today, in one space (moving a tab elsewhere takes it out); groups
    /// of fewer than two end. Then sit
    /// each group's tabs together in Today, in group order, where its first
    /// tab is.
    fn prune_groups(&mut self) {
        let mut groups = std::mem::take(&mut self.groups);
        for group in &mut groups {
            let space = group.tabs().find_map(|t| self.tab(t)).map(|t| t.space);
            let gone: Vec<u32> = group
                .tabs()
                .filter(|&t| {
                    self.tab(t)
                        .is_none_or(|x| x.zone != Zone::Today || Some(x.space) != space)
                })
                .collect();
            for t in gone {
                group.remove(t);
            }
        }
        groups.retain(|g| g.len() > 1);
        for group in &groups {
            let ids: Vec<u32> = group.tabs().collect();
            // None of them comes before the first, so they all go back in
            // at its index.
            let Some(at) = self.tabs.iter().position(|t| ids.contains(&t.id)) else {
                continue;
            };
            let members: Vec<Tab> = ids
                .iter()
                .filter_map(|id| {
                    let i = self.tabs.iter().position(|t| t.id == *id)?;
                    Some(self.tabs.remove(i))
                })
                .collect();
            self.tabs.splice(at..at, members);
        }
        self.groups = groups;
    }

    // --- Spaces ----------------------------------------------------------

    pub fn add_space(&mut self, profile: Option<String>) -> u32 {
        let id = self.next_group_id();
        // The first color and icon no space has yet, else the next in turn.
        let n = self.spaces.len();
        let fresh = |options: &[&str], used: Vec<&str>| {
            options
                .iter()
                .find(|o| !used.contains(o))
                .copied()
                .unwrap_or(options[n % options.len()])
                .to_string()
        };
        let color = fresh(
            &SPACE_COLORS,
            self.spaces.iter().map(|s| s.color.as_str()).collect(),
        );
        let icon = fresh(
            &DEFAULT_SPACE_ICONS,
            self.spaces
                .iter()
                .filter_map(|s| s.icon.as_deref())
                .collect(),
        );
        self.spaces.push(Space {
            id,
            name: format!("Space {}", n + 1),
            color,
            profile,
            icon: Some(icon),
        });
        id
    }

    /// Set or clear (`None`, blank) a space's icon. Returns whether anything
    /// changed; icons longer than one emoji are refused.
    pub fn set_space_icon(&mut self, id: u32, icon: Option<&str>) -> bool {
        let icon = icon.map(str::trim).filter(|i| !i.is_empty());
        if icon.is_some_and(|i| i.len() > MAX_ICON_BYTES) {
            return false;
        }
        let Some(space) = self.space_mut(id) else {
            return false;
        };
        let icon = icon.map(String::from);
        if space.icon == icon {
            return false;
        }
        space.icon = icon;
        true
    }

    /// Switch spaces. Returns the tab to show there: the one last shown in
    /// that space if it still exists, else its first tab.
    pub fn switch_space(&mut self, space: u32) -> Option<u32> {
        self.space(space)?;
        self.active_space = space;
        let order = self.visible_order(space);
        self.last_active
            .get(&space)
            .copied()
            .filter(|id| order.contains(id))
            .or_else(|| {
                order
                    .iter()
                    .copied()
                    .find(|id| self.tab(*id).is_some_and(|t| t.zone != Zone::Favorite))
            })
    }

    /// The space `delta` positions from the active one; None past the first
    /// or last space (no wrapping).
    pub fn space_at_offset(&self, delta: isize) -> Option<u32> {
        let index = self.spaces.iter().position(|s| s.id == self.active_space)?;
        let at = index.checked_add_signed(delta)?;
        self.spaces.get(at).map(|s| s.id)
    }

    /// Delete a space and its tabs and folders (never the last space). Its
    /// favorites move to another space with the same profile, or go with it.
    /// Returns the removed tabs and the space, for webview/PTY cleanup.
    pub fn remove_space(&mut self, id: u32) -> Option<(Vec<Tab>, Space)> {
        if self.spaces.len() <= 1 {
            return None;
        }
        let index = self.spaces.iter().position(|s| s.id == id)?;
        let space = self.spaces.remove(index);
        let heir = self
            .spaces
            .iter()
            .find(|s| s.profile == space.profile)
            .map(|s| s.id);
        let (removed, kept): (Vec<Tab>, Vec<Tab>) = std::mem::take(&mut self.tabs)
            .into_iter()
            .map(|mut t| {
                if t.zone == Zone::Favorite && t.space == id {
                    t.space = heir.unwrap_or(id);
                }
                t
            })
            .partition(|t| t.space == id);
        self.tabs = kept;
        self.prune_groups();
        self.folders.retain(|f| f.space != id);
        self.last_active.remove(&id);
        self.closed.retain(|c| c.space != id);
        Some((removed, space))
    }

    /// Point a space at a profile. Returns the old profile and the loaded
    /// tabs that must reload under the new one (its favorites included: they
    /// follow their space), or `None` if nothing changed.
    pub fn set_space_profile(
        &mut self,
        id: u32,
        profile: Option<String>,
    ) -> Option<(Option<String>, Vec<u32>)> {
        let space = self.space_mut(id)?;
        if space.profile == profile {
            return None;
        }
        let old = std::mem::replace(&mut space.profile, profile);
        let affected = self
            .tabs
            .iter()
            .filter(|t| t.loaded && t.space == id)
            .map(|t| t.id)
            .collect();
        Some((old, affected))
    }

    /// Every profile: the main one first (even when no space uses it), then
    /// each separate one in the order of its first space. Unnamed separate
    /// profiles are named after the spaces using them.
    pub fn profile_groups(&self) -> Vec<ProfileGroup> {
        let mut groups = vec![ProfileGroup {
            profile: None,
            choice: "shared".into(),
            name: MAIN_PROFILE_NAME.into(),
            spaces: Vec::new(),
        }];
        let mut space_names: Vec<Vec<&str>> = vec![Vec::new()];
        for space in &self.spaces {
            match groups.iter().position(|g| g.profile == space.profile) {
                Some(i) => {
                    groups[i].spaces.push(space.id);
                    space_names[i].push(&space.name);
                }
                None => {
                    groups.push(ProfileGroup {
                        profile: space.profile.clone(),
                        choice: format!("space-{}", space.id),
                        name: String::new(),
                        spaces: vec![space.id],
                    });
                    space_names.push(vec![&space.name]);
                }
            }
        }
        for (group, names) in groups.iter_mut().zip(space_names).skip(1) {
            let id = group.profile.as_deref().unwrap_or_default();
            group.name = self
                .profile_names
                .get(id)
                .cloned()
                .unwrap_or_else(|| names.join(", "));
        }
        groups
    }

    /// Name a separate profile (an empty name goes back to its spaces'
    /// names). The main profile keeps its name. Returns whether anything
    /// changed.
    pub fn rename_profile(&mut self, profile: &str, name: &str) -> bool {
        if !self.profile_in_use(profile) {
            return false;
        }
        let name = name.trim();
        if name.is_empty() {
            self.profile_names.remove(profile).is_some()
        } else {
            self.profile_names
                .insert(profile.into(), name.into())
                .as_deref()
                != Some(name)
        }
    }

    // --- Folders ---------------------------------------------------------

    pub fn add_folder(&mut self, space: u32, name: String) -> Option<u32> {
        self.space(space)?;
        let id = self.next_group_id();
        self.folders.push(Folder {
            id,
            space,
            name,
            open: true,
        });
        Some(id)
    }

    /// Move a folder to `index` among its space's folders (clamped; counted
    /// without the folder itself).
    pub fn move_folder(&mut self, id: u32, index: usize) -> bool {
        let Some(from) = self.folders.iter().position(|f| f.id == id) else {
            return false;
        };
        let folder = self.folders.remove(from);
        let space = folder.space;
        let at = self
            .folders
            .iter()
            .enumerate()
            .filter(|(_, f)| f.space == space)
            .map(|(i, _)| i)
            .nth(index)
            .or_else(|| {
                // After the space's last folder (or where it was, if alone).
                self.folders
                    .iter()
                    .rposition(|f| f.space == space)
                    .map(|i| i + 1)
            })
            .unwrap_or(from.min(self.folders.len()));
        self.folders.insert(at, folder);
        true
    }

    /// Delete a folder; its tabs stay pinned, outside any folder.
    pub fn remove_folder(&mut self, id: u32) -> bool {
        let before = self.folders.len();
        self.folders.retain(|f| f.id != id);
        for tab in self.tabs.iter_mut().filter(|t| t.folder == Some(id)) {
            tab.folder = None;
        }
        self.folders.len() != before
    }

    // --- Persistence -------------------------------------------------------

    /// Rebuild from a saved session (all web tabs unloaded). Accepts the
    /// pre-spaces format, where tabs only had a `pinned` flag.
    pub fn restore(saved: Saved) -> Self {
        let mut state = State {
            sidebar_width: saved.sidebar_width,
            sidebar_collapsed: saved.sidebar_collapsed,
            ..State::default()
        };
        if !saved.spaces.is_empty() {
            state.spaces = saved.spaces;
        }
        state.profile_names = saved.profile_names;
        state.folders = saved
            .folders
            .into_iter()
            .filter(|f| state.spaces.iter().any(|s| s.id == f.space))
            .collect();
        state.next_group_id = state
            .spaces
            .iter()
            .map(|s| s.id)
            .chain(state.folders.iter().map(|f| f.id))
            .max()
            .unwrap_or(0)
            + 1;
        let first_space = state.spaces[0].id;
        state.active_space = saved
            .active_space
            .filter(|id| state.space(*id).is_some())
            .unwrap_or(first_space);

        let ids: Vec<u32> = saved
            .tabs
            .into_iter()
            .map(|t| {
                let id = state.next_tab_id;
                state.next_tab_id += 1;
                let space = t
                    .space
                    .filter(|s| state.space(*s).is_some())
                    .unwrap_or(first_space);
                let mut zone = t.zone.unwrap_or(if t.pinned {
                    Zone::Favorite
                } else {
                    Zone::Today
                });
                if t.kind == TabKind::Terminal {
                    zone = Zone::Today;
                }
                let folder = t.folder.filter(|f| {
                    zone == Zone::Pinned
                        && state.folders.iter().any(|x| x.id == *f && x.space == space)
                });
                let mut tab = Tab::new(id, t.kind, space, t.url, t.title);
                tab.zone = zone;
                tab.folder = folder;
                tab.name = t.name.filter(|n| !n.trim().is_empty());
                if let (Some(panes), Some(saved)) = (&mut tab.panes, t.panes) {
                    *panes = Panes::restore(saved);
                }
                state.tabs.push(tab);
                id
            })
            .collect();
        let index = |i: &usize| ids.get(*i).copied();
        let legacy = saved.splits.iter().map(|&(left, right, ratio)| SavedGroup {
            name: String::new(),
            columns: vec![vec![left], vec![right]],
            ratio,
            rows: vec![0.5, 0.5],
        });
        for saved in saved.groups.into_iter().chain(legacy) {
            let columns: Vec<Vec<u32>> = saved
                .columns
                .iter()
                .map(|c| c.iter().filter_map(index).take(2).collect::<Vec<_>>())
                .filter(|c| !c.is_empty())
                .take(2)
                .collect();
            let id = state.next_group_id();
            let name = match saved.name.trim() {
                "" => state.next_group_name(),
                name => name.to_string(),
            };
            let rows = (0..columns.len())
                .map(|i| saved.rows.get(i).copied().unwrap_or(0.5))
                .map(|r| r.clamp(group::MIN_RATIO, group::MAX_RATIO))
                .collect();
            state.groups.push(Group {
                id,
                name,
                columns,
                ratio: saved.ratio.clamp(group::MIN_RATIO, group::MAX_RATIO),
                rows,
            });
        }
        state.prune_groups();
        state.group_shown = saved.group_shown;
        for (space, index) in saved.last_active {
            if let Some(&id) = ids.get(index) {
                state.last_active.insert(space, id);
            }
        }
        state.active = saved.active.and_then(|i| ids.get(i).copied());
        state
    }

    pub fn save(&self) -> Saved {
        let index = |id: u32| self.tabs.iter().position(|t| t.id == id);
        Saved {
            tabs: self
                .tabs
                .iter()
                .map(|t| SavedTab {
                    kind: t.kind,
                    url: t.url.clone(),
                    title: t.title.clone(),
                    pinned: false,
                    zone: Some(t.zone),
                    space: Some(t.space),
                    folder: t.folder,
                    name: t.name.clone(),
                    panes: t
                        .panes
                        .as_ref()
                        .filter(|p| p.len() > 1)
                        .map(|p| p.root.clone()),
                })
                .collect(),
            active: self.active.and_then(index),
            spaces: self.spaces.clone(),
            folders: self.folders.clone(),
            // Names of deleted profiles go with them.
            profile_names: self
                .profile_names
                .iter()
                .filter(|(id, _)| self.profile_in_use(id))
                .map(|(id, name)| (id.clone(), name.clone()))
                .collect(),
            active_space: Some(self.active_space),
            groups: self
                .groups
                .iter()
                .map(|g| SavedGroup {
                    name: g.name.clone(),
                    columns: g
                        .columns
                        .iter()
                        .map(|c| c.iter().filter_map(|&t| index(t)).collect())
                        .collect(),
                    ratio: g.ratio,
                    rows: g.rows.clone(),
                })
                .collect(),
            group_shown: self.group_shown,
            splits: Vec::new(),
            last_active: self
                .last_active
                .iter()
                .filter_map(|(&space, &id)| Some((space, index(id)?)))
                .collect(),
            sidebar_width: self.sidebar_width,
            sidebar_collapsed: self.sidebar_collapsed,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTab {
    pub kind: TabKind,
    pub url: String,
    pub title: String,
    /// Pre-spaces format: pinned tabs became favorites.
    #[serde(default, skip_serializing)]
    pub pinned: bool,
    #[serde(default)]
    pub zone: Option<Zone>,
    #[serde(default)]
    pub space: Option<u32>,
    #[serde(default)]
    pub folder: Option<u32>,
    /// A name the user gave the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Terminals split into panes: the layout (each pane gets a new shell).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panes: Option<Node>,
}

/// A tab group; tabs are indexes into `Saved::tabs`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedGroup {
    pub name: String,
    pub columns: Vec<Vec<usize>>,
    pub ratio: f64,
    pub rows: Vec<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub tabs: Vec<SavedTab>,
    /// Index into `tabs`.
    pub active: Option<usize>,
    #[serde(default)]
    pub spaces: Vec<Space>,
    #[serde(default)]
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub profile_names: HashMap<String, String>,
    #[serde(default)]
    pub active_space: Option<u32>,
    /// `(space id, index into tabs)`.
    #[serde(default)]
    pub last_active: Vec<(u32, usize)>,
    /// Before groups: two tabs side by side, `(left, right, ratio)`,
    /// indexes into `tabs`. Read as groups.
    #[serde(default, skip_serializing)]
    pub splits: Vec<(usize, usize, f64)>,
    #[serde(default)]
    pub groups: Vec<SavedGroup>,
    #[serde(default)]
    pub group_shown: bool,
    pub sidebar_width: f64,
    pub sidebar_collapsed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web(state: &mut State, name: &str) -> u32 {
        state.add_tab(TabKind::Web, format!("https://{name}"), name.into())
    }

    fn titles(state: &State, ids: &[u32]) -> Vec<String> {
        ids.iter()
            .map(|id| state.tab(*id).unwrap().title.clone())
            .collect()
    }

    #[test]
    fn new_tabs_go_on_top_of_today() {
        let mut s = State::default();
        web(&mut s, "a");
        web(&mut s, "b");
        assert_eq!(titles(&s, &s.visible_order(1)), ["b", "a"]);
    }

    #[test]
    fn visible_order_is_favorites_pinned_today() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        let c = web(&mut s, "c");
        let folder = s.add_folder(1, "f".into()).unwrap();
        s.move_tab(a, Zone::Favorite, None, 0);
        s.move_tab(b, Zone::Pinned, Some(folder), 0);
        assert_eq!(titles(&s, &s.visible_order(1)), ["a", "b", "c"]);
        s.folder_mut(folder).unwrap().open = false;
        assert_eq!(titles(&s, &s.visible_order(1)), ["a", "c"]);
        let _ = c;
    }

    #[test]
    fn terminals_come_after_todays_pages() {
        let mut s = State::default();
        web(&mut s, "a");
        s.add_tab(TabKind::Terminal, String::new(), "t".into());
        web(&mut s, "b"); // today: b, t, a
        assert_eq!(titles(&s, &s.visible_order(1)), ["b", "a", "t"]);
    }

    #[test]
    fn move_within_and_between_groups() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let _b = web(&mut s, "b");
        let c = web(&mut s, "c"); // today: c, b, a
        assert!(s.move_tab(c, Zone::Today, None, 2));
        assert_eq!(titles(&s, &s.visible_order(1)), ["b", "a", "c"]);
        assert!(s.move_tab(a, Zone::Pinned, None, 0));
        assert_eq!(titles(&s, &s.visible_order(1)), ["a", "b", "c"]);
        assert!(
            !s.move_tab(a, Zone::Pinned, None, 0),
            "no-op move reports no change"
        );
    }

    #[test]
    fn terminals_stay_in_today() {
        let mut s = State::default();
        let t = s.add_tab(TabKind::Terminal, String::new(), "t".into());
        assert!(!s.move_tab(t, Zone::Favorite, None, 0));
        assert_eq!(s.tab(t).unwrap().zone, Zone::Today);
    }

    #[test]
    fn favorites_are_shared_across_spaces_with_the_same_profile() {
        let mut s = State::default();
        let fav = web(&mut s, "fav");
        s.move_tab(fav, Zone::Favorite, None, 0);
        let home_tab = web(&mut s, "home");
        let reading = s.add_space(None);
        s.switch_space(reading);
        assert_eq!(titles(&s, &s.visible_order(reading)), ["fav"]);
        assert!(!s.visible_order(reading).contains(&home_tab));

        // A separate profile has its own favorites, using its own store.
        let work = s.add_space(Some("w".into()));
        s.switch_space(work);
        let mail = web(&mut s, "mail");
        s.move_tab(mail, Zone::Favorite, None, 0);
        assert_eq!(titles(&s, &s.visible_order(work)), ["mail"]);
        assert_eq!(titles(&s, &s.visible_order(1)), ["fav", "home"]);
        assert_eq!(s.profile_for(s.tab(mail).unwrap()).as_deref(), Some("w"));
        assert_eq!(s.profile_for(s.tab(fav).unwrap()), None);
    }

    #[test]
    fn favorites_follow_their_space_profile() {
        let mut s = State::default();
        let work = s.add_space(None);
        s.switch_space(work);
        let mail = web(&mut s, "mail");
        s.move_tab(mail, Zone::Favorite, None, 0);
        s.tab_mut(mail).unwrap().loaded = true;
        let (old, affected) = s.set_space_profile(work, Some("w".into())).unwrap();
        assert_eq!((old, affected), (None, vec![mail]));
        assert!(
            s.set_space_profile(work, Some("w".into())).is_none(),
            "no-op"
        );
        assert!(!s.visible_order(1).contains(&mail));
        assert!(s.profile_in_use("w"));
    }

    #[test]
    fn removing_a_space_hands_favorites_to_its_profile() {
        let mut s = State::default();
        let work = s.add_space(Some("w".into()));
        let work2 = s.add_space(Some("w".into()));
        let solo = s.add_space(Some("solo".into()));
        let [a, b] = [work, solo].map(|space| {
            s.switch_space(space);
            let id = web(&mut s, "fav");
            s.move_tab(id, Zone::Favorite, None, 0);
            id
        });
        s.remove_space(work).unwrap();
        assert_eq!(
            s.tab(a).unwrap().space,
            work2,
            "kept by the space sharing the profile"
        );
        let (removed, space) = s.remove_space(solo).unwrap();
        assert_eq!(
            removed.iter().map(|t| t.id).collect::<Vec<_>>(),
            [b],
            "last of its profile: removed"
        );
        assert!(!s.profile_in_use(space.profile.as_deref().unwrap()));
    }

    #[test]
    fn switching_spaces_restores_last_tab() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        s.focus(a);
        let work = s.add_space(None);
        assert_eq!(s.switch_space(work), None, "empty space shows nothing");
        let w = web(&mut s, "w");
        s.focus(w);
        assert_eq!(s.switch_space(1), Some(a));
        let _ = b;
    }

    #[test]
    fn focusing_a_tab_switches_to_its_space() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let work = s.add_space(None);
        s.switch_space(work);
        s.focus(a);
        assert_eq!(s.active_space, 1);
    }

    #[test]
    fn neighbor_prefers_below_then_above() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        let c = web(&mut s, "c"); // c, b, a
        assert_eq!(s.neighbor(b), Some(a));
        assert_eq!(s.neighbor(a), Some(b));
        assert_eq!(s.neighbor(c), Some(b));
    }

    #[test]
    fn closing_keeps_favorites_and_pinned() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        s.move_tab(a, Zone::Pinned, None, 0);
        s.remove_tab(a);
        s.remove_tab(b);
        assert!(s.tab(a).is_some());
        assert!(s.tab(b).is_none());
        assert_eq!(s.closed.last().map(|c| c.title.as_str()), Some("b"));
    }

    #[test]
    fn removing_a_space_drops_its_tabs_but_not_favorites() {
        let mut s = State::default();
        let work = s.add_space(None);
        s.switch_space(work);
        let w = web(&mut s, "w");
        let fav = web(&mut s, "fav");
        s.move_tab(fav, Zone::Favorite, None, 0);
        s.add_folder(work, "f".into());
        let (removed, _) = s.remove_space(work).unwrap();
        assert_eq!(removed.iter().map(|t| t.id).collect::<Vec<_>>(), [w]);
        assert!(s.tab(fav).is_some());
        assert!(s.folders.is_empty());
        assert!(
            s.remove_space(1).is_none(),
            "the last space can't be removed"
        );
    }

    #[test]
    fn tabs_keep_a_given_name_until_it_is_cleared() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        assert!(s.rename_tab(a, "  Inbox  "));
        assert_eq!(s.tab(a).unwrap().name.as_deref(), Some("Inbox"));
        let r = State::restore(
            serde_json::from_str(&serde_json::to_string(&s.save()).unwrap()).unwrap(),
        );
        assert_eq!(
            r.tabs[0].name.as_deref(),
            Some("Inbox"),
            "saved with the session"
        );
        s.remove_tab(a);
        assert_eq!(
            s.closed[0].name.as_deref(),
            Some("Inbox"),
            "kept for reopening"
        );
        let b = web(&mut s, "b");
        assert!(s.rename_tab(b, &"x".repeat(500)));
        assert_eq!(s.tab(b).unwrap().name.as_ref().unwrap().len(), MAX_TAB_NAME);
        assert!(s.rename_tab(b, "   "));
        assert_eq!(s.tab(b).unwrap().name, None, "empty: back to its own title");
        assert!(!s.rename_tab(42, "x"));
    }

    #[test]
    fn folders_reorder_within_their_space() {
        let mut s = State::default();
        let [a, b, c] = ["a", "b", "c"].map(|n| s.add_folder(1, n.into()).unwrap());
        let work = s.add_space(None);
        let w = s.add_folder(work, "w".into()).unwrap();
        let order = |s: &State, space: u32| {
            s.folders
                .iter()
                .filter(|f| f.space == space)
                .map(|f| f.id)
                .collect::<Vec<_>>()
        };
        assert!(s.move_folder(c, 0));
        assert_eq!(order(&s, 1), [c, a, b]);
        assert!(s.move_folder(c, 1));
        assert_eq!(order(&s, 1), [a, c, b]);
        assert!(s.move_folder(a, 99), "clamped to the end");
        assert_eq!(order(&s, 1), [c, b, a]);
        assert!(s.move_folder(w, 5));
        assert_eq!(order(&s, work), [w], "a lone folder stays put");
        assert_eq!(order(&s, 1), [c, b, a], "other spaces untouched");
        assert!(!s.move_folder(42, 0));
    }

    #[test]
    fn deleting_a_folder_keeps_its_tabs_pinned() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let f = s.add_folder(1, "f".into()).unwrap();
        s.move_tab(a, Zone::Pinned, Some(f), 0);
        assert!(s.remove_folder(f));
        let tab = s.tab(a).unwrap();
        assert_eq!((tab.zone, tab.folder), (Zone::Pinned, None));
    }

    #[test]
    fn save_and_restore_round_trip() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let work = s.add_space(None);
        s.switch_space(work);
        let w = web(&mut s, "w");
        let f = s.add_folder(work, "f".into()).unwrap();
        s.move_tab(w, Zone::Pinned, Some(f), 0);
        s.focus(w);
        s.last_active.insert(1, a);

        let json = serde_json::to_string(&s.save()).unwrap();
        let r = State::restore(serde_json::from_str(&json).unwrap());
        assert_eq!(r.spaces.len(), 2);
        assert_eq!(r.active_space, work);
        let rw = r.tabs.iter().find(|t| t.title == "w").unwrap();
        assert_eq!(
            (rw.zone, rw.space, rw.folder),
            (Zone::Pinned, work, Some(f))
        );
        assert_eq!(r.active, Some(rw.id));
        assert!(r.tabs.iter().all(|t| !t.loaded));
        let ra = r.tabs.iter().find(|t| t.title == "a").unwrap();
        assert_eq!(r.last_active.get(&1), Some(&ra.id));
        assert!(r.add_space_id_is_fresh());
    }

    #[test]
    fn restores_pre_spaces_sessions() {
        let json = r#"{"tabs":[{"kind":"web","url":"https://a","title":"a","pinned":true},
            {"kind":"web","url":"https://b","title":"b","pinned":false}],
            "active":1,"sidebarWidth":300.0,"sidebarCollapsed":false}"#;
        let r = State::restore(serde_json::from_str(json).unwrap());
        assert_eq!(r.spaces.len(), 1);
        assert_eq!(r.tabs[0].zone, Zone::Favorite);
        assert_eq!(r.tabs[1].zone, Zone::Today);
        assert_eq!(r.active, Some(r.tabs[1].id));
        assert_eq!(r.sidebar_width, 300.0);
    }

    #[test]
    fn profile_groups_for_the_space_menu() {
        let mut s = State::default();
        let work = s.add_space(Some("w".into()));
        s.space_mut(work).unwrap().name = "Work".into();
        let clients = s.add_space(Some("w".into()));
        s.space_mut(clients).unwrap().name = "Clients".into();
        let solo = s.add_space(Some("s".into()));
        s.space_mut(solo).unwrap().name = "Solo".into();
        let groups: Vec<(String, String, Vec<u32>)> = s
            .profile_groups()
            .into_iter()
            .map(|g| (g.choice, g.name, g.spaces))
            .collect();
        assert_eq!(
            groups,
            [
                ("shared".into(), "Default".into(), vec![1]),
                (
                    format!("space-{work}"),
                    "Work, Clients".into(),
                    vec![work, clients]
                ),
                (format!("space-{solo}"), "Solo".into(), vec![solo]),
            ]
        );
        // With every space separate, the main profile is still offered.
        s.space_mut(1).unwrap().profile = Some("h".into());
        assert_eq!(s.profile_groups()[0].name, "Default");
        assert!(s.profile_groups()[0].spaces.is_empty());
    }

    #[test]
    fn space_icons_are_one_emoji_and_saved() {
        let mut s = State::default();
        assert_eq!(
            s.space(1).unwrap().icon.as_deref(),
            Some("sym:home"),
            "Home starts with a symbol"
        );
        let second = s.add_space(None);
        assert_eq!(
            s.space(second).unwrap().icon.as_deref(),
            Some("sym:briefcase"),
            "new spaces get the next one"
        );
        assert!(s.set_space_icon(1, Some(" 🏠 ")));
        assert_eq!(s.space(1).unwrap().icon.as_deref(), Some("🏠"));
        assert!(!s.set_space_icon(1, Some("🏠")), "same icon: no change");
        assert!(
            s.set_space_icon(1, Some("👩🏽‍💻")),
            "ZWJ sequences with skin tones fit"
        );
        assert!(!s.set_space_icon(1, Some(&"x".repeat(MAX_ICON_BYTES + 1))));
        assert!(!s.set_space_icon(99, Some("🏠")), "no such space");

        let r = State::restore(
            serde_json::from_str(&serde_json::to_string(&s.save()).unwrap()).unwrap(),
        );
        assert_eq!(r.space(1).unwrap().icon.as_deref(), Some("👩🏽‍💻"));

        assert!(s.set_space_icon(1, Some("  ")), "blank clears it");
        assert_eq!(s.space(1).unwrap().icon, None);
    }

    #[test]
    fn profiles_can_be_renamed_and_the_name_is_saved() {
        let mut s = State::default();
        let work = s.add_space(Some("w".into()));
        s.space_mut(work).unwrap().name = "Work".into();
        assert!(s.rename_profile("w", "  Job  "));
        assert!(!s.rename_profile("w", "Job"), "same name: no change");
        assert!(!s.rename_profile("gone", "x"), "only profiles in use");
        assert_eq!(s.profile_groups()[1].name, "Job");

        let r = State::restore(
            serde_json::from_str(&serde_json::to_string(&s.save()).unwrap()).unwrap(),
        );
        assert_eq!(r.profile_groups()[1].name, "Job");

        assert!(s.rename_profile("w", ""), "empty: back to the space name");
        assert_eq!(s.profile_groups()[1].name, "Work");

        // A profile no space uses any more isn't saved with its name.
        s.rename_profile("w", "Job");
        s.space_mut(work).unwrap().profile = None;
        assert!(s.save().profile_names.is_empty());
    }

    #[test]
    fn move_clamps_index_and_rejects_other_spaces_folders() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        s.move_tab(a, Zone::Pinned, None, 99);
        s.move_tab(b, Zone::Pinned, None, 99);
        assert_eq!(
            titles(&s, &s.visible_order(1)),
            ["a", "b"],
            "a too-large index appends"
        );
        let work = s.add_space(None);
        let foreign = s.add_folder(work, "elsewhere".into()).unwrap();
        s.move_tab(a, Zone::Pinned, Some(foreign), 0);
        assert_eq!(
            s.tab(a).unwrap().folder,
            None,
            "can't file into another space's folder"
        );
        s.move_tab(a, Zone::Today, Some(foreign), 0);
        assert_eq!(
            s.tab(a).unwrap().folder,
            None,
            "only pinned tabs have folders"
        );
    }

    #[test]
    fn spaces_step_without_wrapping_and_ignore_unknown_ids() {
        let mut s = State::default();
        let two = s.add_space(None);
        let three = s.add_space(None);
        assert_eq!(s.space_at_offset(1), Some(two));
        assert_eq!(s.space_at_offset(-1), None, "stops at the first space");
        s.active_space = three;
        assert_eq!(s.space_at_offset(-1), Some(two));
        assert_eq!(s.space_at_offset(1), None, "stops at the last space");
        assert_eq!(s.switch_space(999), None);
        assert_eq!(s.active_space, three, "unknown space: no change");
    }

    #[test]
    fn focusing_a_favorite_keeps_the_space() {
        let mut s = State::default();
        let fav = web(&mut s, "fav");
        s.move_tab(fav, Zone::Favorite, None, 0);
        let other = s.add_space(None);
        s.switch_space(other);
        s.focus(fav);
        assert_eq!(s.active_space, other);
        assert_eq!(s.last_active.get(&other), Some(&fav));

        // A space with its own profile doesn't show another profile's
        // favorite: focusing it goes to the favorite's space.
        let own = s.add_space(Some("p1".into()));
        s.switch_space(own);
        s.focus(fav);
        assert_eq!(s.active_space, 1);
    }

    fn term(state: &mut State) -> u32 {
        state.add_tab(TabKind::Terminal, String::new(), "Terminal".into())
    }

    #[test]
    fn grouping_tabs_sits_them_together_and_shows_the_group() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        let c = web(&mut s, "c"); // Today: c, b, a
        let g = s.place_tab(a, c, Place::Right).unwrap();
        assert_eq!(titles(&s, &s.visible_order(1)), ["c", "a", "b"]);
        assert_eq!(s.groups[0].name, "Group 1");
        assert_eq!((s.active, s.shown_group().map(|g| g.id)), (Some(a), Some(g)));
        assert_eq!(s.on_screen(), [c, a]);

        // A third beside them; focus moving inside keeps the group up.
        s.place_tab(b, a, Place::Bottom).unwrap();
        assert_eq!(s.on_screen(), [c, a, b]);
        s.focus(c);
        assert_eq!(s.on_screen(), [c, a, b]);
        // Another tab shows alone; showing the group again focuses the last.
        let d = web(&mut s, "d");
        s.focus(d);
        assert_eq!(s.on_screen(), [d]);
        assert_eq!(s.show_group(g), Some(c));
        assert_eq!(s.on_screen(), [c, a, b]);
        // A tab of the group, alone.
        s.group_shown = false;
        s.focus(a);
        assert_eq!(s.on_screen(), [a]);

        // New groups take the next free name.
        let e = web(&mut s, "e");
        s.place_tab(e, d, Place::Left).unwrap();
        assert_eq!(s.groups[1].name, "Group 2");
        s.rename_group(g, "  Research ");
        assert_eq!(s.groups[0].name, "Research");
    }

    #[test]
    fn group_rules() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let t1 = term(&mut s);
        let t2 = term(&mut s);
        assert!(!s.can_place(a, a, Place::Right), "not beside itself");
        assert!(s.place_tab(t2, t1, Place::Right).is_some(), "terminals group too");
        assert_eq!(titles(&s, &s.visible_order(1)), ["Terminal", "Terminal", "a"], "with the pages");
        let fav = web(&mut s, "fav");
        s.move_tab(fav, Zone::Favorite, None, 0);
        assert!(!s.can_place(fav, a, Place::Right), "favorites belong to every space");
        let work = s.add_space(None);
        s.switch_space(work);
        let w = web(&mut s, "w");
        assert!(!s.can_place(w, a, Place::Right), "same space only");

        // A pinned tab joins in Today.
        s.switch_space(1);
        s.move_tab(a, Zone::Pinned, None, 0);
        let p = web(&mut s, "p");
        s.place_tab(p, a, Place::Right).unwrap();
        assert_eq!(s.tab(a).unwrap().zone, Zone::Today);
    }

    #[test]
    fn tabs_leave_groups_by_closing_moving_or_ungrouping() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        let c = web(&mut s, "c");
        s.place_tab(b, a, Place::Right);
        s.place_tab(c, b, Place::Bottom);
        s.remove_tab(c);
        assert_eq!(s.groups[0].columns, [vec![a], vec![b]], "closed: the rest fill in");
        s.move_tab(b, Zone::Pinned, None, 0);
        assert!(s.groups.is_empty(), "pinned away; a group of one ends");

        let d = web(&mut s, "d");
        let e = web(&mut s, "e");
        let f = web(&mut s, "f"); // Today: f, e, d, a
        s.place_tab(d, a, Place::Right);
        s.place_tab(e, a, Place::Bottom);
        assert_eq!(titles(&s, &s.visible_order(1)), ["b", "f", "a", "e", "d"]);
        s.move_tab(e, Zone::Today, None, 0);
        assert_eq!(s.groups[0].columns, [vec![a], vec![d]], "dragged out of the block");
        s.move_tab(f, Zone::Today, None, 3);
        assert_eq!(titles(&s, &s.visible_order(1)), ["b", "e", "a", "d", "f"], "a stranger can't split a group");
        assert!(s.ungroup_tab(d) && s.groups.is_empty());
    }

    #[test]
    fn groups_are_saved_with_their_layout() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        let c = web(&mut s, "c");
        let g = s.place_tab(b, a, Place::Right).unwrap();
        s.place_tab(c, b, Place::Bottom);
        s.rename_group(g, "Work");
        assert_eq!(s.set_group_ratio(g, None, 0.05), Some(group::MIN_RATIO));
        assert_eq!(s.set_group_ratio(g, Some(1), 0.7), Some(0.7));

        let r = State::restore(serde_json::from_str(&serde_json::to_string(&s.save()).unwrap()).unwrap());
        let g = &r.groups[0];
        let names: Vec<Vec<&str>> = g
            .columns
            .iter()
            .map(|c| c.iter().map(|&t| r.tab(t).unwrap().title.as_str()).collect())
            .collect();
        assert_eq!(names, [vec!["a"], vec!["b", "c"]]);
        assert_eq!((g.name.as_str(), g.ratio, g.rows[1]), ("Work", group::MIN_RATIO, 0.7));
        assert!(r.group_shown);

        // Sessions from before groups: a split is a group of two.
        let old = r#"{"tabs":[{"kind":"web","url":"x","title":"x"},{"kind":"web","url":"y","title":"y"}],
            "active":0,"splits":[[0,1,0.3]],"sidebarWidth":240,"sidebarCollapsed":false}"#;
        let r = State::restore(serde_json::from_str(old).unwrap());
        assert_eq!((r.groups[0].name.as_str(), r.groups[0].ratio, r.groups[0].len()), ("Group 1", 0.3, 2));
    }

    #[test]
    fn closed_tabs_reopen_by_id_or_most_recent_first() {
        let mut s = State::default();
        for name in ["a", "b", "c"] {
            let id = web(&mut s, name);
            s.remove_tab(id);
        }
        let b = s.closed.iter().find(|c| c.title == "b").unwrap().id;
        assert_eq!(s.take_closed(Some(b)).unwrap().url, "https://b");
        assert_eq!(s.take_closed(None).unwrap().title, "c");
        assert_eq!(s.take_closed(Some(b)).map(|c| c.id), None, "taken once");
        assert_eq!(s.take_closed(None).unwrap().title, "a");
        assert!(s.take_closed(None).is_none());
    }

    #[test]
    fn terminal_panes_are_saved_and_restored() {
        use crate::panes::Direction;
        let mut s = State::default();
        s.add_tab(TabKind::Terminal, String::new(), "one".into());
        let split = s.add_tab(TabKind::Terminal, String::new(), "two".into());
        let panes = s.tab_mut(split).unwrap().panes.as_mut().unwrap();
        panes.split(Direction::Right);
        panes.split(Direction::Down);
        let saved = s.save();
        assert!(
            saved.tabs.iter().any(|t| t.panes.is_none()),
            "one pane: nothing saved"
        );
        let r =
            State::restore(serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap());
        let panes = |title: &str| {
            r.tabs
                .iter()
                .find(|t| t.title == title)
                .unwrap()
                .panes
                .clone()
                .unwrap()
        };
        assert_eq!(panes("one"), Panes::default());
        let two = panes("two");
        assert_eq!(two.len(), 3, "the layout comes back");
        assert_eq!(two.focused, two.ids()[0]);
    }

    #[test]
    fn closed_tabs_are_capped_and_forgotten_with_their_space() {
        let mut s = State::default();
        for i in 0..30 {
            let id = web(&mut s, &i.to_string());
            s.remove_tab(id);
        }
        assert_eq!(s.closed.len(), MAX_CLOSED_TABS);
        let work = s.add_space(None);
        s.switch_space(work);
        let w = web(&mut s, "w");
        s.focus(w);
        s.remove_tab(w);
        s.remove_space(work);
        assert!(s.closed.iter().all(|c| c.space != work));
        assert!(!s.last_active.contains_key(&work));
    }

    #[test]
    fn restore_drops_dangling_references() {
        let json = r#"{"tabs":[
            {"kind":"web","url":"https://a","title":"a","zone":"pinned","space":7,"folder":9},
            {"kind":"terminal","url":"","title":"t","zone":"favorite","space":1}],
            "spaces":[{"id":1,"name":"Home","color":"blue"}],
            "folders":[{"id":9,"space":7,"name":"gone","open":true}],
            "active":5,"activeSpace":7,"lastActive":[[1,8]],
            "sidebarWidth":240.0,"sidebarCollapsed":false}"#;
        let r = State::restore(serde_json::from_str(json).unwrap());
        assert_eq!(
            r.active_space, 1,
            "unknown active space falls back to the first"
        );
        assert!(
            r.folders.is_empty(),
            "folders of unknown spaces are dropped"
        );
        assert_eq!((r.tabs[0].space, r.tabs[0].folder), (1, None));
        assert_eq!(r.tabs[1].zone, Zone::Today, "terminals only live in Today");
        assert_eq!(r.active, None);
        assert!(r.last_active.is_empty());
    }

    impl State {
        fn add_space_id_is_fresh(&self) -> bool {
            let mut copy = State {
                next_group_id: self.next_group_id,
                spaces: self.spaces.clone(),
                ..State::default()
            };
            let id = copy.add_space(None);
            !self.spaces.iter().any(|s| s.id == id) && !self.folders.iter().any(|f| f.id == id)
        }
    }
}
