/// A rendered document — a list of lines, each a list of styled spans.
/// This is what the window renders instead of the raw terminal output.
#[derive(Debug, Clone, Default)]
pub struct RenderDoc {
    pub lines: Vec<Vec<Span>>,
}

impl RenderDoc {
    pub fn new() -> Self { Self::default() }

    pub fn push_line(&mut self, spans: Vec<Span>) {
        self.lines.push(spans);
    }

    pub fn push_text(&mut self, text: &str, style: Style) {
        self.lines.push(vec![Span { text: text.to_owned(), style }]);
    }
}

#[derive(Debug, Clone)]
pub struct Span {
    pub text:  String,
    pub style: Style,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), style: Style::default() }
    }
    pub fn colored(text: impl Into<String>, fg: Color) -> Self {
        Self { text: text.into(), style: Style { fg: Some(fg), ..Style::default() } }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Style {
    pub fg:        Option<Color>,
    pub bg:        Option<Color>,
    pub bold:      bool,
    pub italic:    bool,
    pub underline: bool,
    pub dim:       bool,
}

/// sRGB color for a span. Matches the config palette naming where possible.
#[derive(Debug, Clone, Copy)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub fn to_rgba(self) -> [u8; 4] { [self.0, self.1, self.2, 0xff] }

    // Semantic palette — chosen to read well on the dark `#131313` background.
    pub const FG:       Self = Self(0xd8, 0xd8, 0xd8);
    pub const DIM:      Self = Self(0x70, 0x70, 0x70);
    pub const RED:      Self = Self(0xe0, 0x5a, 0x4f);
    pub const GREEN:    Self = Self(0x87, 0xc3, 0x6c);
    pub const YELLOW:   Self = Self(0xe5, 0xc0, 0x76);
    pub const BLUE:     Self = Self(0x8f, 0xc3, 0xff);
    pub const MAGENTA:  Self = Self(0xc0, 0x7d, 0xd4);
    pub const CYAN:     Self = Self(0x5b, 0xc8, 0xd4);
    pub const WHITE:    Self = Self(0xff, 0xff, 0xff);
    pub const ORANGE:   Self = Self(0xff, 0x99, 0x57);
    pub const BG_PANEL: Self = Self(0x1a, 0x1a, 0x2a); // slightly lighter than terminal bg
    pub const BG_CODE:  Self = Self(0x22, 0x22, 0x33);
}
