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
    pub title: String,
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
}

impl Tab {
    pub fn new(id: u32, kind: TabKind, space: u32, url: String, title: String) -> Self {
        Self {
            id,
            kind,
            url,
            title,
            zone: Zone::Today,
            space,
            folder: None,
            loaded: false,
            loading: false,
            can_go_back: false,
            can_go_forward: false,
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

/// Two tabs shown side by side, in the same space (usually `right` right
/// below `left` in the sidebar). At most one is a terminal: terminals share
/// one webview.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Split {
    pub left: u32,
    pub right: u32,
    /// The left pane's share of the width.
    pub ratio: f64,
}

pub const MIN_SPLIT_RATIO: f64 = 0.2;
pub const MAX_SPLIT_RATIO: f64 = 0.8;

impl Split {
    pub fn has(&self, tab: u32) -> bool {
        self.left == tab || self.right == tab
    }

    pub fn partner(&self, tab: u32) -> Option<u32> {
        if self.left == tab {
            Some(self.right)
        } else if self.right == tab {
            Some(self.left)
        } else {
            None
        }
    }
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
    pub space: u32,
}

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
    splits: Vec<Split>,
    active: Option<u32>,
    active_space: u32,
    sidebar_width: f64,
    sidebar_collapsed: bool,
}

pub struct State {
    pub tabs: Vec<Tab>,
    pub spaces: Vec<Space>,
    pub folders: Vec<Folder>,
    /// Names given to separate profiles, by profile id.
    pub profile_names: HashMap<String, String>,
    pub splits: Vec<Split>,
    pub active: Option<u32>,
    pub active_space: u32,
    /// The tab last shown in each space, restored when switching back.
    pub last_active: HashMap<u32, u32>,
    pub sidebar_width: f64,
    pub sidebar_collapsed: bool,
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
            splits: Vec::new(),
            active: None,
            active_space: 1,
            last_active: HashMap::new(),
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_collapsed: false,
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
            splits: self.splits.clone(),
            active: self.active,
            active_space: self.active_space,
            sidebar_width: self.sidebar_width,
            sidebar_collapsed: self.sidebar_collapsed,
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
        self.prune_splits();
        before != (zone, space, folder, at)
    }

