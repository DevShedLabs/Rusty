use crate::Tab;
use serde::Serialize;

pub struct Session {
    pub id:         u32,
    pub tabs:       Vec<Tab>,
    pub active_tab: usize,
    cols:           usize,
    rows:           usize,
    next_pane_id:   u32,
}

impl Session {
    pub fn new(id: u32, cols: usize, rows: usize) -> Self {
        let tab = Tab::with_pane_id(0, 0, cols, rows);
        Self { id, tabs: vec![tab], active_tab: 0, cols, rows, next_pane_id: 1 }
    }

    /// Allocate the next globally-unique pane ID.
    pub fn alloc_pane_id(&mut self) -> u32 {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        id
    }

    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active_tab]
    }

    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active_tab]
    }

    /// Open a new tab. Returns its index.
    pub fn new_tab(&mut self) -> usize {
        let tab_id   = self.tabs.len() as u32;
        let pane_id  = self.alloc_pane_id();
        self.tabs.push(Tab::with_pane_id(tab_id, pane_id, self.cols, self.rows));
        self.active_tab = self.tabs.len() - 1;
        self.active_tab
    }

    /// Close the active tab. Returns true if the session is now empty.
    pub fn close_active_tab(&mut self) -> bool {
        if self.tabs.len() == 1 { return true; }
        self.tabs.remove(self.active_tab);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        false
    }

    pub fn next_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
        }
    }

    pub fn prev_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
        }
    }

    pub fn cols(&self) -> usize { self.cols }
    pub fn rows(&self) -> usize { self.rows }

    /// Resize all panes in all tabs.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        for tab in &mut self.tabs {
            tab.resize(cols, rows);
        }
    }

    /// Serialize session layout to JSON for restore.
    pub fn save_layout(&self) -> anyhow::Result<String> {
        #[derive(Serialize)]
        struct Snapshot<'a> {
            id:         u32,
            active_tab: usize,
            tabs:       Vec<TabSnap<'a>>,
        }
        #[derive(Serialize)]
        struct TabSnap<'a> {
            id:     u32,
            title:  &'a str,
            layout: &'a crate::layout::Layout,
        }
        let snap = Snapshot {
            id:         self.id,
            active_tab: self.active_tab,
            tabs:       self.tabs.iter().map(|t| TabSnap {
                id:     t.id,
                title:  &t.title,
                layout: &t.layout,
            }).collect(),
        };
        Ok(serde_json::to_string(&snap)?)
    }
}
