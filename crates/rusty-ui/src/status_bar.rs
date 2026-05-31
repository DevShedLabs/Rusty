use rusty_git::GitInfo;

/// Data model for the status bar rendered below/above the grid.
/// Populated from Git info + active pane state each frame.
#[derive(Default)]
pub struct StatusBar {
    pub git:   Option<GitInfo>,
    pub title: String,
    pub cols:  u16,
    pub rows:  u16,
}

impl StatusBar {
    pub fn update_git(&mut self, info: Option<GitInfo>) {
        self.git = info;
    }
}
