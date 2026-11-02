//! Terminal panes: one terminal tab split into several shells, side by side
//! or stacked, as a binary tree. Pure data; `browser.rs` runs the shells and
//! the terminal webview lays the tree out.

use serde::{Deserialize, Serialize};

/// Where a split puts the new pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    /// Side by side.
    Right,
    /// Stacked.
    Down,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Node {
    Pane {
        id: u32,
    },
    Split {
        id: u32,
        direction: Direction,
        /// The first child's share of the room.
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}

pub const MIN_PANE_RATIO: f64 = 0.1;
pub const MAX_PANE_RATIO: f64 = 0.9;

impl Node {
    /// Pane ids in reading order (left to right, top to bottom).
    fn panes(&self, out: &mut Vec<u32>) {
        match self {
            Node::Pane { id } => out.push(*id),
            Node::Split { first, second, .. } => {
                first.panes(out);
                second.panes(out);
            }
        }
    }

    fn first_pane(&self) -> u32 {
        match self {
            Node::Pane { id } => *id,
            Node::Split { first, .. } => first.first_pane(),
        }
    }

    fn last_pane(&self) -> u32 {
        match self {
            Node::Pane { id } => *id,
            Node::Split { second, .. } => second.last_pane(),
        }
    }

    fn pane_mut(&mut self, pane: u32) -> Option<&mut Node> {
        match self {
            Node::Pane { id } if *id == pane => Some(self),
            Node::Pane { .. } => None,
            Node::Split { first, second, .. } => {
                first.pane_mut(pane).or_else(|| second.pane_mut(pane))
            }
        }
    }

    fn split_mut(&mut self, split: u32) -> Option<&mut Node> {
        match self {
            Node::Pane { .. } => None,
            Node::Split { id, .. } if *id == split => Some(self),
            Node::Split { first, second, .. } => {
                first.split_mut(split).or_else(|| second.split_mut(split))
            }
        }
    }

    /// Replace the split holding `pane` with its other child. Returns the
    /// pane that takes over the focus: the closest one in that child.
    fn remove(&mut self, pane: u32) -> Option<u32> {
        let Node::Split { first, second, .. } = self else {
            return None;
        };
        let is = |n: &Node| matches!(n, Node::Pane { id } if *id == pane);
        let (rest, focus) = if is(first) {
            let rest = std::mem::replace(&mut **second, Node::Pane { id: 0 });
            let focus = rest.first_pane();
            (rest, focus)
        } else if is(second) {
            let rest = std::mem::replace(&mut **first, Node::Pane { id: 0 });
            let focus = rest.last_pane();
            (rest, focus)
        } else {
            return first.remove(pane).or_else(|| second.remove(pane));
        };
        *self = rest;
        Some(focus)
    }

    /// The same layout with ratios clamped and ids from `next` up, so a
    /// saved tree can't hold duplicates.
    fn renumbered(self, next: &mut u32) -> Node {
        let id = *next;
        *next += 1;
        match self {
            Node::Pane { .. } => Node::Pane { id },
            Node::Split {
                direction,
                ratio,
                first,
                second,
                ..
            } => Node::Split {
                id,
                direction,
                ratio: clamp(ratio),
                first: Box::new(first.renumbered(next)),
                second: Box::new(second.renumbered(next)),
            },
        }
    }
}

fn clamp(ratio: f64) -> f64 {
    if ratio.is_finite() {
        ratio.clamp(MIN_PANE_RATIO, MAX_PANE_RATIO)
    } else {
        0.5
    }
}

/// A terminal tab's panes. Each pane runs its own shell, keyed by
/// `(tab, pane)`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Panes {
    pub root: Node,
    /// The pane with the keyboard; it names the tab.
    pub focused: u32,
    #[serde(skip)]
    next_id: u32,
}

impl Default for Panes {
    fn default() -> Self {
        Self {
            root: Node::Pane { id: 0 },
            focused: 0,
            next_id: 1,
        }
    }
}

impl Panes {
    /// Panes laid out as saved (fresh ids; the first pane focused).
    pub fn restore(root: Node) -> Self {
        let mut next_id = 0;
        let root = root.renumbered(&mut next_id);
        Self {
            focused: root.first_pane(),
            root,
            next_id,
        }
    }

    /// Pane ids in reading order.
    pub fn ids(&self) -> Vec<u32> {
        let mut ids = Vec::new();
        self.root.panes(&mut ids);
        ids
    }

    pub fn has(&self, pane: u32) -> bool {
        self.ids().contains(&pane)
    }

    pub fn len(&self) -> usize {
        self.ids().len()
    }

