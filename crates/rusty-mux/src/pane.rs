use rusty_core::{Attrs, Cell, Color, Grid};
use rusty_core::cursor::Cursor;
use rusty_core::parser::{Action, Parser};

pub enum PaneEvent {
    Cwd(String),
}

pub struct Pane {
    pub id:           u32,
    pub grid:         Grid,
    pub cursor:       Cursor,
    pub scroll_off:   usize,
    parser:           Parser,
    pen_fg:           Color,
    pen_bg:           Color,
    pen_attrs:        Attrs,
    /// DECSTBM scroll region (inclusive row indices).
    scroll_top:       usize,
    scroll_bot:       usize,
    /// DECAWM — autowrap mode. When false, characters past the right margin are clipped.
    autowrap:         bool,
    /// Last printed character, for REP (CSI Pb b).
    last_char:        char,
    /// Application cursor keys mode (CSI ?1h): arrows send SS3 sequences.
    pub app_cursor:   bool,
}

impl Pane {
    pub fn new(id: u32, cols: usize, rows: usize) -> Self {
        Self {
            id,
            grid:        Grid::new(cols, rows),
            cursor:      Cursor { col: 0, row: 0, visible: true },
            scroll_off:  0,
            parser:      Parser::new(),
            pen_fg:      Color::Default,
            pen_bg:      Color::Default,
            pen_attrs:   Attrs::empty(),
            scroll_top:  0,
            scroll_bot:  rows.saturating_sub(1),
            autowrap:    true,
            last_char:   ' ',
            app_cursor:  false,
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.grid.resize(cols, rows);
        self.cursor.col = self.cursor.col.min(cols.saturating_sub(1));
        self.cursor.row = self.cursor.row.min(rows.saturating_sub(1));
        self.scroll_off  = 0;
        self.scroll_top  = 0;
        self.scroll_bot  = rows.saturating_sub(1);
    }

    pub fn scroll_up_view(&mut self, lines: usize) {
        if self.grid.in_alt_screen { return; }
        let max = self.grid.scrollback.len();
        self.scroll_off = (self.scroll_off + lines).min(max);
    }

    pub fn scroll_down_view(&mut self, lines: usize) {
        if self.grid.in_alt_screen { return; }
        self.scroll_off = self.scroll_off.saturating_sub(lines);
    }

    pub fn process(&mut self, bytes: &[u8]) {
        for action in self.parser.advance(bytes) {
            self.apply(action);
        }
    }

    /// Process bytes and return side-channel events (e.g. OSC 7 CWD changes).
    pub fn process_with_events(&mut self, bytes: &[u8]) -> Vec<PaneEvent> {
        let mut events = Vec::new();
        for action in self.parser.advance(bytes) {
            if let Action::OscDispatch { ref params, .. } = action {
                // OSC 7: notify CWD — params[0] = "7", params[1] = "file://host/path"
                if params.first().map(|p| p.as_slice() == b"7").unwrap_or(false) {
                    if let Some(payload) = params.get(1) {
                        if let Ok(s) = std::str::from_utf8(payload) {
                            events.push(PaneEvent::Cwd(s.to_owned()));
                        }
                    }
                }
            }
            self.apply(action);
        }
        events
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Print(ch) => self.print(ch),
            Action::Execute(byte) => match byte {
                0x08 => self.cursor_left(1),
                0x09 => self.tab(),
                0x0a | 0x0b | 0x0c => self.linefeed(),
                0x0d => self.cursor.col = 0,
                _ => {}
            },
            Action::CsiDispatch { params, intermediates, action, .. } => self.csi(&params, &intermediates, action),
            _ => {}
        }
    }

    // ── printing ──────────────────────────────────────────────────────────────

    fn print(&mut self, ch: char) {
        if self.cursor.col >= self.grid.width {
            if !self.autowrap { return; }
            self.cursor.col = 0;
            self.linefeed();
        }
        let cell = Cell { ch, fg: self.pen_fg, bg: self.pen_bg, attrs: self.pen_attrs };
        self.grid.set(self.cursor.col, self.cursor.row, cell);
        self.cursor.col += 1;
        self.last_char = ch;
    }

    // ── cursor movement ───────────────────────────────────────────────────────

    fn cursor_up(&mut self, n: usize)   { self.cursor.row = self.cursor.row.saturating_sub(n); }
    fn cursor_down(&mut self, n: usize) { self.cursor.row = (self.cursor.row + n).min(self.grid.height - 1); }
    fn cursor_left(&mut self, n: usize)  { self.cursor.col = self.cursor.col.saturating_sub(n); }
    fn cursor_right(&mut self, n: usize) { self.cursor.col = (self.cursor.col + n).min(self.grid.width - 1); }

    fn linefeed(&mut self) {
        if self.cursor.row == self.scroll_bot {
            self.grid.scroll_up_region(1, self.scroll_top, self.scroll_bot);
        } else if self.cursor.row + 1 < self.grid.height {
            self.cursor.row += 1;
        }
    }

    fn tab(&mut self) {
        self.cursor.col = (((self.cursor.col / 8) + 1) * 8).min(self.grid.width - 1);
    }

    // ── erase ─────────────────────────────────────────────────────────────────

    fn erase_in_display(&mut self, mode: i64) {
        let (col, row) = (self.cursor.col, self.cursor.row);
        let (w, h) = (self.grid.width, self.grid.height);
        match mode {
            0 => {
                for c in col..w { self.grid.set(c, row, Cell::default()); }
                for r in (row + 1)..h { for c in 0..w { self.grid.set(c, r, Cell::default()); } }
            }
            1 => {
                for r in 0..row { for c in 0..w { self.grid.set(c, r, Cell::default()); } }
                for c in 0..=col { self.grid.set(c, row, Cell::default()); }
            }
            2 | 3 => {
                self.grid.clear();
                self.grid.scrollback.clear();
                self.scroll_off = 0;
            }
            _ => {}
        }
    }