    /// Tabs in the order the sidebar shows them for `space`: favorites,
    /// pinned (open folders, then loose), then Today.
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
        out.extend(ids(Zone::Today, None));
        out
    }

    /// Make `id` the active tab, switching to its space if needed. Returns
    /// the previously active tab, or `None` if `id` doesn't exist.
    pub fn focus(&mut self, id: u32) -> Option<Option<u32>> {
        let tab = self.tab(id)?;
        if tab.zone != Zone::Favorite {
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
        self.prune_splits();
        self.closed.push(ClosedTab {
            id: self.next_closed_id,
            kind: tab.kind,
            url: tab.url.clone(),
            title: tab.title.clone(),
            space: tab.space,
        });
        self.next_closed_id += 1;
        if self.closed.len() > MAX_CLOSED_TABS {
            self.closed.remove(0);
        }
        Some(tab)
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

    // --- Split view -------------------------------------------------------

    pub fn split_of(&self, tab: u32) -> Option<&Split> {
        self.splits.iter().find(|s| s.has(tab))
    }

    /// The tab shown beside `tab`, if it's in a split.
    pub fn split_partner(&self, tab: u32) -> Option<u32> {
        self.split_of(tab)?.partner(tab)
    }

    /// The tabs on screen with `tab`: itself, plus its split partner.
    pub fn shown_with(&self, tab: u32) -> Vec<u32> {
        match self.split_of(tab) {
            Some(split) => vec![split.left, split.right],
            None => vec![tab],
        }
    }

    /// Show `right` beside `left`; either leaves any split it was in.
    /// `right` moves right below `left` in the sidebar when it can (a
    /// terminal can't follow a pinned tab: terminals only live in Today).
    /// Refused for favorites (they belong to every space of a profile),
    /// tabs in different spaces, and two terminals.
    pub fn create_split(&mut self, left: u32, right: u32) -> bool {
        if !self.can_split(left, right) {
            return false;
        }
        let (Some(l), Some(r)) = (self.tab(left), self.tab(right)) else {
            return false;
        };
        let (zone, folder, space) = (l.zone, l.folder, l.space);
        let follows = r.kind == TabKind::Web || zone == Zone::Today;
        self.splits.retain(|s| !s.has(left) && !s.has(right));
        if follows {
            // `move_tab` counts positions without the moved tab.
            let index = self
                .tabs
                .iter()
                .filter(|t| t.id != right && self.in_group(t, zone, space, folder))
                .position(|t| t.id == left)
                .map_or(0, |i| i + 1);
            let active = std::mem::replace(&mut self.active_space, space);
            self.move_tab(right, zone, folder, index);
            self.active_space = active;
        }
        self.splits.push(Split {
            left,
            right,
            ratio: 0.5,
        });
        true
    }

    /// Whether two tabs can be shown side by side (see `create_split`).
    pub fn can_split(&self, left: u32, right: u32) -> bool {
        let (Some(l), Some(r)) = (self.tab(left), self.tab(right)) else {
            return false;
        };
        left != right
            && l.space == r.space
            && l.zone != Zone::Favorite
            && r.zone != Zone::Favorite
            && !(l.kind == TabKind::Terminal && r.kind == TabKind::Terminal)
    }

    /// End the split `tab` is in. Returns it.
    pub fn remove_split(&mut self, tab: u32) -> Option<Split> {
        let index = self.splits.iter().position(|s| s.has(tab))?;
        Some(self.splits.remove(index))
    }

    /// Set the left pane's share of the split `tab` is in (clamped).
    /// Returns the ratio applied.
    pub fn set_split_ratio(&mut self, tab: u32, ratio: f64) -> Option<f64> {
        let split = self.splits.iter_mut().find(|s| s.has(tab))?;
        split.ratio = ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
        Some(split.ratio)
    }

    /// Drop splits whose tabs are gone or can't be shown together any more
    /// (one was closed, moved to another space, or became a favorite).
    fn prune_splits(&mut self) {
        let together = |s: &State, split: &Split| {
            let (Some(l), Some(r)) = (s.tab(split.left), s.tab(split.right)) else {
                return false;
            };
            l.space == r.space && l.zone != Zone::Favorite && r.zone != Zone::Favorite
        };
        let keep: Vec<bool> = self
            .splits
            .iter()
            .map(|split| together(self, split))
            .collect();
        let mut keep = keep.into_iter();
        self.splits.retain(|_| keep.next().unwrap_or(false));
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

    /// The space `delta` positions from the active one, wrapping.
    pub fn space_at_offset(&self, delta: isize) -> Option<u32> {
        let index = self.spaces.iter().position(|s| s.id == self.active_space)? as isize;
        let len = self.spaces.len() as isize;
        Some(self.spaces[(index + delta).rem_euclid(len) as usize].id)
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
        self.prune_splits();
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
                state.tabs.push(tab);
                id
            })
            .collect();
        state.splits = saved
            .splits
            .into_iter()
            .filter_map(|(left, right, ratio)| {
                Some(Split {
                    left: *ids.get(left)?,
                    right: *ids.get(right)?,
                    ratio: ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO),
                })
            })
            .collect();
        state.prune_splits();
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
            splits: self
                .splits
                .iter()
                .filter_map(|s| Some((index(s.left)?, index(s.right)?, s.ratio)))
                .collect(),
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
    /// `(left, right, ratio)`, indexes into `tabs`.
    #[serde(default)]
    pub splits: Vec<(usize, usize, f64)>,
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
    fn spaces_cycle_and_ignore_unknown_ids() {
        let mut s = State::default();
        let two = s.add_space(None);
        let three = s.add_space(None);
        assert_eq!(s.space_at_offset(1), Some(two));
        assert_eq!(s.space_at_offset(-1), Some(three), "wraps backwards");
        s.active_space = three;
        assert_eq!(s.space_at_offset(1), Some(1), "wraps forwards");
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
    }

    fn term(state: &mut State) -> u32 {
        state.add_tab(TabKind::Terminal, String::new(), "Terminal".into())
    }

    #[test]
    fn split_puts_the_second_tab_below_the_first() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        let c = web(&mut s, "c"); // Today: c, b, a
        assert!(s.create_split(c, a));
        assert_eq!(titles(&s, &s.visible_order(1)), ["c", "a", "b"]);
        assert_eq!(s.split_partner(a), Some(c));
        assert_eq!(s.shown_with(c), [c, a]);
        assert_eq!(s.shown_with(b), [b]);

        // A tab is in one split at a time: splitting it again ends the old one.
        assert!(s.create_split(b, a));
        assert_eq!(s.splits.len(), 1);
        assert_eq!(s.split_partner(c), None);
        assert_eq!(s.split_partner(b), Some(a));
    }

    #[test]
    fn split_rules() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let t1 = term(&mut s);
        let t2 = term(&mut s);
        assert!(!s.create_split(a, a), "not with itself");
        assert!(!s.create_split(t1, t2), "two terminals share one view");
        let fav = web(&mut s, "fav");
        s.move_tab(fav, Zone::Favorite, None, 0);
        assert!(!s.create_split(a, fav), "favorites belong to every space");
        let work = s.add_space(None);
        s.switch_space(work);
        let w = web(&mut s, "w");
        assert!(!s.create_split(a, w), "same space only");

        // A pinned page and a terminal: the terminal stays in Today.
        s.switch_space(1);
        s.move_tab(a, Zone::Pinned, None, 0);
        assert!(s.create_split(a, t1));
        assert_eq!(s.tab(t1).unwrap().zone, Zone::Today);
        assert_eq!(s.split_partner(t1), Some(a));
    }

    #[test]
    fn splits_end_when_a_tab_closes_leaves_or_becomes_a_favorite() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        s.create_split(a, b);
        s.remove_tab(b);
        assert!(s.splits.is_empty(), "closed");

        let c = web(&mut s, "c");
        s.create_split(a, c);
        s.move_tab(c, Zone::Favorite, None, 0);
        assert!(s.splits.is_empty(), "favorite");

        let d = web(&mut s, "d");
        s.create_split(a, d);
        s.move_tab(d, Zone::Pinned, None, 0);
        assert_eq!(s.split_partner(a), Some(d), "pinning one keeps the split");
        assert_eq!(s.remove_split(a).map(|x| x.right), Some(d));
        assert!(s.splits.is_empty());
    }

    #[test]
    fn split_ratio_is_clamped_and_splits_are_saved() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        s.create_split(a, b);
        assert_eq!(s.set_split_ratio(b, 0.05), Some(MIN_SPLIT_RATIO));
        assert_eq!(s.set_split_ratio(a, 0.7), Some(0.7));
        assert_eq!(s.set_split_ratio(999, 0.5), None);

        let r = State::restore(
            serde_json::from_str(&serde_json::to_string(&s.save()).unwrap()).unwrap(),
        );
        let split = &r.splits[0];
        assert_eq!(
            (
                r.tab(split.left).unwrap().title.as_str(),
                r.tab(split.right).unwrap().title.as_str(),
                split.ratio
            ),
            ("a", "b", 0.7)
        );
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
