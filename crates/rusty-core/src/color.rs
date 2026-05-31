#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Default for Color {
    fn default() -> Self { Self::Default }
}

impl Color {
    /// Resolve to RGBA bytes for the renderer. `is_fg` picks the fallback.
    pub fn to_rgba(self, is_fg: bool) -> [u8; 4] {
        match self {
            Self::Default => if is_fg { [0xcc, 0xcc, 0xcc, 0xff] } else { [0x1a, 0x1a, 0x2e, 0xff] },
            Self::Rgb(r, g, b) => [r, g, b, 0xff],
            Self::Indexed(i) => ansi256(i),
        }
    }
}

/// Standard xterm-256 palette.
pub fn ansi256(i: u8) -> [u8; 4] {
    let (r, g, b) = match i {
        // First 16: standard ANSI
        0  => (0x1a, 0x1a, 0x2e),
        1  => (0xcc, 0x00, 0x00),
        2  => (0x00, 0xaa, 0x00),
        3  => (0xcc, 0xaa, 0x00),
        4  => (0x00, 0x00, 0xcc),
        5  => (0xaa, 0x00, 0xaa),
        6  => (0x00, 0xaa, 0xaa),
        7  => (0xcc, 0xcc, 0xcc),
        8  => (0x55, 0x55, 0x55),
        9  => (0xff, 0x55, 0x55),
        10 => (0x55, 0xff, 0x55),
        11 => (0xff, 0xff, 0x55),
        12 => (0x55, 0x55, 0xff),
        13 => (0xff, 0x55, 0xff),
        14 => (0x55, 0xff, 0xff),
        15 => (0xff, 0xff, 0xff),
        // 6×6×6 color cube
        16..=231 => {
            let i = i - 16;
            let b = i % 6;
            let g = (i / 6) % 6;
            let r = i / 36;
            let f = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (f(r), f(g), f(b))
        }
        // Grayscale ramp
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
    };
    [r, g, b, 0xff]
}
