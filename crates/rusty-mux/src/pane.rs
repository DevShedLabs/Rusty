use rusty_core::{Attrs, Cell, Color, Grid};
use rusty_core::cursor::Cursor;
use rusty_core::parser::{Action, Parser};

pub enum PaneEvent {
    Cwd(String),
    /// Response bytes to write back to the PTY (e.g. cursor position report).
    PtyWrite(Vec<u8>),
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
    /// Mouse click reporting enabled (CSI ?1000h).
    pub mouse_report: bool,
    /// SGR extended mouse encoding (CSI ?1006h).
    pub mouse_sgr:    bool,
    /// Cursor/pen state saved on 1049h, restored on 1049l.
    saved_cursor:     Option<Cursor>,
    saved_pen_fg:     Option<Color>,
    saved_pen_bg:     Option<Color>,
    saved_pen_attrs:  Option<Attrs>,
    /// Cursor/pen state saved on ESC 7 (DECSC), restored on ESC 8 (DECRC).
    decsc_cursor:     Option<Cursor>,
    decsc_pen_fg:     Color,
    decsc_pen_bg:     Color,
    decsc_pen_attrs:  Attrs,
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
            scroll_top:   0,
            scroll_bot:   rows.saturating_sub(1),
            autowrap:     true,
            last_char:    ' ',
            app_cursor:   false,
            mouse_report: false,
            mouse_sgr:    false,
            saved_cursor:    None,
            saved_pen_fg:    None,
            saved_pen_bg:    None,
            saved_pen_attrs: None,
            decsc_cursor:    None,
            decsc_pen_fg:    Color::Default,
            decsc_pen_bg:    Color::Default,
            decsc_pen_attrs: Attrs::empty(),
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
            match &action {
                Action::OscDispatch { params, .. } => {
                    // OSC 7: notify CWD — params[0] = "7", params[1] = "file://host/path"
                    if params.first().map(|p| p.as_slice() == b"7").unwrap_or(false) {
                        if let Some(payload) = params.get(1) {
                            if let Ok(s) = std::str::from_utf8(payload) {
                                events.push(PaneEvent::Cwd(s.to_owned()));
                            }
                        }
                    }
                }
                Action::CsiDispatch { params, intermediates, action: 'n', .. }
                    if intermediates.is_empty() =>
                {
                    // DSR — Device Status Report.
                    // CSI 6 n → respond with CPR: ESC [ row ; col R  (1-based).
                    if params.first().copied() == Some(6) {
                        let row = self.cursor.row + 1;
                        let col = self.cursor.col + 1;
                        let cpr = format!("\x1b[{};{}R", row, col);
                        events.push(PaneEvent::PtyWrite(cpr.into_bytes()));
                    }
                }
                _ => {}
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
            Action::EscDispatch { intermediates, byte, .. } => self.esc(&intermediates, byte),
            _ => {}
        }
    }

    /// Handle ESC dispatch sequences (no CSI `[`), e.g. ESC 7 / ESC 8 / ESC M.
    fn esc(&mut self, intermediates: &[u8], byte: u8) {
        // Intermediates are for charset selection (ESC ( B etc.) — ignore those.
        if !intermediates.is_empty() { return; }
        match byte {
            // DECSC — save cursor, pen, (charset/origin) state.
            b'7' => {
                self.decsc_cursor    = Some(self.cursor);
                self.decsc_pen_fg    = self.pen_fg;
                self.decsc_pen_bg    = self.pen_bg;
                self.decsc_pen_attrs = self.pen_attrs;
            }
            // DECRC — restore cursor and pen saved by DECSC.
            b'8' => {
                if let Some(c) = self.decsc_cursor {
                    self.cursor = c;
                    self.cursor.col = self.cursor.col.min(self.grid.width.saturating_sub(1));
                    self.cursor.row = self.cursor.row.min(self.grid.height.saturating_sub(1));
                }
                self.pen_fg    = self.decsc_pen_fg;
                self.pen_bg    = self.decsc_pen_bg;
                self.pen_attrs = self.decsc_pen_attrs;
            }
            // IND — index: move down one line, scrolling if at bottom of region.
            b'D' => self.linefeed(),
            // NEL — next line: CR + LF.
            b'E' => { self.cursor.col = 0; self.linefeed(); }
            // RI — reverse index: move up one line, scrolling down if at top of region.
            b'M' => {
                if self.cursor.row == self.scroll_top {
                    self.grid.scroll_down_region(1, self.scroll_top, self.scroll_bot);
                } else if self.cursor.row > 0 {
                    self.cursor.row -= 1;
                }
            }
            // RIS — full reset.
            b'c' => {
                self.grid.clear();
                self.cursor = rusty_core::cursor::Cursor { col: 0, row: 0, visible: true };
                self.scroll_top = 0;
                self.scroll_bot = self.grid.height.saturating_sub(1);
                self.pen_fg    = Color::Default;
                self.pen_bg    = Color::Default;
                self.pen_attrs = Attrs::empty();
                self.autowrap  = true;
            }
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
                // ED 0: erase from cursor to end of screen.
                // When erasing the full screen from (0,0), scroll existing content
                // into scrollback first so history is preserved and the screen is clean.
                if row == 0 && col == 0 && !self.grid.in_alt_screen {
                    let content_rows = self.grid.last_nonempty_row().map(|r| r + 1).unwrap_or(0);
                    for r in 0..content_rows {
                        let start = r * self.grid.width;
                        let end = (start + self.grid.width).min(self.grid.cells_len());
                        let row_cells = self.grid.cells_row(start, end);
                        self.grid.push_scrollback(row_cells);
                    }
                    self.grid.clear();
                    self.scroll_off = 0;
                } else {
                    for c in col..w { self.grid.set(c, row, Cell::default()); }
                    for r in (row + 1)..h { for c in 0..w { self.grid.set(c, r, Cell::default()); } }
                }
            }
            1 => {
                for r in 0..row { for c in 0..w { self.grid.set(c, r, Cell::default()); } }
                for c in 0..=col { self.grid.set(c, row, Cell::default()); }
            }
            2 => {
                // ED 2: erase visible screen.
                // Push only content rows (up to last non-blank row) to scrollback
                // so that scrolling back doesn't show a wall of blank lines.
                if !self.grid.in_alt_screen {
                    let content_rows = self.grid.last_nonempty_row().map(|r| r + 1).unwrap_or(0);
                    for r in 0..content_rows {
                        let start = r * self.grid.width;
                        let end = (start + self.grid.width).min(self.grid.cells_len());
                        let row = self.grid.cells_row(start, end);
                        self.grid.push_scrollback(row);
                    }
                    self.grid.clear();
                } else {
                    self.grid.clear();
                }
                self.scroll_off = 0;
            }
            3 => {
                // ED(3): xterm "clear scrollback" — but we preserve history so users
                // can still scroll back after `clear`, matching iTerm2/Terminal.app behavior.
                // We treat it like ED(2): push content rows to scrollback, then blank screen.
                if !self.grid.in_alt_screen {
                    let content_rows = self.grid.last_nonempty_row().map(|r| r + 1).unwrap_or(0);
                    for r in 0..content_rows {
                        let start = r * self.grid.width;
                        let end = (start + self.grid.width).min(self.grid.cells_len());
                        let row = self.grid.cells_row(start, end);
                        self.grid.push_scrollback(row);
                    }
                }
                self.grid.clear();
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
        // Private mode sequences: CSI ? Pn [; Pn ...] h/l
        // Multiple modes may appear in one sequence (e.g. CSI ?1006;1000h).
        if intermediates == b"?" {
            for &param in params {
                match (param, action) {
                    (1049, 'h') => {
                        self.saved_cursor    = Some(self.cursor);
                        self.saved_pen_fg    = Some(self.pen_fg);
                        self.saved_pen_bg    = Some(self.pen_bg);
                        self.saved_pen_attrs = Some(self.pen_attrs);
                        self.grid.enter_alt_screen();
                        self.cursor     = rusty_core::cursor::Cursor { col: 0, row: 0, visible: true };
                        self.scroll_off = 0;
                        self.scroll_top = 0;
                        self.scroll_bot = self.grid.height.saturating_sub(1);
                        self.autowrap   = true;
                    }
                    (1049, 'l') => {
                        self.grid.leave_alt_screen();
                        if let Some(c) = self.saved_cursor.take() { self.cursor = c; }
                        if let Some(f) = self.saved_pen_fg.take()    { self.pen_fg    = f; }
                        if let Some(b) = self.saved_pen_bg.take()    { self.pen_bg    = b; }
                        if let Some(a) = self.saved_pen_attrs.take() { self.pen_attrs = a; }
                        self.scroll_off   = 0;
                        self.scroll_top   = 0;
                        self.scroll_bot   = self.grid.height.saturating_sub(1);
                        self.autowrap     = true;
                        self.mouse_report = false;
                        self.mouse_sgr    = false;
                    }
                    (25,   'l') => self.cursor.visible = false,
                    (25,   'h') => self.cursor.visible = true,
                    (7,    'l') => self.autowrap = false,
                    (7,    'h') => self.autowrap = true,
                    (1,    'h') => self.app_cursor = true,
                    (1,    'l') => self.app_cursor = false,
                    (1000, 'h') => self.mouse_report = true,
                    (1000, 'l') => self.mouse_report = false,
                    (1006, 'h') => self.mouse_sgr    = true,
                    (1006, 'l') => self.mouse_sgr    = false,
                    _ => {}
                }
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
                let new_row = (p(0, 1) as usize).saturating_sub(1).min(self.grid.height - 1);
                let new_col = (p(1, 1) as usize).saturating_sub(1).min(self.grid.width - 1);
                self.cursor.row = new_row;
                self.cursor.col = new_col;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pane() -> Pane {
        Pane::new(0, 80, 24)
    }

    #[test]
    fn alt_screen_blanks_cells() {
        let mut pane = make_pane();
        pane.process(b"hello world\r\nline two\r\n");
        let cell = pane.grid.get(0, 0);
        assert_eq!(cell.ch, 'h', "primary screen should have content");
        pane.process(b"\x1b[?1049h");
        assert!(pane.grid.in_alt_screen);
        for row in 0..pane.grid.height {
            for col in 0..pane.grid.width {
                let cell = pane.grid.get(col, row);
                assert_eq!(cell.ch, ' ', "alt screen cell ({col},{row}) should be blank, got {:?}", cell.ch);
            }
        }
        assert_eq!(pane.grid.scrollback.len(), 0, "alt screen scrollback should be empty");
        pane.process(b"\x1b[?1049l");
        assert!(!pane.grid.in_alt_screen);
        let cell = pane.grid.get(0, 0);
        assert_eq!(cell.ch, 'h', "primary screen content should be restored");
    }

    #[test]
    fn alt_screen_cursor_saved_and_restored() {
        let mut pane = make_pane();
        pane.process(b"\x1b[5;10H");
        assert_eq!(pane.cursor.row, 4);
        assert_eq!(pane.cursor.col, 9);
        pane.process(b"\x1b[?1049h");
        assert_eq!(pane.cursor.row, 0);
        assert_eq!(pane.cursor.col, 0);
        pane.process(b"\x1b[10;20H");
        pane.process(b"\x1b[?1049l");
        assert_eq!(pane.cursor.row, 4, "cursor row should be restored");
        assert_eq!(pane.cursor.col, 9, "cursor col should be restored");
    }

    #[test]
    fn ls_then_claude_no_overlap() {
        let mut pane = make_pane(); // 80x24

        for i in 0..24 {
            let line = format!("file_{:02}.txt\r\n", i);
            pane.process(line.as_bytes());
        }
        assert!(pane.grid.scrollback.len() > 0, "ls output should have pushed to scrollback");
        assert_eq!(pane.cursor.row, 23);

        pane.process(b"\x1b[?1049h");
        assert!(pane.grid.in_alt_screen);
        assert_eq!(pane.grid.scrollback.len(), 0);

        let mut non_blank = 0;
        for row in 0..24 {
            for col in 0..80 {
                if pane.grid.get(col, row).ch != ' ' {
                    non_blank += 1;
                }
            }
        }
        assert_eq!(non_blank, 0, "alt screen should have no ls content, found {non_blank} non-blank cells");
        assert_eq!(pane.grid.total_rows(), 24);
    }

    #[test]
    fn alt_screen_1049h_actually_received() {
        // Confirm the parser correctly receives 1049h and triggers enter_alt_screen
        let mut pane = make_pane();
        pane.process(b"content\r\n");
        assert!(!pane.grid.in_alt_screen);
        // Send the exact bytes that a TUI app sends: CSI ? 1049 h
        pane.process(b"\x1b[?1049h");
        assert!(pane.grid.in_alt_screen, "1049h must trigger alt screen");
        // Also confirm compound sequence works
        let mut pane2 = make_pane();
        pane2.process(b"content\r\n");
        pane2.process(b"\x1b[?2004;1049h"); // bracketed paste + alt screen together
        assert!(pane2.grid.in_alt_screen, "compound 1049h must trigger alt screen");
    }

    #[test]
    fn primary_screen_scroll_pushes_content_up() {
        // In a real terminal: ls output fills the screen, then new content (Claude)
        // is written below. Old content scrolls into scrollback.
        // The renderer uses view_start = scrollback.len() (with scroll_off=0),
        // so the viewport always shows exactly cells[0..height].
        // This invariant must hold: view_start == scrollback.len() always.
        let mut pane = make_pane(); // 80x24

        // Fill screen with ls-like content (26 lines = 2 in scrollback, 24 on screen)
        for i in 0..26 {
            let line = format!("ls_line_{:02}\r\n", i);
            pane.process(line.as_bytes());
        }
        let sb_after_ls = pane.grid.scrollback.len();
        let total = pane.grid.total_rows();
        let view_start = total.saturating_sub(pane.grid.height);
        assert_eq!(view_start, sb_after_ls,
            "view_start must equal scrollback.len() — viewport shows only cells");

        // Shell prompt + Enter
        pane.process(b"$ claude\r\n");
        let sb_after_prompt = pane.grid.scrollback.len();
        let total = pane.grid.total_rows();
        let view_start = total.saturating_sub(pane.grid.height);
        assert_eq!(view_start, sb_after_prompt,
            "view_start must still equal scrollback.len() after prompt");

        // Claude writes several lines
        for i in 0..10 {
            let line = format!("Claude line {}\r\n", i);
            pane.process(line.as_bytes());
        }
        let sb_after_claude = pane.grid.scrollback.len();
        let total = pane.grid.total_rows();
        let view_start = total.saturating_sub(pane.grid.height);
        assert_eq!(view_start, sb_after_claude,
            "view_start must equal scrollback.len() — renderer sees only cells, no scrollback bleed");

        // Confirm scrollback grew as content scrolled off top
        assert!(sb_after_claude > sb_after_ls,
            "ls content should have moved to scrollback as Claude wrote");
    }

    #[test]
    fn decsc_decrc_save_restore_cursor() {
        // ESC 7 saves the cursor; ESC 8 restores it. Without this, an app that
        // does ESC 7 ... move-cursor ... ESC 8 ends up with the wrong position.
        let mut pane = make_pane();
        pane.process(b"\x1b[10;20H"); // row 9, col 19 (0-based)
        assert_eq!((pane.cursor.row, pane.cursor.col), (9, 19));
        pane.process(b"\x1b7");        // DECSC
        pane.process(b"\x1b[1;1H");    // move to home
        assert_eq!((pane.cursor.row, pane.cursor.col), (0, 0));
        pane.process(b"\x1b8");        // DECRC
        assert_eq!((pane.cursor.row, pane.cursor.col), (9, 19),
            "DECRC must restore the cursor saved by DECSC");
    }

    #[test]
    fn claude_startup_keeps_cursor_at_bottom() {
        // Regression: Claude (Ink) starts with `ESC 7  ESC [ r  ESC 8`.
        // ESC[r (DECSTBM with no params) resets the scroll region AND homes the
        // cursor to (0,0). ESC 8 (DECRC) must then restore the cursor to the
        // bottom where the shell left it — otherwise Claude paints its UI from
        // row 0 over the existing `ls` output instead of scrolling it up.
        let mut pane = make_pane(); // 80x24

        // Simulate `ls` leaving the cursor at the bottom row.
        for i in 0..24 {
            pane.process(format!("file_{:02}\r\n", i).as_bytes());
        }
        assert_eq!(pane.cursor.row, 23, "after ls the cursor is at the bottom");

        // Claude's exact startup prologue.
        pane.process(b"\x1b7\x1b[r\x1b8");

        assert_eq!(pane.cursor.row, 23,
            "DECRC after DECSTBM must put the cursor back at the bottom, not row 0");
    }
}
