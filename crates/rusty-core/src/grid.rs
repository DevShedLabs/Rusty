use crate::Cell;
use std::collections::VecDeque;

const MAX_SCROLLBACK: usize = 10_000;

pub struct Grid {
    pub width:    usize,
    pub height:   usize,
    /// Active screen cells, row-major.
    cells:        Vec<Cell>,
    /// Lines that have scrolled off the top (oldest first).
    pub scrollback: VecDeque<Vec<Cell>>,
}

impl Grid {
    pub fn new(width: usize, height: usize) -> Self {
        let len = width * height;
        Self {
            width,
            height,
            cells:      vec![Cell::default(); len],
            scrollback: VecDeque::new(),
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
        self.cells[i] = cell;
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
    }

    /// Scroll up by `n` rows, pushing displaced rows into scrollback.
    pub fn scroll_up(&mut self, n: usize) {
        let n = n.min(self.height);
        for r in 0..n {
            let start = r * self.width;
            let row: Vec<Cell> = self.cells[start..start + self.width].to_vec();
            if self.scrollback.len() >= MAX_SCROLLBACK {
                self.scrollback.pop_front();
            }
            self.scrollback.push_back(row);
        }
        self.cells.copy_within(n * self.width.., 0);
        let clear_start = (self.height - n) * self.width;
        self.cells[clear_start..].fill(Cell::default());
    }

    /// Reflow content into new dimensions, preserving as much as possible.
    pub fn resize(&mut self, new_width: usize, new_height: usize) {
        let mut new_cells = vec![Cell::default(); new_width * new_height];
        let copy_rows = self.height.min(new_height);
        let copy_cols = self.width.min(new_width);
        for r in 0..copy_rows {
            for c in 0..copy_cols {
                new_cells[r * new_width + c] = self.cells[r * self.width + c];
            }
        }
        self.cells  = new_cells;
        self.width  = new_width;
        self.height = new_height;
    }

    /// Get a cell from the combined scrollback+screen view.
    /// `row` 0 = oldest scrollback line, scrollback.len() = first screen row.
    pub fn scrollback_get(&self, col: usize, row: usize) -> &Cell {
        let sb = self.scrollback.len();
        if row < sb {
            self.scrollback[row].get(col).unwrap_or(&Cell::BLANK)
        } else {
            let screen_row = row - sb;
            if screen_row < self.height && col < self.width {
                &self.cells[screen_row * self.width + col]
            } else {
                &Cell::BLANK
            }
        }
    }

    /// Total rows in scrollback + screen.
    pub fn total_rows(&self) -> usize {
        self.scrollback.len() + self.height
    }

    pub fn rows(&self) -> impl Iterator<Item = &[Cell]> {
        self.cells.chunks(self.width)
    }
}
