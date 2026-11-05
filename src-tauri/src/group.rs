//! Tab groups: up to four tabs shown together, composed by dragging tabs
//! onto the content area. A group is one or two columns, each one or two
//! tabs stacked, which covers 50/50 (side by side or stacked), 50/25/25,
//! 25/25/50 and quarters. Pure data and layout math; `model.rs` keeps the
//! groups with the tabs, `browser.rs` places the views.

use serde::Serialize;

pub const MAX_TABS: usize = 4;
pub const MIN_RATIO: f64 = 0.2;
pub const MAX_RATIO: f64 = 0.8;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: u32,
    pub name: String,
    /// Left to right; each column's tabs top to bottom.
    pub columns: Vec<Vec<u32>>,
    /// The first column's share of the width.
    pub ratio: f64,
    /// Each column's top tab's share of its height.
    pub rows: Vec<f64>,
}

/// Where a dropped tab goes, beside the tab it's dropped on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Left,
    Right,
    Top,
    Bottom,
}

/// A frame in logical pixels: `(x, y, width, height)`.
pub type Frame = (f64, f64, f64, f64);

impl Group {
    /// A group of one tab, to drop others beside.
    pub fn new(id: u32, name: String, tab: u32) -> Self {
        Self {
            id,
            name,
            columns: vec![vec![tab]],
            ratio: 0.5,
            rows: vec![0.5],
        }
    }

    /// Its tabs, column by column.
    pub fn tabs(&self) -> impl Iterator<Item = u32> + '_ {
        self.columns.iter().flatten().copied()
    }

    pub fn len(&self) -> usize {
        self.columns.iter().map(Vec::len).sum()
    }

    pub fn has(&self, tab: u32) -> bool {
        self.tabs().any(|t| t == tab)
    }

    fn column_of(&self, tab: u32) -> Option<usize> {
        self.columns.iter().position(|c| c.contains(&tab))
    }

    /// Whether `tab` can go at `place` beside `target` (a member). A tab
    /// already in the group moves.
    pub fn can_place(&self, tab: u32, target: u32, place: Place) -> bool {
        let mut trial = self.clone();
        trial.remove(tab);
        tab != target && trial.has(target) && trial.len() < MAX_TABS && trial.place(tab, target, place)
    }

    /// Put `tab` at `place` beside `target`: left or right makes a second
    /// column (only one column so far), top or bottom stacks it in
    /// `target`'s column (only if that holds one tab). Returns whether it
    /// fit; on `false` nothing changed.
    pub fn place(&mut self, tab: u32, target: u32, place: Place) -> bool {
        let Some(column) = self.column_of(target) else {
            return false;
        };
        if self.has(tab) || self.len() >= MAX_TABS {
            return false;
        }
        match place {
            Place::Left | Place::Right => {
                if self.columns.len() > 1 {
                    return false;
                }
                let at = if place == Place::Left { 0 } else { 1 };
                self.columns.insert(at, vec![tab]);
                self.rows.insert(at, 0.5);
                self.ratio = 0.5;
            }
            Place::Top | Place::Bottom => {
                if self.columns[column].len() > 1 {
                    return false;
                }
                let at = if place == Place::Top { 0 } else { 1 };
                self.columns[column].insert(at, tab);
                self.rows[column] = 0.5;
            }
        }
        true
    }

    /// Take `tab` out; the rest fill its space. Returns whether it was in.
    pub fn remove(&mut self, tab: u32) -> bool {
        let Some(column) = self.column_of(tab) else {
            return false;
        };
        self.columns[column].retain(|&t| t != tab);
        if self.columns[column].is_empty() {
            self.columns.remove(column);
            self.rows.remove(column);
            self.ratio = 0.5;
        }
        true
    }

    /// Set the column split (`column: None`) or a column's row split
    /// (clamped). Returns the ratio applied.
    pub fn set_ratio(&mut self, column: Option<usize>, ratio: f64) -> Option<f64> {
        let ratio = ratio.clamp(MIN_RATIO, MAX_RATIO);
        match column {
            None if self.columns.len() > 1 => self.ratio = ratio,
            Some(i) if self.columns.get(i).is_some_and(|c| c.len() > 1) => self.rows[i] = ratio,
            _ => return None,
        }
        Some(ratio)
    }

    /// Each tab's frame within `area`, `gap` apart.
    pub fn frames(&self, area: Frame, gap: f64) -> Vec<(u32, Frame)> {
        let (x, y, w, h) = area;
        let split = |start: f64, size: f64, ratio: f64, parts: usize| -> Vec<(f64, f64)> {
            if parts < 2 {
                return vec![(start, size)];
            }
            let room = (size - gap).max(0.0);
            let first = (room * ratio).round();
            vec![(start, first), (start + first + gap, room - first)]
        };
        let mut out = Vec::new();
        for ((cx, cw), (column, &rows)) in split(x, w, self.ratio, self.columns.len())
            .into_iter()
            .zip(self.columns.iter().zip(&self.rows))
        {
            for ((ry, rh), &tab) in split(y, h, rows, column.len()).into_iter().zip(column) {
                out.push((tab, (cx, ry, cw, rh)));
            }
        }
        out
    }
}

