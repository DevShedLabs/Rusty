#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const BLACK:   Self = Self::rgb(0,   0,   0);
    pub const WHITE:   Self = Self::rgb(255, 255, 255);
    pub const DEFAULT: Self = Self { r: 0, g: 0, b: 0, a: 0 };
}
