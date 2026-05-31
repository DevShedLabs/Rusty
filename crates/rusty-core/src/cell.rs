use crate::{Color, Attrs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch:    char,
    pub fg:    Color,
    pub bg:    Color,
    pub attrs: Attrs,
    /// Phantom hint text rendered after this cell (type-ahead suggestion).
    /// Empty string means no hint.
    pub hint:  bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch:    ' ',
            fg:    Color::DEFAULT,
            bg:    Color::DEFAULT,
            attrs: Attrs::empty(),
            hint:  false,
        }
    }
}
