bitflags::bitflags! {
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Attrs: u8 {
        const BOLD      = 0b0000_0001;
        const ITALIC    = 0b0000_0010;
        const UNDERLINE = 0b0000_0100;
        const BLINK     = 0b0000_1000;
        const REVERSE   = 0b0001_0000;
        const STRIKETHROUGH = 0b0010_0000;
    }
}
