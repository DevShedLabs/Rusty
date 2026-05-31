use rusty_core::{Attrs, Cell, Color, Grid};
use rusty_core::cursor::Cursor;
use rusty_core::parser::{Action, Parser};

pub struct Pane {
    pub id:          u32,
    pub grid:        Grid,
    pub cursor:      Cursor,
    /// How many scrollback rows are hidden above the viewport (0 = live view).
    pub scroll_off:  usize,
    parser:          Parser,
    pen_fg:          Color,
    pen_bg:          Color,
    pen_attrs:       Attrs,
}

impl Pane {
    pub fn new(id: u32, cols: usize, rows: usize) -> Self {
        Self {
            id,
            grid:       Grid::new(cols, rows),
            cursor:     Cursor { col: 0, row: 0, visible: true },
            scroll_off: 0,
            parser:     Parser::new(),
            pen_fg:     Color::Default,
            pen_bg:     Color::Default,
            pen_attrs:  Attrs::empty(),
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.grid.resize(cols, rows);
        self.cursor.col  = self.cursor.col.min(cols.saturating_sub(1));
        self.cursor.row  = self.cursor.row.min(rows.saturating_sub(1));
        // Any input resets to live view.
        self.scroll_off  = 0;
    }

    pub fn scroll_up_view(&mut self, lines: usize) {
        let max = self.grid.scrollback.len();
        self.scroll_off = (self.scroll_off + lines).min(max);
    }

    pub fn scroll_down_view(&mut self, lines: usize) {
        self.scroll_off = self.scroll_off.saturating_sub(lines);
    }

    pub fn process(&mut self, bytes: &[u8]) {
        for action in self.parser.advance(bytes) {
            self.apply(action);
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Print(ch) => self.print(ch),

            Action::Execute(byte) => match byte {
                0x08 => self.cursor_left(1),            // BS
                0x09 => self.tab(),                     // HT
                0x0a | 0x0b | 0x0c => self.linefeed(),  // LF/VT/FF
                0x0d => self.cursor.col = 0,             // CR
                _ => {}
            },

            Action::CsiDispatch { params, action, .. } => {
                self.csi(&params, action);
            }

            _ => {}
        }
    }

    // ── printing ────────────────────────────────────────────────────────────

    fn print(&mut self, ch: char) {
        if self.cursor.col >= self.grid.width {
            self.cursor.col = 0;
            self.linefeed();
        }
        let cell = Cell { ch, fg: self.pen_fg, bg: self.pen_bg, attrs: self.pen_attrs };
        self.grid.set(self.cursor.col, self.cursor.row, cell);
        self.cursor.col += 1;
    }

    // ── cursor movement ──────────────────────────────────────────────────────

    fn cursor_up(&mut self, n: usize) {
        self.cursor.row = self.cursor.row.saturating_sub(n);
    }
    fn cursor_down(&mut self, n: usize) {
        self.cursor.row = (self.cursor.row + n).min(self.grid.height - 1);
    }
    fn cursor_left(&mut self, n: usize) {
        self.cursor.col = self.cursor.col.saturating_sub(n);
    }
    fn cursor_right(&mut self, n: usize) {
        self.cursor.col = (self.cursor.col + n).min(self.grid.width - 1);
    }

    fn linefeed(&mut self) {
        if self.cursor.row + 1 >= self.grid.height {
            self.scroll_up(1);
        } else {
            self.cursor.row += 1;
        }
    }

    fn tab(&mut self) {
        let next = ((self.cursor.col / 8) + 1) * 8;
        self.cursor.col = next.min(self.grid.width - 1);
    }

    fn scroll_up(&mut self, n: usize) {
        self.grid.scroll_up(n);
    }

    // ── erase ────────────────────────────────────────────────────────────────

    fn erase_in_display(&mut self, mode: i64) {
        let (col, row) = (self.cursor.col, self.cursor.row);
        let (w, h) = (self.grid.width, self.grid.height);
        match mode {
            0 => {
                // cursor to end
                for c in col..w { self.grid.set(c, row, Cell::default()); }
                for r in (row + 1)..h {
                    for c in 0..w { self.grid.set(c, r, Cell::default()); }
                }
            }
            1 => {
                // start to cursor
                for r in 0..row {
                    for c in 0..w { self.grid.set(c, r, Cell::default()); }
                }
                for c in 0..=col { self.grid.set(c, row, Cell::default()); }
            }
            2 | 3 => self.grid.clear(),
            _ => {}
        }
    }

    fn erase_in_line(&mut self, mode: i64) {
        let (col, row) = (self.cursor.col, self.cursor.row);
        let w = self.grid.width;
        match mode {
            0 => for c in col..w  { self.grid.set(c, row, Cell::default()); },
            1 => for c in 0..=col { self.grid.set(c, row, Cell::default()); },
            2 => for c in 0..w   { self.grid.set(c, row, Cell::default()); },
            _ => {}
        }
    }

    // ── CSI dispatch ─────────────────────────────────────────────────────────

    fn csi(&mut self, params: &[i64], action: char) {
        let p = |i: usize, default: i64| -> i64 {
            params.get(i).copied().filter(|&v| v != 0).unwrap_or(default)
        };

        match action {
            'A' => self.cursor_up(p(0, 1) as usize),
            'B' => self.cursor_down(p(0, 1) as usize),
            'C' => self.cursor_right(p(0, 1) as usize),
            'D' => self.cursor_left(p(0, 1) as usize),
            'E' => { self.cursor_down(p(0, 1) as usize); self.cursor.col = 0; }
            'F' => { self.cursor_up(p(0, 1) as usize);   self.cursor.col = 0; }
            'G' => self.cursor.col = (p(0, 1) as usize).saturating_sub(1).min(self.grid.width - 1),
            'H' | 'f' => {
                self.cursor.row = (p(0, 1) as usize).saturating_sub(1).min(self.grid.height - 1);
                self.cursor.col = (p(1, 1) as usize).saturating_sub(1).min(self.grid.width - 1);
            }
            'J' => self.erase_in_display(p(0, 0)),
            'K' => self.erase_in_line(p(0, 0)),
            'S' => self.scroll_up(p(0, 1) as usize),
            'm' => self.sgr(params),
            'l' | 'h' => {} // mode set/reset — ignore for now
            _ => {}
        }
    }

    // ── SGR (Select Graphic Rendition) ───────────────────────────────────────

    fn sgr(&mut self, params: &[i64]) {
        let params = if params.is_empty() { &[0i64][..] } else { params };
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0  => { self.pen_fg = Color::Default; self.pen_bg = Color::Default; self.pen_attrs = Attrs::empty(); }
                1  => self.pen_attrs |= Attrs::BOLD,
                3  => self.pen_attrs |= Attrs::ITALIC,
                4  => self.pen_attrs |= Attrs::UNDERLINE,
                5  => self.pen_attrs |= Attrs::BLINK,
                7  => self.pen_attrs |= Attrs::REVERSE,
                9  => self.pen_attrs |= Attrs::STRIKETHROUGH,
                22 => self.pen_attrs.remove(Attrs::BOLD),
                23 => self.pen_attrs.remove(Attrs::ITALIC),
                24 => self.pen_attrs.remove(Attrs::UNDERLINE),
                27 => self.pen_attrs.remove(Attrs::REVERSE),
                // Standard fg (30-37) and bright fg (90-97)
                30..=37 => self.pen_fg = Color::Indexed(params[i] as u8 - 30),
                38 => {
                    if let Some(c) = self.parse_extended_color(params, &mut i) {
                        self.pen_fg = c;
                    }
                }
                39 => self.pen_fg = Color::Default,
                40..=47 => self.pen_bg = Color::Indexed(params[i] as u8 - 40),
                48 => {
                    if let Some(c) = self.parse_extended_color(params, &mut i) {
                        self.pen_bg = c;
                    }
                }
                49 => self.pen_bg = Color::Default,
                90..=97  => self.pen_fg = Color::Indexed(params[i] as u8 - 90 + 8),
                100..=107 => self.pen_bg = Color::Indexed(params[i] as u8 - 100 + 8),
                _ => {}
            }
            i += 1;
        }
    }

    fn parse_extended_color(&self, params: &[i64], i: &mut usize) -> Option<Color> {
        match params.get(*i + 1).copied() {
            Some(2) => {
                let r = *params.get(*i + 2)? as u8;
                let g = *params.get(*i + 3)? as u8;
                let b = *params.get(*i + 4)? as u8;
                *i += 4;
                Some(Color::Rgb(r, g, b))
            }
            Some(5) => {
                let idx = *params.get(*i + 2)? as u8;
                *i += 2;
                Some(Color::Indexed(idx))
            }
            _ => None,
        }
    }
}
