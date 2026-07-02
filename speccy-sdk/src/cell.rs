//! A small grid point type shared by [`crate::Entities`] and grid games.

/// A grid point (cell or pixel coordinate) — a small, `Copy` element type for
/// [`crate::Entities`] and grid games.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
pub struct Cell {
    pub x: u8,
    pub y: u8,
}

impl Cell {
    pub fn new(x: u8, y: u8) -> Self {
        Cell { x, y }
    }
}
