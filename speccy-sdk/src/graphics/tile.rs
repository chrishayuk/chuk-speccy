//! An 8×8 cell-aligned bitmap tile.

/// An 8×8 cell-aligned tile (one byte per pixel row, bit 7 = leftmost).
#[derive(Copy, Clone)]
pub struct Tile {
    pub rows: [u8; 8],
}

/// A solid 8×8 block — handy for grid games.
pub const BLOCK: Tile = Tile { rows: [0xFF; 8] };