    fn erase_in_line(&mut self, mode: i64) {
        let (col, row, w) = (self.cursor.col, self.cursor.row, self.grid.width);
        match mode {
            0 => for c in col..w  { self.grid.set(c, row, Cell::default()); },
            1 => for c in 0..=col { self.grid.set(c, row, Cell::default()); },
            2 => for c in 0..w   { self.grid.set(c, row, Cell::default()); },
            _ => {}
        }
    }

    // ── CSI ───────────────────────────────────────────────────────────────────

    fn csi(&mut self, params: &[i64], intermediates: &[u8], action: char) {
        let p = |i: usize, default: i64| params.get(i).copied().filter(|&v| v != 0).unwrap_or(default);
        // Private mode sequences: CSI ? Pn h/l
        if intermediates == b"?" {
            match (params.first().copied().unwrap_or(0), action) {
                (1049, 'h') => {
                    self.grid.enter_alt_screen();
                    self.cursor     = rusty_core::cursor::Cursor { col: 0, row: 0, visible: true };
                    self.scroll_off = 0;
                    self.scroll_top = 0;
                    self.scroll_bot = self.grid.height.saturating_sub(1);
                    self.autowrap   = true;
                }
                (1049, 'l') => {
                    self.grid.leave_alt_screen();
                    self.scroll_off = 0;
                    self.scroll_top = 0;
                    self.scroll_bot = self.grid.height.saturating_sub(1);
                    self.autowrap   = true;
                }
                (25,   'l') => self.cursor.visible = false,
                (25,   'h') => self.cursor.visible = true,
                (7,    'l') => self.autowrap = false,
                (7,    'h') => self.autowrap = true,
                (1,    'h') => self.app_cursor = true,
                (1,    'l') => self.app_cursor = false,
                // Mouse reporting — acknowledge but don't need to act on it.
                (1000, 'h') | (1000, 'l') => {}
                (1006, 'h') | (1006, 'l') => {}
                _ => {}
            }
            return;
        }
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
            // REP — repeat last printed character N times.
            'b' => { let n = p(0, 1) as usize; let ch = self.last_char; for _ in 0..n { self.print(ch); } }
            // VPA — move cursor to absolute row (1-based), keep column.
            'd' => self.cursor.row = (p(0, 1) as usize).saturating_sub(1).min(self.grid.height - 1),
            'J' => self.erase_in_display(p(0, 0)),
            'K' => self.erase_in_line(p(0, 0)),
            // Erase N characters from cursor position (fill with blanks, don't move cursor).
            'X' => {
                let n = p(0, 1) as usize;
                let (col, row, w) = (self.cursor.col, self.cursor.row, self.grid.width);
                for c in col..(col + n).min(w) { self.grid.set(c, row, Cell::default()); }
            }
            // Delete N characters (shift left, fill right with blanks).
            'P' => {
                let n = p(0, 1) as usize;
                let (col, row, w) = (self.cursor.col, self.cursor.row, self.grid.width);
                for c in col..w {
                    let src_c = c + n;
                    let cell = if src_c < w { *self.grid.get(src_c, row) } else { Cell::default() };
                    self.grid.set(c, row, cell);
                }
            }
            // Insert N blank characters (shift right, drop chars past end).
            '@' => {
                let n = p(0, 1) as usize;
                let (col, row, w) = (self.cursor.col, self.cursor.row, self.grid.width);
                for c in (col..w).rev() {
                    let cell = if c >= col + n { *self.grid.get(c - n, row) } else { Cell::default() };
                    self.grid.set(c, row, cell);
                }
            }
            'S' => self.grid.scroll_up_region(p(0, 1) as usize, self.scroll_top, self.scroll_bot),
            'T' => self.grid.scroll_down_region(p(0, 1) as usize, self.scroll_top, self.scroll_bot),
            'L' => self.grid.scroll_down_region(p(0, 1) as usize, self.cursor.row, self.scroll_bot),
            'M' => self.grid.scroll_up_region(p(0, 1) as usize, self.cursor.row, self.scroll_bot),
            'r' => {
                // DECSTBM — set scroll region (1-based, default = full screen).
                let top = (p(0, 1) as usize).saturating_sub(1).min(self.grid.height - 1);
                let bot = (p(1, self.grid.height as i64) as usize).saturating_sub(1).min(self.grid.height - 1);
                if top < bot {
                    self.scroll_top = top;
                    self.scroll_bot = bot;
                    self.cursor = rusty_core::cursor::Cursor { col: 0, row: 0, visible: self.cursor.visible };
                }
            }
            'm' => self.sgr(params),
            'l' | 'h' => {}
            _ => {}
        }
    }

    // ── SGR ───────────────────────────────────────────────────────────────────

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
                30..=37  => self.pen_fg = Color::Indexed(params[i] as u8 - 30),
                38 => { if let Some(c) = self.parse_extended_color(params, &mut i) { self.pen_fg = c; } }
                39 => self.pen_fg = Color::Default,
                40..=47  => self.pen_bg = Color::Indexed(params[i] as u8 - 40),
                48 => { if let Some(c) = self.parse_extended_color(params, &mut i) { self.pen_bg = c; } }
                49 => self.pen_bg = Color::Default,
                90..=97   => self.pen_fg = Color::Indexed(params[i] as u8 - 90 + 8),
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
