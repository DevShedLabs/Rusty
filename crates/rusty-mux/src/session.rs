use crate::Tab;
use serde::Serialize;

pub struct Session {
    pub id:          u32,
    pub tabs:        Vec<Tab>,
    pub active_tab:  usize,
    cols:            usize,
    rows:            usize,
}

impl Session {
    pub fn new(id: u32, cols: usize, rows: usize) -> Self {
        let tab = Tab::new(0, cols, rows);
        Self { id, tabs: vec![tab], active_tab: 0, cols, rows }
    }

    pub fn new_tab(&mut self) {
        let id = self.tabs.len() as u32;
        self.tabs.push(Tab::new(id, self.cols, self.rows));
        self.active_tab = self.tabs.len() - 1;
    }

    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active_tab]
    }

    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active_tab]
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
