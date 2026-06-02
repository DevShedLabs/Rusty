use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Split {
    Horizontal, // panes side-by-side (vertical divider)
    Vertical,   // panes stacked (horizontal divider)
}

/// A recursive layout tree. Each leaf holds a pane ID;
/// each node describes how to split space between two subtrees.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Layout {
    Leaf { pane_id: u32 },
    Split {
        direction: Split,
        /// 0.0–1.0 fraction of space given to `first`.
        ratio:     f32,
        first:     Box<Layout>,
        second:    Box<Layout>,
    },
}

/// Pixel/cell rectangle for a pane.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x:    usize,
    pub y:    usize,
    pub w:    usize,
    pub h:    usize,
}

impl Layout {
    pub fn single(pane_id: u32) -> Self {
        Self::Leaf { pane_id }
    }

    pub fn split(direction: Split, ratio: f32, first: Layout, second: Layout) -> Self {
        Self::Split {
            direction,
            ratio,
            first:  Box::new(first),
            second: Box::new(second),
        }
    }

    /// Walk the layout tree and collect (pane_id, Rect) for every leaf,
    /// given the available area starting at (x, y) with size (w, h).
    pub fn rects(&self, x: usize, y: usize, w: usize, h: usize) -> Vec<(u32, Rect)> {
        match self {
            Layout::Leaf { pane_id } => vec![(*pane_id, Rect { x, y, w, h })],
            Layout::Split { direction, ratio, first, second } => {
                let mut out = Vec::new();
                match direction {
                    Split::Horizontal => {
                        // Side-by-side: first on the left, second on the right.
                        // Reserve 1 col for the divider.
                        let first_w = ((w as f32 * ratio).round() as usize)
                            .min(w.saturating_sub(2))
                            .max(1);
                        let second_w = w.saturating_sub(first_w + 1);
                        out.extend(first.rects(x, y, first_w, h));
                        out.extend(second.rects(x + first_w + 1, y, second_w, h));
                    }
                    Split::Vertical => {
                        // Stacked: first on top, second below.
                        // Reserve 1 row for the divider.
                        let first_h = ((h as f32 * ratio).round() as usize)
                            .min(h.saturating_sub(2))
                            .max(1);
                        let second_h = h.saturating_sub(first_h + 1);
                        out.extend(first.rects(x, y, w, first_h));
                        out.extend(second.rects(x, y + first_h + 1, w, second_h));
                    }
                }
                out
            }
        }
    }

    /// Returns the ordered list of pane IDs in the tree (depth-first, leaves).
    pub fn pane_ids(&self) -> Vec<u32> {
        match self {
            Layout::Leaf { pane_id } => vec![*pane_id],
            Layout::Split { first, second, .. } => {
                let mut ids = first.pane_ids();
                ids.extend(second.pane_ids());
                ids
            }
        }
    }

    /// Replace a Leaf with the given pane_id with a Split node containing
    /// the original leaf and a new leaf for new_pane_id.
    pub fn insert_split(&mut self, target: u32, direction: Split, new_pane_id: u32) -> bool {
        match self {
            Layout::Leaf { pane_id } if *pane_id == target => {
                let original = Layout::Leaf { pane_id: *pane_id };
                *self = Layout::Split {
                    direction,
                    ratio: 0.5,
                    first:  Box::new(original),
                    second: Box::new(Layout::Leaf { pane_id: new_pane_id }),
                };
                true
            }
            Layout::Leaf { .. } => false,
            Layout::Split { first, second, .. } => {
                first.insert_split(target, direction, new_pane_id)
                    || second.insert_split(target, direction, new_pane_id)
            }
        }
    }

    /// Remove the leaf with pane_id. Returns Some(sibling) if the parent
    /// node should be replaced by the sibling, None if no change needed.
    pub fn remove_pane(&mut self, target: u32) -> RemoveResult {
        match self {
            Layout::Leaf { pane_id } => {
                if *pane_id == target { RemoveResult::RemoveMe } else { RemoveResult::NotFound }
            }
            Layout::Split { first, second, .. } => {
                match first.remove_pane(target) {
                    RemoveResult::RemoveMe   => RemoveResult::ReplaceWith(*second.clone()),
                    RemoveResult::ReplaceWith(n) => { *first = Box::new(n); RemoveResult::Done }
                    RemoveResult::Done       => RemoveResult::Done,
                    RemoveResult::NotFound   => {
                        match second.remove_pane(target) {
                            RemoveResult::RemoveMe      => RemoveResult::ReplaceWith(*first.clone()),
                            RemoveResult::ReplaceWith(n) => { *second = Box::new(n); RemoveResult::Done }
                            r => r,
                        }
                    }
                }
            }
        }
    }

    /// Adjust the ratio of the split containing `target` by `delta` (−1.0..1.0).
    pub fn adjust_ratio(&mut self, target: u32, delta: f32) -> bool {
        match self {
            Layout::Leaf { .. } => false,
            Layout::Split { first, second, ratio, .. } => {
                let ids_first  = first.pane_ids();
                let ids_second = second.pane_ids();
                if ids_first.contains(&target) || ids_second.contains(&target) {
                    *ratio = (*ratio + delta).clamp(0.1, 0.9);
                    true
                } else {
                    first.adjust_ratio(target, delta) || second.adjust_ratio(target, delta)
                }
            }
        }
    }
}

pub enum RemoveResult {
    NotFound,
    RemoveMe,
    ReplaceWith(Layout),
    Done,
}
