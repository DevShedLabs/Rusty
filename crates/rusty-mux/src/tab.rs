use crate::{Layout, Pane};
use std::collections::HashMap;

pub struct Tab {
    pub id:           u32,
    pub title:        String,
    pub layout:       Layout,
    pub panes:        HashMap<u32, Pane>,
    pub active_pane:  u32,
    #[allow(dead_code)]
    next_pane_id: u32,
}

impl Tab {
    pub fn new(id: u32, cols: usize, rows: usize) -> Self {
        let pane = Pane::new(0, cols, rows);
        let layout = Layout::single(0);
        let mut panes = HashMap::new();
        panes.insert(0, pane);
        Self {
            id,
            title: String::new(),
            layout,
            panes,
            active_pane:  0,
            next_pane_id: 1,
        }
    }

    pub fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(&self.active_pane)
    }
}
