//! The tab/space/folder model: pure data and ordering rules, no webviews.
//!
//! Arc-style layout, top to bottom:
//! - **Favorites**: icon tiles shared by every space.
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
    /// Owning space (ignored for favorites, which every space shares).
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

    fn in_group(&self, zone: Zone, space: u32, folder: Option<u32>) -> bool {
        self.zone == zone && (zone == Zone::Favorite || self.space == space) && self.folder == folder
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: u32,
    pub space: u32,
    pub name: String,
    pub open: bool,
}

pub enum ClosedTab {
    Web { url: String, space: u32 },
    Terminal { space: u32 },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    tabs: Vec<Tab>,
    spaces: Vec<Space>,
    folders: Vec<Folder>,
    active: Option<u32>,
    active_space: u32,
    sidebar_width: f64,
    sidebar_collapsed: bool,
}

pub struct State {
    pub tabs: Vec<Tab>,
    pub spaces: Vec<Space>,
    pub folders: Vec<Folder>,
    pub active: Option<u32>,
    pub active_space: u32,
    /// The tab last shown in each space, restored when switching back.
    pub last_active: HashMap<u32, u32>,
    pub sidebar_width: f64,
    pub sidebar_collapsed: bool,
    pub closed: Vec<ClosedTab>,
    next_tab_id: u32,
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
            }],
            folders: Vec::new(),
            active: None,
            active_space: 1,
            last_active: HashMap::new(),
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_collapsed: false,
            closed: Vec::new(),
            next_tab_id: 1,
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
        if self.sidebar_collapsed { 0.0 } else { self.sidebar_width }
    }

    /// The data store a tab's webview should use. Favorites always use the
    /// shared one.
    pub fn profile_for(&self, tab: &Tab) -> Option<String> {
        if tab.zone == Zone::Favorite {
            return None;
        }
        self.space(tab.space)?.profile.clone()
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
            .position(|t| t.in_group(Zone::Today, tab.space, None))
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
        let before = (self.tabs[from].zone, self.tabs[from].space, self.tabs[from].folder, from);

        let mut tab = self.tabs.remove(from);
        tab.zone = zone;
        tab.space = space;
        tab.folder = folder;
        let members: Vec<usize> = (0..self.tabs.len())
            .filter(|&i| self.tabs[i].in_group(zone, space, folder))
            .collect();
        let at = match members.get(index) {
            Some(&i) => i,
            None => members.last().map_or(self.tabs.len(), |&i| i + 1),
        };
        self.tabs.insert(at, tab);
        before != (zone, space, folder, at)
    }

    /// Tabs in the order the sidebar shows them for `space`: favorites,
    /// pinned (open folders, then loose), then Today.
    pub fn visible_order(&self, space: u32) -> Vec<u32> {
        let ids = |zone, folder| {
            self.tabs
                .iter()
                .filter(move |t| t.in_group(zone, space, folder))
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
        order.get(index + 1).or_else(|| index.checked_sub(1).and_then(|i| order.get(i))).copied()
    }

    /// Remove a Today tab (recording it for "reopen"). Kept tabs are left in
    /// place; the caller unloads them.
    pub fn remove_tab(&mut self, id: u32) -> Option<Tab> {
        let index = self.tabs.iter().position(|t| t.id == id)?;
        if self.tabs[index].is_kept() {
            return Some(self.tabs[index].clone());
        }
        let tab = self.tabs.remove(index);
        self.closed.push(match tab.kind {
            TabKind::Web => ClosedTab::Web {
                url: tab.url.clone(),
                space: tab.space,
            },
            TabKind::Terminal => ClosedTab::Terminal { space: tab.space },
        });
        if self.closed.len() > MAX_CLOSED_TABS {
            self.closed.remove(0);
        }
        Some(tab)
    }

    // --- Spaces ----------------------------------------------------------

    pub fn add_space(&mut self) -> u32 {
        let id = self.next_group_id();
        let used: Vec<&str> = self.spaces.iter().map(|s| s.color.as_str()).collect();
        let color = SPACE_COLORS
            .iter()
            .find(|c| !used.contains(c))
            .unwrap_or(&SPACE_COLORS[self.spaces.len() % SPACE_COLORS.len()]);
        self.spaces.push(Space {
            id,
            name: format!("Space {}", self.spaces.len() + 1),
            color: (*color).into(),
            profile: None,
        });
        id
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
            .or_else(|| order.iter().copied().find(|id| self.tab(*id).is_some_and(|t| t.zone != Zone::Favorite)))
    }

    /// The space `delta` positions from the active one, wrapping.
    pub fn space_at_offset(&self, delta: isize) -> Option<u32> {
        let index = self.spaces.iter().position(|s| s.id == self.active_space)? as isize;
        let len = self.spaces.len() as isize;
        Some(self.spaces[(index + delta).rem_euclid(len) as usize].id)
    }

    /// Delete a space and its tabs and folders (never the last space).
    /// Returns the removed tabs and the space, for webview/PTY cleanup.
    pub fn remove_space(&mut self, id: u32) -> Option<(Vec<Tab>, Space)> {
        if self.spaces.len() <= 1 {
            return None;
        }
        let index = self.spaces.iter().position(|s| s.id == id)?;
        let space = self.spaces.remove(index);
        let (removed, kept): (Vec<Tab>, Vec<Tab>) = std::mem::take(&mut self.tabs)
            .into_iter()
            .partition(|t| t.zone != Zone::Favorite && t.space == id);
        self.tabs = kept;
        self.folders.retain(|f| f.space != id);
        self.last_active.remove(&id);
        self.closed.retain(|c| match c {
            ClosedTab::Web { space, .. } | ClosedTab::Terminal { space } => *space != id,
        });
        Some((removed, space))
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
                let space = t.space.filter(|s| state.space(*s).is_some()).unwrap_or(first_space);
                let mut zone = t.zone.unwrap_or(if t.pinned { Zone::Favorite } else { Zone::Today });
                if t.kind == TabKind::Terminal {
                    zone = Zone::Today;
                }
                let folder = t.folder.filter(|f| {
                    zone == Zone::Pinned && state.folders.iter().any(|x| x.id == *f && x.space == space)
                });
                let mut tab = Tab::new(id, t.kind, space, t.url, t.title);
                tab.zone = zone;
                tab.folder = folder;
                state.tabs.push(tab);
                id
            })
            .collect();
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
            active_space: Some(self.active_space),
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
    pub active_space: Option<u32>,
    /// `(space id, index into tabs)`.
    #[serde(default)]
    pub last_active: Vec<(u32, usize)>,
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
        ids.iter().map(|id| state.tab(*id).unwrap().title.clone()).collect()
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
        assert!(!s.move_tab(a, Zone::Pinned, None, 0), "no-op move reports no change");
    }

    #[test]
    fn terminals_stay_in_today() {
        let mut s = State::default();
        let t = s.add_tab(TabKind::Terminal, String::new(), "t".into());
        assert!(!s.move_tab(t, Zone::Favorite, None, 0));
        assert_eq!(s.tab(t).unwrap().zone, Zone::Today);
    }

    #[test]
    fn favorites_are_shared_across_spaces() {
        let mut s = State::default();
        let fav = web(&mut s, "fav");
        s.move_tab(fav, Zone::Favorite, None, 0);
        let home_tab = web(&mut s, "home");
        let work = s.add_space();
        s.switch_space(work);
        assert_eq!(titles(&s, &s.visible_order(work)), ["fav"]);
        assert!(!s.visible_order(work).contains(&home_tab));
    }

    #[test]
    fn switching_spaces_restores_last_tab() {
        let mut s = State::default();
        let a = web(&mut s, "a");
        let b = web(&mut s, "b");
        s.focus(a);
        let work = s.add_space();
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
        let work = s.add_space();
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
        assert!(matches!(s.closed.last(), Some(ClosedTab::Web { .. })));
    }

    #[test]
    fn removing_a_space_drops_its_tabs_but_not_favorites() {
        let mut s = State::default();
        let work = s.add_space();
        s.switch_space(work);
        let w = web(&mut s, "w");
        let fav = web(&mut s, "fav");
        s.move_tab(fav, Zone::Favorite, None, 0);
        s.add_folder(work, "f".into());
        let (removed, _) = s.remove_space(work).unwrap();
        assert_eq!(removed.iter().map(|t| t.id).collect::<Vec<_>>(), [w]);
        assert!(s.tab(fav).is_some());
        assert!(s.folders.is_empty());
        assert!(s.remove_space(1).is_none(), "the last space can't be removed");
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
        let work = s.add_space();
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
        assert_eq!((rw.zone, rw.space, rw.folder), (Zone::Pinned, work, Some(f)));
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

    impl State {
        fn add_space_id_is_fresh(&self) -> bool {
            let mut copy = State {
                next_group_id: self.next_group_id,
                spaces: self.spaces.clone(),
                ..State::default()
            };
            let id = copy.add_space();
            !self.spaces.iter().any(|s| s.id == id) && !self.folders.iter().any(|f| f.id == id)
        }
    }
}
