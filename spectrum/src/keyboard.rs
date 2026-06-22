//! The 8×5 keyboard matrix, read through port 0xFE.
//!
//! 8 half-rows of 5 keys. A read with the high byte selecting row(s) returns
//! bits 0..4 = pressed keys (active-low: 0 = pressed), bit 5 unused (1),
//! bit 6 = EAR (tape in), bit 7 high (`docs/01-core-emulator-spec.md` §6).

/// A physical key location in the 8×5 matrix: `row` selects the half-row (the
/// high-byte address line), `col` the key bit (0–4).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct KeyPos {
    pub row: u8,
    pub col: u8,
}

const fn k(row: u8, col: u8) -> KeyPos {
    KeyPos { row, col }
}

// The four keys every head needs by name.
pub const CAPS_SHIFT: KeyPos = k(0, 0);
pub const SYM_SHIFT: KeyPos = k(7, 1);
pub const ENTER: KeyPos = k(6, 0);
pub const SPACE: KeyPos = k(7, 0);

/// Map an ASCII character to its matrix position plus whether CAPS/SYM shift is
/// needed. Covers letters, digits, space, enter, and the common SYM-shifted
/// symbols. Returns `(pos, caps_shift, sym_shift)`.
pub fn key_for_char(ch: char) -> Option<(KeyPos, bool, bool)> {
    // Half-rows, in matrix order. Index = row; value[col] = the unshifted letter.
    // row0 CAPS,Z,X,C,V  row1 A,S,D,F,G  row2 Q,W,E,R,T  row3 1,2,3,4,5
    // row4 0,9,8,7,6     row5 P,O,I,U,Y  row6 ENTER,L,K,J,H  row7 SPACE,SYM,M,N,B
    let up = ch.to_ascii_uppercase();
    let letter = |r: u8, cols: &[char]| cols.iter().position(|&c| c == up).map(|i| k(r, i as u8));
    let pos = letter(1, &['A', 'S', 'D', 'F', 'G'])
        .or_else(|| letter(2, &['Q', 'W', 'E', 'R', 'T']))
        .or_else(|| letter(5, &['P', 'O', 'I', 'U', 'Y']))
        .or_else(|| letter(0, &['_', 'Z', 'X', 'C', 'V']))
        .or_else(|| letter(6, &['_', 'L', 'K', 'J', 'H']))
        .or_else(|| letter(7, &['_', '_', 'M', 'N', 'B']))
        .or_else(|| letter(3, &['1', '2', '3', '4', '5']))
        .or_else(|| letter(4, &['0', '9', '8', '7', '6']));
    if let Some(p) = pos {
        return Some((p, false, false));
    }
    match ch {
        ' ' => Some((SPACE, false, false)),
        '\n' | '\r' => Some((ENTER, false, false)),
        // A few SYM-shifted symbols, useful for typing BASIC expressions.
        '+' => Some((k(6, 2), false, true)),  // SYM + K
        '-' => Some((k(6, 3), false, true)),  // SYM + J
        '*' => Some((k(7, 4), false, true)),  // SYM + B
        '/' => Some((k(0, 4), false, true)),  // SYM + V
        '=' => Some((k(6, 1), false, true)),  // SYM + L
        '"' => Some((k(5, 0), false, true)),  // SYM + P
        ';' => Some((k(5, 1), false, true)),  // SYM + O
        ',' => Some((k(7, 3), false, true)),  // SYM + N
        '.' => Some((k(7, 2), false, true)),  // SYM + M
        '(' => Some((k(4, 2), false, true)),  // SYM + 8
        ')' => Some((k(4, 1), false, true)),  // SYM + 9
        _ => None,
    }
}

/// The 8 half-rows, indexed by which high-byte address line is pulled low.
/// Each entry holds the 5 key bits in 0..4 (1 = released, 0 = pressed).
pub struct Keyboard {
    rows: [u8; 8],
    /// EAR input bit (tape). Reflected into bit 6 of reads.
    pub ear: bool,
}

impl Keyboard {
    /// Append the matrix + EAR bit to a full-state blob.
    pub(crate) fn save(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.rows);
        out.push(self.ear as u8);
    }
    /// Restore the matrix + EAR bit from a blob cursor.
    pub(crate) fn load(&mut self, c: &mut crate::serialize::Cur) {
        for r in &mut self.rows {
            *r = c.u8();
        }
        self.ear = c.bool();
    }
}

impl Keyboard {
    pub fn new() -> Self {
        // All keys released => low 5 bits high.
        Self {
            rows: [0x1F; 8],
            ear: false,
        }
    }

    /// Read port 0xFE for the row(s) selected by `port`'s high byte. Active-low
    /// address lines select rows; results for multiple selected rows are ANDed.
    pub fn read(&self, port: u16) -> u8 {
        let select = (port >> 8) as u8;
        let mut keys = 0x1F;
        for (i, &row) in self.rows.iter().enumerate() {
            if select & (1 << i) == 0 {
                keys &= row;
            }
        }
        let ear = if self.ear { 0x40 } else { 0 };
        // bits 0..4 = keys, bit 5 = 1 (unused), bit 6 = EAR, bit 7 = 1.
        (keys & 0x1F) | 0x20 | ear | 0x80
    }

    /// Press/release a key by (row, col) where row is 0..7 and col is 0..4.
    pub fn set_key(&mut self, row: usize, col: usize, pressed: bool) {
        if row < 8 && col < 5 {
            if pressed {
                self.rows[row] &= !(1 << col);
            } else {
                self.rows[row] |= 1 << col;
            }
        }
    }

    /// Release every key.
    pub fn release_all(&mut self) {
        self.rows = [0x1F; 8];
    }

    /// True if the key at `pos` is currently held (its active-low matrix bit = 0).
    pub fn is_pressed(&self, pos: KeyPos) -> bool {
        let (row, col) = (pos.row as usize, pos.col as usize);
        row < 8 && col < 5 && self.rows[row] & (1 << col) == 0
    }
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}
