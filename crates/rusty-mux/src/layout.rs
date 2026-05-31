use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Split {
    Horizontal,
    Vertical,
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
}
