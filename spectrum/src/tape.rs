//! `.tap` tape support via ROM-trap fast loading. Rather than emulating the
//! tape's edge timing, we trap the 48K ROM's `LD-BYTES` routine at `0x0556` and
//! splat the next block straight into memory (`docs/01-core-emulator-spec.md`
//! §7). Real-time edge loading is a later refinement.

use crate::snapshot::SnapshotError;

/// The 48K ROM `LD-BYTES` entry point. With the standard ROM, every tape read
/// passes through here with: `A` = expected flag byte (0x00 header / 0xFF data),
/// carry = LOAD(1)/VERIFY(0), `IX` = destination, `DE` = byte count.
pub const LD_BYTES: u16 = 0x0556;

/// A parsed `.tap`: an ordered list of blocks, each `flag .. data .. checksum`.
pub struct Tape {
    blocks: Vec<Vec<u8>>,
    pos: usize,
}

impl Tape {
    /// Parse a `.tap`: repeated `[u16 little-endian length][length bytes]`.
    pub fn from_tap(data: &[u8]) -> Result<Self, SnapshotError> {
        let mut blocks = Vec::new();
        let mut i = 0;
        while i + 2 <= data.len() {
            let len = (data[i] as usize) | ((data[i + 1] as usize) << 8);
            i += 2;
            if len == 0 || i + len > data.len() {
                break;
            }
            blocks.push(data[i..i + len].to_vec());
            i += len;
        }
        if blocks.is_empty() {
            return Err(SnapshotError::Truncated);
        }
        Ok(Self { blocks, pos: 0 })
    }

    /// Take the next block (consuming it), or None when the tape is exhausted.
    pub fn next_block(&mut self) -> Option<Vec<u8>> {
        let b = self.blocks.get(self.pos)?.clone();
        self.pos += 1;
        Some(b)
    }

    /// Rewind to the first block.
    pub fn rewind(&mut self) {
        self.pos = 0;
    }

    /// True once every block has been read.
    pub fn finished(&self) -> bool {
        self.pos >= self.blocks.len()
    }
}
