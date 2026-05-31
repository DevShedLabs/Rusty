use crate::Cell;

pub struct Grid {
    pub width:  usize,
    pub height: usize,
    cells:      Vec<Cell>,
    dirty:      Vec<bool>,
}

impl Grid {
    pub fn new(width: usize, height: usize) -> Self {
        let len = width * height;
        Self {
            width,
            height,
            cells: vec![Cell::default(); len],
            dirty: vec![true; len],
        }
    }

    #[inline]
    fn idx(&self, col: usize, row: usize) -> usize {
        row * self.width + col
    }

    pub fn get(&self, col: usize, row: usize) -> &Cell {
        &self.cells[self.idx(col, row)]
    }

    pub fn set(&mut self, col: usize, row: usize, cell: Cell) {
        let i = self.idx(col, row);
        if self.cells[i] != cell {
            self.cells[i] = cell;
            self.dirty[i] = true;
        }
    }

    pub fn is_dirty(&self, col: usize, row: usize) -> bool {
        self.dirty[self.idx(col, row)]
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.fill(false);
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
        self.dirty.fill(true);
    }

    /// Scroll the grid up by `n` rows, clearing the vacated rows at the bottom.
    pub fn scroll_up(&mut self, n: usize) {
        let n = n.min(self.height);
        let row_bytes = self.width;
        self.cells.copy_within(n * row_bytes.., 0);
        let clear_start = (self.height - n) * row_bytes;
        self.cells[clear_start..].fill(Cell::default());
        self.dirty.fill(true);
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        *self = Self::new(width, height);
    }

    pub fn rows(&self) -> impl Iterator<Item = &[Cell]> {
        self.cells.chunks(self.width)
    }
}
