#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Default for Color {
    fn default() -> Self { Self::Default }
}

// ── Theme constants ───────────────────────────────────────────────────────────
// Background: near-black with a very slight warm tint so it doesn't feel cold.
pub const BG:      [u8; 4] = [0x13, 0x13, 0x13, 0xff];
// Foreground: off-white, easier on eyes than pure white.
pub const FG:      [u8; 4] = [0xd8, 0xd8, 0xd8, 0xff];
// Cursor block.
pub const CURSOR:  [u8; 4] = [0xf8, 0xf8, 0xf0, 0xff];
// Selection highlight.
pub const SEL_BG:  [u8; 4] = [0x26, 0x4f, 0x78, 0xff];
pub const SEL_FG:  [u8; 4] = [0xff, 0xff, 0xff, 0xff];

impl Color {
    pub fn to_rgba(self, is_fg: bool) -> [u8; 4] {
        match self {
            Self::Default    => if is_fg { FG } else { BG },
            Self::Rgb(r,g,b) => [r, g, b, 0xff],
            Self::Indexed(i) => ansi16_or_256(i),
        }
    }
}

/// The first 16 ANSI colors tuned for dark backgrounds.
/// Bright variants (8-15) are the ones most tools use for emphasis.
const ANSI16: [[u8; 3]; 16] = [
    // Normal (0-7)
    [0x13, 0x13, 0x13], // 0  black        (same as BG — true black looks like a hole)
    [0xe0, 0x5a, 0x4f], // 1  red          bright enough on dark, not eye-searing
    [0x87, 0xc3, 0x6c], // 2  green
    [0xe5, 0xc0, 0x76], // 3  yellow
    [0x6b, 0xa3, 0xe0], // 4  blue         lightened so it's legible (pure blue is dark)
    [0xc0, 0x7d, 0xd4], // 5  magenta
    [0x5b, 0xc8, 0xd4], // 6  cyan
    [0xc5, 0xc5, 0xc5], // 7  white        (slightly dimmed so it differs from bright)
    // Bright (8-15)
    [0x52, 0x52, 0x52], // 8  bright black (dark grey)
    [0xff, 0x7a, 0x70], // 9  bright red
    [0xa8, 0xe0, 0x8a], // 10 bright green
    [0xff, 0xdc, 0x9a], // 11 bright yellow
    [0x8f, 0xc3, 0xff], // 12 bright blue
    [0xda, 0x9f, 0xf5], // 13 bright magenta
    [0x7f, 0xe3, 0xee], // 14 bright cyan
    [0xff, 0xff, 0xff], // 15 bright white
];

pub fn ansi16_or_256(i: u8) -> [u8; 4] {
    if (i as usize) < ANSI16.len() {
        let [r, g, b] = ANSI16[i as usize];
        return [r, g, b, 0xff];
    }
    // 6×6×6 color cube (indices 16-231)
    if i <= 231 {
        let i = i - 16;
        let b = i % 6;
        let g = (i / 6) % 6;
        let r = i / 36;
        let f = |v: u8| if v == 0 { 0u8 } else { 55 + v * 40 };
        return [f(r), f(g), f(b), 0xff];
    }
    // Grayscale ramp (indices 232-255)
    let v = 8 + (i - 232) * 10;
    [v, v, v, 0xff]
}
