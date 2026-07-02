//! The 8 Spectrum colours and the per-cell attribute byte they pack into.

/// The 8 Spectrum colours, with bright variants where useful. `as u8` packs
/// `ink | bright<<3`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Colour {
    Black = 0,
    Blue = 1,
    Red = 2,
    Magenta = 3,
    Green = 4,
    Cyan = 5,
    Yellow = 6,
    White = 7,
    BrightBlue = 9,
    BrightRed = 10,
    BrightMagenta = 11,
    BrightGreen = 12,
    BrightCyan = 13,
    BrightYellow = 14,
    BrightWhite = 15,
}

impl Colour {
    fn ink(self) -> u8 {
        self as u8 & 7
    }
    fn bright(self) -> bool {
        self as u8 & 8 != 0
    }
}

/// A per-cell attribute byte: `flash<<7 | bright<<6 | paper<<3 | ink`.
#[derive(Copy, Clone)]
pub struct Attr(pub u8);

impl Attr {
    pub fn new(ink: Colour, paper: Colour, flash: bool) -> Self {
        let bright = (ink.bright() || paper.bright()) as u8;
        Attr((flash as u8) << 7 | bright << 6 | paper.ink() << 3 | ink.ink())
    }
    /// `ink` on a black paper.
    pub fn ink(c: Colour) -> Self {
        Attr::new(c, Colour::Black, false)
    }
}