    /// Split the focused pane; the new pane goes `direction` of it and
    /// takes the focus. Returns its id.
    pub fn split(&mut self, direction: Direction) -> u32 {
        let (split, pane) = (self.next_id, self.next_id + 1);
        self.next_id += 2;
        let focused = self.focused;
        let node = self
            .root
            .pane_mut(focused)
            .expect("the focused pane is in the tree");
        *node = Node::Split {
            id: split,
            direction,
            ratio: 0.5,
            first: Box::new(Node::Pane { id: focused }),
            second: Box::new(Node::Pane { id: pane }),
        };
        self.focused = pane;
        pane
    }

    /// Close a pane; its neighbor takes its room. Refused for the last one
    /// (closing that closes the tab).
    pub fn close(&mut self, pane: u32) -> bool {
        let Some(next) = self.root.remove(pane) else {
            return false;
        };
        if self.focused == pane {
            self.focused = next;
        }
        true
    }

    pub fn focus(&mut self, pane: u32) -> bool {
        let found = self.has(pane);
        if found {
            self.focused = pane;
        }
        found
    }

    /// Focus the pane `delta` places away in reading order, wrapping.
    pub fn cycle(&mut self, delta: isize) {
        let ids = self.ids();
        let at = ids.iter().position(|&id| id == self.focused).unwrap_or(0) as isize;
        self.focused = ids[(at + delta).rem_euclid(ids.len() as isize) as usize];
    }

    /// Set a split's first-child share (clamped). Returns the ratio applied.
    pub fn set_ratio(&mut self, split: u32, ratio: f64) -> Option<f64> {
        match self.root.split_mut(split)? {
            Node::Split { ratio: r, .. } => {
                *r = clamp(ratio);
                Some(*r)
            }
            Node::Pane { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_nests_and_focuses_the_new_pane() {
        let mut p = Panes::default();
        let b = p.split(Direction::Right);
        assert_eq!(p.focused, b);
        p.focus(0);
        let c = p.split(Direction::Down);
        assert_eq!(
            p.ids(),
            [0, c, b],
            "reading order: top left, bottom left, right"
        );
        assert_eq!(p.len(), 3);
        let Node::Split {
            direction, first, ..
        } = &p.root
        else {
            panic!("root is a split");
        };
        assert_eq!(*direction, Direction::Right);
        assert!(matches!(
            **first,
            Node::Split {
                direction: Direction::Down,
                ..
            }
        ));
    }

    #[test]
    fn closing_gives_the_room_and_focus_to_the_neighbor() {
        let mut p = Panes::default();
        let b = p.split(Direction::Right);
        let c = p.split(Direction::Down);
        assert_eq!(p.ids(), [0, b, c]);
        assert!(p.close(c));
        assert_eq!(p.focused, b, "the pane above takes over");
        assert_eq!(p.ids(), [0, b]);
        p.focus(b);
        assert!(p.close(0));
        assert_eq!(p.root, Node::Pane { id: b }, "the last pane fills the tab");
        assert!(!p.close(b), "the last pane can't close (its tab does)");
        assert!(!p.close(42));
    }

    #[test]
    fn closing_an_unfocused_pane_keeps_the_focus() {
        let mut p = Panes::default();
        let b = p.split(Direction::Right);
        p.split(Direction::Right);
        let focused = p.focused;
        assert!(p.close(b));
        assert_eq!(p.focused, focused);
    }

    #[test]
    fn cycling_wraps() {
        let mut p = Panes::default();
        let b = p.split(Direction::Right);
        let c = p.split(Direction::Down);
        p.cycle(1);
        assert_eq!(p.focused, 0);
        p.cycle(-1);
        assert_eq!(p.focused, c);
        p.cycle(-1);
        assert_eq!(p.focused, b);
        assert!(!p.focus(42));
        assert_eq!(p.focused, b);
    }

    #[test]
    fn ratios_are_clamped() {
        let mut p = Panes::default();
        p.split(Direction::Right);
        let Node::Split { id, .. } = p.root else {
            panic!("root is a split");
        };
        assert_eq!(p.set_ratio(id, 0.01), Some(MIN_PANE_RATIO));
        assert_eq!(p.set_ratio(id, 0.7), Some(0.7));
        assert_eq!(p.set_ratio(0, 0.5), None, "a pane isn't a split");
    }

    #[test]
    fn restoring_renumbers_and_cleans_up() {
        let saved: Node = serde_json::from_str(
            r#"{"kind":"split","id":5,"direction":"down","ratio":3,
                "first":{"kind":"pane","id":7},"second":{"kind":"pane","id":7}}"#,
        )
        .unwrap();
        let mut p = Panes::restore(saved);
        assert_eq!(p.ids(), [1, 2], "duplicate ids made unique");
        assert_eq!(p.focused, 1);
        let Node::Split { ratio, .. } = p.root else {
            panic!("root is a split");
        };
        assert_eq!(ratio, MAX_PANE_RATIO);
        assert_eq!(p.split(Direction::Right), 4, "new ids don't collide");
    }
}
