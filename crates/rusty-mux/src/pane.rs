use rusty_core::{Grid, Cell};
use rusty_core::parser::Parser;
use rusty_core::cursor::Cursor;

/// One terminal pane: its own PTY, parser, grid, and cursor.
/// The mux layer owns panes; the renderer reads from them.
pub struct Pane {
    pub id:     u32,
    pub grid:   Grid,
    pub cursor: Cursor,
    parser:     Parser,
}

impl Pane {
    pub fn new(id: u32, cols: usize, rows: usize) -> Self {
        Self {
            id,
            grid:   Grid::new(cols, rows),
            cursor: Cursor { col: 0, row: 0, visible: true },
            parser: Parser::new(),
        }
    }

    /// Feed raw PTY bytes into this pane, updating the grid.
    pub fn process(&mut self, bytes: &[u8]) {
        use rusty_core::parser::Action;
        for action in self.parser.advance(bytes) {
            match action {
                Action::Print(ch) => {
                    let cell = Cell { ch, ..Cell::default() };
                    self.grid.set(self.cursor.col, self.cursor.row, cell);
                    self.cursor.col += 1;
                    if self.cursor.col >= self.grid.width {
                        self.cursor.col  = 0;
                        self.cursor.row += 1;
                    }
                }
                Action::Execute(0x0a) => {
                    self.cursor.row += 1;
                    if self.cursor.row >= self.grid.height {
                        self.cursor.row = self.grid.height - 1;
                    }
                }
                Action::Execute(0x0d) => {
                    self.cursor.col = 0;
                }
                _ => {}
            }
        }
    }
}
