#[derive(Debug, Default, Clone, Copy)]
pub struct Cursor {
    pub col:     usize,
    pub row:     usize,
    pub visible: bool,
}
