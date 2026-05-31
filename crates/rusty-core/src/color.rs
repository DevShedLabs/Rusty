#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Default for Color {
    fn default() -> Self { Self::Default }
}

// ── Compile-time defaults (used until config is loaded) ───────────────────────

pub const BG:     [u8; 4] = [0x13, 0x13, 0x13, 0xff];
pub const FG:     [u8; 4] = [0xd8, 0xd8, 0xd8, 0xff];
pub const CURSOR: [u8; 4] = [0xf8, 0xf8, 0xf0, 0xff];
pub const SEL_BG: [u8; 4] = [0x26, 0x4f, 0x78, 0xff];
pub const SEL_FG: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

impl Color {
    /// Resolve using the provided palette table.
    /// `ansi16[0..16]` = ANSI colors, `bg`/`fg` = default terminal colors.
    pub fn resolve(self, is_fg: bool, bg: [u8;4], fg: [u8;4], ansi16: &[[u8;4]; 16]) -> [u8; 4] {
        match self {
            Self::Default        => if is_fg { fg } else { bg },
            Self::Rgb(r, g, b)   => [r, g, b, 0xff],
            Self::Indexed(i)     => indexed(i, ansi16),
        }
    }

    /// Resolve using the compile-time defaults (used in contexts without a config).
    pub fn to_rgba(self, is_fg: bool) -> [u8; 4] {
        self.resolve(is_fg, BG, FG, &DEFAULT_ANSI16)
    }
}

pub const DEFAULT_ANSI16: [[u8; 4]; 16] = [
    [0x13, 0x13, 0x13, 0xff], // 0  black
    [0xe0, 0x5a, 0x4f, 0xff], // 1  red
    [0x87, 0xc3, 0x6c, 0xff], // 2  green
    [0xe5, 0xc0, 0x76, 0xff], // 3  yellow
    [0x6b, 0xa3, 0xe0, 0xff], // 4  blue
    [0xc0, 0x7d, 0xd4, 0xff], // 5  magenta
    [0x5b, 0xc8, 0xd4, 0xff], // 6  cyan
    [0xc5, 0xc5, 0xc5, 0xff], // 7  white
    [0x52, 0x52, 0x52, 0xff], // 8  bright black
    [0xff, 0x7a, 0x70, 0xff], // 9  bright red
    [0xa8, 0xe0, 0x8a, 0xff], // 10 bright green
    [0xff, 0xdc, 0x9a, 0xff], // 11 bright yellow
    [0x8f, 0xc3, 0xff, 0xff], // 12 bright blue
    [0xda, 0x9f, 0xf5, 0xff], // 13 bright magenta
    [0x7f, 0xe3, 0xee, 0xff], // 14 bright cyan
    [0xff, 0xff, 0xff, 0xff], // 15 bright white
];

pub fn indexed(i: u8, ansi16: &[[u8; 4]; 16]) -> [u8; 4] {
    if (i as usize) < 16 {
        return ansi16[i as usize];
    }
    if i <= 231 {
        let i = i - 16;
        let b = i % 6;
        let g = (i / 6) % 6;
        let r = i / 36;
        let f = |v: u8| if v == 0 { 0u8 } else { 55 + v * 40 };
        return [f(r), f(g), f(b), 0xff];
    }
    let v = 8 + (i - 232) * 10;
    [v, v, v, 0xff]
}
