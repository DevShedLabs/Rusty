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
    /// Saved primary screen state while alternate screen is active.
    alt_cells:      Option<Vec<Cell>>,
    alt_cells_w:    usize,
    alt_cells_h:    usize,
    alt_scrollback: Option<VecDeque<Vec<Cell>>>,
    pub in_alt_screen: bool,
}

impl Grid {
    pub fn new(width: usize, height: usize) -> Self {
        let len = width * height;
        Self {
            width,
            height,
            cells:         vec![Cell::default(); len],
            scrollback:    VecDeque::new(),
            alt_cells:     None,
            alt_cells_w:   width,
            alt_cells_h:   height,
            alt_scrollback: None,
            in_alt_screen: false,
        }
    }

    /// Enter the alternate screen: save primary state, switch to a blank buffer with no scrollback.
    pub fn enter_alt_screen(&mut self) {
        if self.in_alt_screen { return; }
        self.alt_cells      = Some(self.cells.clone());
        self.alt_cells_w    = self.width;
        self.alt_cells_h    = self.height;
        self.alt_scrollback = Some(std::mem::take(&mut self.scrollback));
        self.cells.fill(Cell::default());
        self.in_alt_screen  = true;
    }

    /// Leave the alternate screen: restore primary state.
    pub fn leave_alt_screen(&mut self) {
        if !self.in_alt_screen { return; }
        if let Some(saved) = self.alt_cells.take() {
            // Refit saved primary cells to current dimensions (window may have resized).
            let ow = self.alt_cells_w;
            let oh = self.alt_cells_h;
            let nw = self.width;
            let nh = self.height;
            if ow == nw && oh == nh {
                self.cells = saved;
            } else {
                let mut new_cells = vec![Cell::default(); nw * nh];
                let cr = oh.min(nh);
                let cc = ow.min(nw);
                for r in 0..cr {
                    for c in 0..cc {
                        let src = r * ow + c;
                        if src < saved.len() {
                            new_cells[r * nw + c] = saved[src];
                        }
                    }
                }
                self.cells = new_cells;
            }
        }
        if let Some(saved) = self.alt_scrollback.take() {
            self.scrollback = saved;
        }
        self.in_alt_screen = false;
    }

    #[inline]
    fn idx(&self, col: usize, row: usize) -> usize {
        row * self.width + col
    }

    pub fn get(&self, col: usize, row: usize) -> &Cell {
        let i = self.idx(col, row);
        self.cells.get(i).unwrap_or(&Cell::BLANK)
    }

    pub fn set(&mut self, col: usize, row: usize, cell: Cell) {
        let i = self.idx(col, row);
        if i < self.cells.len() {
            self.cells[i] = cell;
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
    }

    /// Scroll up by `n` rows within [top..=bot], pushing displaced rows into scrollback only when top==0.
    pub fn scroll_up_region(&mut self, n: usize, top: usize, bot: usize) {
        let n = n.min(bot - top + 1);
        if top == 0 && bot == self.height - 1 {
            // Full-screen scroll — push into scrollback.
            for r in top..top + n {
                let start = r * self.width;
                let row: Vec<Cell> = self.cells[start..start + self.width].to_vec();
                if !self.in_alt_screen {
                    if self.scrollback.len() >= MAX_SCROLLBACK {
                        self.scrollback.pop_front();
                    }
                    self.scrollback.push_back(row);
                }
            }
        }
        // Shift rows up within the region.
        for r in top..=bot.saturating_sub(n) {
            let src = (r + n) * self.width;
            let dst = r * self.width;
            self.cells.copy_within(src..src + self.width, dst);
        }
        // Clear vacated rows at bottom of region.
        for r in (bot + 1).saturating_sub(n)..=bot {
            let start = r * self.width;
            self.cells[start..start + self.width].fill(Cell::default());
        }
    }

    /// Scroll down by `n` rows within [top..=bot] (insert blank lines at top).
    pub fn scroll_down_region(&mut self, n: usize, top: usize, bot: usize) {
        let n = n.min(bot - top + 1);
        // Shift rows down within the region.
        for r in (top..=bot.saturating_sub(n)).rev() {
            let src = r * self.width;
            let dst = (r + n) * self.width;
            self.cells.copy_within(src..src + self.width, dst);
        }
        // Clear vacated rows at top of region.
        for r in top..top + n {
            let start = r * self.width;
            self.cells[start..start + self.width].fill(Cell::default());
        }
    }

    /// Scroll up by `n` rows, pushing displaced rows into scrollback.
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_up_region(n, 0, self.height - 1);
    }

    /// Reflow content into new dimensions, preserving as much as possible.
    pub fn resize(&mut self, new_width: usize, new_height: usize) {
        let mut new_cells = vec![Cell::default(); new_width * new_height];
        let copy_rows = self.height.min(new_height);
        let copy_cols = self.width.min(new_width);
        for r in 0..copy_rows {
            for c in 0..copy_cols {
                let src = r * self.width + c;
                if src < self.cells.len() {
                    new_cells[r * new_width + c] = self.cells[src];
                }
            }
        }
        self.cells  = new_cells;
        self.width  = new_width;
        self.height = new_height;
        // leave_alt_screen handles refitting alt_cells via alt_cells_w/h.
    }

    /// Get a cell from the combined scrollback+screen view.
    /// `row` 0 = oldest scrollback line, scrollback.len() = first screen row.
    pub fn scrollback_get(&self, col: usize, row: usize) -> &Cell {
        let sb = self.scrollback.len();
        if row < sb {
            self.scrollback[row].get(col).unwrap_or(&Cell::BLANK)
        } else {
            let screen_row = row - sb;
            let idx = screen_row * self.width + col;
            if screen_row < self.height && col < self.width && idx < self.cells.len() {
                &self.cells[idx]
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
