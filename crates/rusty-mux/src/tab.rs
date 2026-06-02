use crate::{Layout, Pane};
use crate::layout::{RemoveResult, Split};
use std::collections::HashMap;

pub struct Tab {
    pub id:          u32,
    pub title:       String,
    pub layout:      Layout,
    pub panes:       HashMap<u32, Pane>,
    pub active_pane: u32,
}

impl Tab {
    pub fn new(id: u32, cols: usize, rows: usize) -> Self {
        Self::with_pane_id(id, 0, cols, rows)
    }

    /// Create a new tab whose initial pane uses a caller-supplied globally-unique pane_id.
    pub fn with_pane_id(id: u32, pane_id: u32, cols: usize, rows: usize) -> Self {
        let pane   = Pane::new(pane_id, cols, rows);
        let layout = Layout::single(pane_id);
        let mut panes = HashMap::new();
        panes.insert(pane_id, pane);
        Self {
            id,
            title:        String::new(),
            layout,
            panes,
            active_pane:  pane_id,
        }
    }

    /// Update the tab title from an OSC 7 "file://host/path" payload,
    /// using just the last path component as the display name.
    pub fn update_title_from_cwd(&mut self, osc7: &str) {
        // Strip "file://hostname" prefix to get the path.
        let path = if let Some(rest) = osc7.strip_prefix("file://") {
            // rest = "hostname/path/to/dir" — skip the hostname component.
            rest.find('/').map(|i| &rest[i..]).unwrap_or(rest)
        } else {
            osc7
        };
        // Use the last non-empty path component.
        let name = path.trim_end_matches('/').rsplit('/').find(|s| !s.is_empty()).unwrap_or(path);
        self.title = name.to_owned();
    }

    pub fn active_pane(&self) -> Option<&Pane> {
        self.panes.get(&self.active_pane)
    }

    pub fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(&self.active_pane)
    }

    /// Split the active pane with a caller-supplied globally-unique new_id.
    /// Returns the new pane's ID so the caller can spawn a PTY.
    pub fn split(&mut self, direction: Split, new_id: u32, cols: usize, rows: usize) -> u32 {
        // Each half gets roughly half the space; the renderer will recompute sizes
        // from layout_rects, but the pane needs *some* valid initial size.
        let (half_cols, half_rows) = match direction {
            Split::Horizontal => (cols / 2, rows),
            Split::Vertical   => (cols, rows / 2),
        };
        self.panes.insert(new_id, Pane::new(new_id, half_cols.max(1), half_rows.max(1)));
        self.layout.insert_split(self.active_pane, direction, new_id);
        self.active_pane = new_id;
        new_id
    }

    /// Close the active pane. Returns true if the tab is now empty (caller should close the tab).
    pub fn close_active_pane(&mut self) -> bool {
        if self.panes.len() == 1 {
            return true; // last pane — caller closes the whole tab
        }
        let removed = self.active_pane;
        let ids_before = self.layout.pane_ids();

        match self.layout.remove_pane(removed) {
            RemoveResult::ReplaceWith(new_layout) => self.layout = new_layout,
            RemoveResult::Done => {}
            _ => return false, // shouldn't happen with len>1
        }
        self.panes.remove(&removed);

        // Focus the pane that was before the removed one in document order.
        let remaining = self.layout.pane_ids();
        let focus = ids_before
            .iter()
            .rev()
            .find(|id| remaining.contains(id))
            .copied()
            .unwrap_or(remaining[0]);
        self.active_pane = focus;
        false
    }

    /// Cycle focus to the next pane in document order.
    pub fn focus_next_pane(&mut self) {
        let ids = self.layout.pane_ids();
        if ids.len() < 2 { return; }
        let pos = ids.iter().position(|&id| id == self.active_pane).unwrap_or(0);
        self.active_pane = ids[(pos + 1) % ids.len()];
    }

    /// Cycle focus to the previous pane in document order.
    pub fn focus_prev_pane(&mut self) {
        let ids = self.layout.pane_ids();
        if ids.len() < 2 { return; }
        let pos = ids.iter().position(|&id| id == self.active_pane).unwrap_or(0);
        self.active_pane = ids[(pos + ids.len() - 1) % ids.len()];
    }

    /// Move focus by directional arrow within the layout geometry.
    /// `cols`/`rows` are the total available cell dimensions for the tab.
    pub fn focus_direction(&mut self, dir: FocusDir, cols: usize, rows: usize) {
        let rects = self.layout.rects(0, 0, cols, rows);
        let cur = rects.iter().find(|(id, _)| *id == self.active_pane);
        let (_, cur_rect) = match cur { Some(r) => r, None => return };
        let cx = cur_rect.x + cur_rect.w / 2;
        let cy = cur_rect.y + cur_rect.h / 2;

        let best = rects.iter()
            .filter(|(id, _)| *id != self.active_pane)
            .filter_map(|(id, r)| {
                let rx = r.x + r.w / 2;
                let ry = r.y + r.h / 2;
                let (dx, dy) = (rx as i64 - cx as i64, ry as i64 - cy as i64);
                let is_dir = match dir {
                    FocusDir::Right => dx > 0 && dx.abs() >= dy.abs(),
                    FocusDir::Left  => dx < 0 && dx.abs() >= dy.abs(),
                    FocusDir::Down  => dy > 0 && dy.abs() >= dx.abs(),
                    FocusDir::Up    => dy < 0 && dy.abs() >= dx.abs(),
                };
                if is_dir { Some((id, dx * dx + dy * dy)) } else { None }
            })
            .min_by_key(|(_, dist)| *dist)
            .map(|(id, _)| *id);

        if let Some(id) = best {
            self.active_pane = id;
        }
    }

    /// Resize all panes to fit `cols`×`rows`, recomputing from layout_rects.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        for (id, rect) in self.layout.rects(0, 0, cols, rows) {
            if let Some(pane) = self.panes.get_mut(&id) {
                pane.resize(rect.w.max(1), rect.h.max(1));
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FocusDir {
    Up,
    Down,
    Left,
    Right,
}
