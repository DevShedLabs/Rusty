use crate::{Color, Attrs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch:    char,
    pub fg:    Color,
    pub bg:    Color,
    pub attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch:    ' ',
            fg:    Color::Default,
            bg:    Color::Default,
            attrs: Attrs::empty(),
        }
    }
}