/// Where a point over `frame` would drop: beside the nearest edge.
pub fn place_at(frame: Frame, point: (f64, f64)) -> Place {
    let (x, y, w, h) = frame;
    let (px, py) = ((point.0 - x) / w.max(1.0), (point.1 - y) / h.max(1.0));
    let edges = [
        (px, Place::Left),
        (1.0 - px, Place::Right),
        (py, Place::Top),
        (1.0 - py, Place::Bottom),
    ];
    edges
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, place)| place)
        .unwrap_or(Place::Right)
}

/// The half of `frame` a tab dropped at `place` would take (the overlay).
pub fn half(frame: Frame, place: Place) -> Frame {
    let (x, y, w, h) = frame;
    match place {
        Place::Left => (x, y, w / 2.0, h),
        Place::Right => (x + w / 2.0, y, w / 2.0, h),
        Place::Top => (x, y, w, h / 2.0),
        Place::Bottom => (x, y + h / 2.0, w, h / 2.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(columns: Vec<Vec<u32>>) -> Group {
        let rows = vec![0.5; columns.len()];
        Group {
            id: 1,
            name: "Group 1".into(),
            columns,
            ratio: 0.5,
            rows,
        }
    }

    #[test]
    fn composes_every_layout_up_to_four() {
        let mut g = Group::new(1, "g".into(), 1);
        assert!(g.place(2, 1, Place::Right), "50/50");
        assert_eq!(g.columns, [vec![1], vec![2]]);
        assert!(g.place(3, 2, Place::Bottom), "50/25/25");
        assert_eq!(g.columns, [vec![1], vec![2, 3]]);
        assert!(!g.place(4, 2, Place::Right), "two columns at most");
        assert!(!g.place(4, 3, Place::Top), "two rows per column");
        assert!(g.place(4, 1, Place::Top), "quarters");
        assert_eq!(g.columns, [vec![4, 1], vec![2, 3]]);
        assert!(!g.place(5, 1, Place::Bottom), "four tabs at most");

        let mut g = Group::new(1, "g".into(), 1);
        assert!(g.place(2, 1, Place::Bottom), "stacked 50/50");
        assert!(g.place(3, 1, Place::Right), "25/25/50");
        assert_eq!(g.columns, [vec![1, 2], vec![3]]);
        assert!(!g.place(9, 2, Place::Left) && g.place(4, 3, Place::Top));
        assert_eq!(g.columns, [vec![1, 2], vec![4, 3]]);
    }

    #[test]
    fn removing_a_tab_lets_the_others_fill_its_space() {
        let mut g = group(vec![vec![1, 2], vec![3]]);
        g.ratio = 0.3;
        assert!(g.remove(3));
        assert_eq!(g.columns, [vec![1, 2]]);
        assert_eq!(g.ratio, 0.5);
        assert!(g.remove(1));
        assert_eq!(g.columns, [vec![2]]);
        assert!(!g.remove(1));
    }

    #[test]
    fn can_place_moves_a_member() {
        let g = group(vec![vec![1], vec![2, 3]]);
        assert!(g.can_place(3, 1, Place::Bottom), "a member moves within the group");
        assert!(!g.can_place(1, 1, Place::Bottom), "not beside itself");
        assert!(!g.can_place(9, 7, Place::Bottom), "the target must be a member");
        assert!(g.can_place(9, 1, Place::Bottom));
        assert!(!g.can_place(9, 2, Place::Top));
    }

    #[test]
    fn frames_share_the_area_with_gaps() {
        let mut g = group(vec![vec![1], vec![2, 3]]);
        g.set_ratio(None, 0.25);
        let frames = g.frames((0.0, 0.0, 408.0, 208.0), 8.0);
        assert_eq!(
            frames,
            [
                (1, (0.0, 0.0, 100.0, 208.0)),
                (2, (108.0, 0.0, 300.0, 100.0)),
                (3, (108.0, 108.0, 300.0, 100.0)),
            ]
        );
        assert_eq!(g.set_ratio(Some(0), 0.3), None, "a single tab has no row split");
        assert_eq!(g.set_ratio(Some(1), 0.01), Some(MIN_RATIO));
    }

    #[test]
    fn drops_go_beside_the_nearest_edge() {
        let f = (100.0, 0.0, 200.0, 100.0);
        assert_eq!(place_at(f, (110.0, 50.0)), Place::Left);
        assert_eq!(place_at(f, (290.0, 40.0)), Place::Right);
        assert_eq!(place_at(f, (200.0, 5.0)), Place::Top);
        assert_eq!(place_at(f, (200.0, 95.0)), Place::Bottom);
        assert_eq!(half(f, Place::Right), (200.0, 0.0, 100.0, 100.0));
    }
}
