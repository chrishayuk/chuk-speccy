//! The [`Frame`] games draw into: 256×192 1-bit pixels + 32×24 attributes, laid out
//! exactly as Spectrum screen RAM (so handing it to the machine is a straight copy).

use super::colour::{Attr, Colour};
use super::tile::Tile;

pub(crate) const PIXELS: usize = 6144; // 0x4000..0x5800
pub(crate) const ATTRS: usize = 768; // 0x5800..0x5B00
/// The ROM font is 96 glyphs (chars 32..127) × 8 bytes — coincidentally the same
/// byte count as [`ATTRS`], but a distinct quantity; named separately so the two
/// don't read as the same constant by accident.
pub(crate) const FONT_BYTES: usize = 768;

/// A frame to draw into: 256×192 1-bit pixels + 32×24 attributes, in the
/// Spectrum's interleaved screen layout (so handing it to the machine is a
/// straight copy). Reused across frames; call [`Frame::clear`] each tick.
pub struct Frame {
    pub(crate) pixels: [u8; PIXELS],
    pub(crate) attrs: [u8; ATTRS],
    ink: Colour,
    paper: Colour,
    font: [u8; FONT_BYTES], // 96 glyphs × 8 bytes, lifted from the ROM
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

impl Frame {
    /// A blank frame (white ink on black). The host head supplies one each tick;
    /// games receive `&mut Frame`. Public so you can construct one to unit-test a
    /// `Game::update`.
    pub fn new() -> Self {
        Frame {
            pixels: [0; PIXELS],
            attrs: [0; ATTRS],
            ink: Colour::White,
            paper: Colour::Black,
            font: [0; FONT_BYTES],
        }
    }

    /// Clear all pixels and reset every cell to the current ink on `paper`.
    pub fn clear(&mut self, paper: Colour) {
        self.paper = paper;
        self.pixels = [0; PIXELS];
        self.attrs = [Attr::new(self.ink, paper, false).0; ATTRS];
    }

    /// Set the current ink for subsequent draws.
    pub fn ink(&mut self, c: Colour) -> &mut Self {
        self.ink = c;
        self
    }

    /// Set/clear a single pixel (top-left origin; `x` 0..256, `y` 0..192).
    pub fn pixel(&mut self, x: u8, y: u8, on: bool) {
        if y >= 192 {
            return; // x is a u8, always within the 256-wide screen
        }
        let (i, m) = pixel_at(x as usize, y as usize);
        if on {
            self.pixels[i] |= m;
        } else {
            self.pixels[i] &= !m;
        }
    }

    /// Draw a tile at cell `(cx, cy)` (0..32, 0..24) and colour the cell.
    pub fn tile(&mut self, t: &Tile, cx: u8, cy: u8) -> &mut Self {
        if cx >= 32 || cy >= 24 {
            return self;
        }
        for (r, &row) in t.rows.iter().enumerate() {
            let idx = byte_index(cx as usize * 8, cy as usize * 8 + r);
            self.pixels[idx] = row;
        }
        self.attrs[cy as usize * 32 + cx as usize] = Attr::new(self.ink, self.paper, false).0;
        self
    }

    /// Draw a **solid 8×8 block** at cell `(cx, cy)` in `ink` on black paper — the
    /// data-free sprite primitive. Unlike [`Frame::tile`] it carries no tile bytes,
    /// so it routes straight through the compiler to the pure tape (the colour is a
    /// by-value arg, not a `&Tile` to relocate); use it for grid sprites (Snake's
    /// body/food, a blob). Assumes a black background — `clear(Colour::Black)` first,
    /// or set the cell with [`Frame::attr`] for other paper.
    pub fn fill_cell(&mut self, cx: u8, cy: u8, ink: Colour) -> &mut Self {
        if cx < 32 && cy < 24 {
            for r in 0..8 {
                self.pixels[byte_index(cx as usize * 8, cy as usize * 8 + r)] = 0xFF;
            }
            self.attrs[cy as usize * 32 + cx as usize] = Attr::ink(ink).0;
        }
        self
    }

    /// Erase a cell's pixels (blank the 8×8 block; the attribute is left as-is) — the
    /// companion to [`Frame::fill_cell`] for an erase-old → move → draw-new sprite
    /// loop. Also routes to the pure tape.
    pub fn clear_cell(&mut self, cx: u8, cy: u8) -> &mut Self {
        if cx < 32 && cy < 24 {
            for r in 0..8 {
                self.pixels[byte_index(cx as usize * 8, cy as usize * 8 + r)] = 0x00;
            }
        }
        self
    }

    /// Set a cell's attribute explicitly.
    pub fn attr(&mut self, cx: u8, cy: u8, a: Attr) {
        if cx < 32 && cy < 24 {
            self.attrs[cy as usize * 32 + cx as usize] = a.0;
        }
    }

    /// Print `s` at cell `(cx, cy)` using the ROM's 8×8 font, in the current ink.
    pub fn text(&mut self, cx: u8, cy: u8, s: &str) {
        for (i, ch) in s.bytes().enumerate() {
            let col = cx as usize + i;
            if col >= 32 || cy >= 24 || !(32..127).contains(&ch) {
                continue;
            }
            let glyph = (ch as usize - 32) * 8;
            for r in 0..8 {
                let idx = byte_index(col * 8, cy as usize * 8 + r);
                self.pixels[idx] = self.font[glyph + r];
            }
            self.attrs[cy as usize * 32 + col] = Attr::new(self.ink, self.paper, false).0;
        }
    }

    /// Print `n` as decimal at cell `(cx, cy)` — a no-alloc HUD number (no
    /// `format!`/heap, so it suits the subset-clean discipline).
    pub fn text_u16(&mut self, cx: u8, cy: u8, mut n: u16) {
        let mut buf = [0u8; 5];
        let mut i = buf.len();
        loop {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        if let Ok(s) = core::str::from_utf8(&buf[i..]) {
            self.text(cx, cy, s);
        }
    }

    /// Load the ROM's 8×8 font (96 glyphs, chars 32..127) once at startup — the
    /// host runtime lifts it from ROM `$3D00`; games never call this.
    pub(crate) fn load_font(&mut self, bytes: &[u8]) {
        self.font.copy_from_slice(bytes);
    }
}

/// Pixel byte index + bit mask for `(x, y)` in the interleaved screen layout.
fn pixel_at(x: usize, y: usize) -> (usize, u8) {
    (byte_index(x, y), 0x80 >> (x & 7))
}

/// Byte index into the 6144-byte pixel area for the byte containing pixel `(x,y)` —
/// the canonical ZX interleave, from [`display::screen_byte_index`] (one source).
fn byte_index(x: usize, y: usize) -> usize {
    display::screen_byte_index(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::BLOCK;

    #[test]
    fn screen_layout_matches_zx_interleave() {
        // Row 0 byte 0 → $4000; row 1 → $4100; row 8 → $4020; third 1 → $4800.
        assert_eq!(byte_index(0, 0), 0x0000);
        assert_eq!(byte_index(0, 1), 0x0100);
        assert_eq!(byte_index(0, 8), 0x0020);
        assert_eq!(byte_index(8, 0), 0x0001);
        assert_eq!(byte_index(0, 64), 0x0800);
        assert_eq!(byte_index(255, 191), PIXELS - 1);
    }

    #[test]
    fn tile_sets_pixels_and_attr() {
        let mut f = Frame::new();
        f.ink(Colour::BrightGreen).tile(&BLOCK, 1, 2);
        // Cell (1,2): top pixel row at byte_index(8, 16).
        assert_eq!(f.pixels[byte_index(8, 16)], 0xFF);
        // attr = bright green ink (12 → ink 4, bright) on black.
        assert_eq!(f.attrs[2 * 32 + 1], 0b0100_0100);
    }

    #[test]
    fn fill_cell_draws_solid_block_in_ink() {
        let mut f = Frame::new();
        f.clear(Colour::Black);
        f.fill_cell(3, 4, Colour::BrightGreen);
        // The whole 8×8 cell is lit (top and bottom pixel rows solid)...
        assert_eq!(f.pixels[byte_index(3 * 8, 4 * 8)], 0xFF);
        assert_eq!(f.pixels[byte_index(3 * 8, 4 * 8 + 7)], 0xFF);
        // ...coloured ink-on-black (matches the prelude's by-value colour path).
        assert_eq!(f.attrs[4 * 32 + 3], Attr::ink(Colour::BrightGreen).0);
        // clear_cell blanks the pixels again.
        f.clear_cell(3, 4);
        assert_eq!(f.pixels[byte_index(3 * 8, 4 * 8)], 0x00);
        assert_eq!(f.pixels[byte_index(3 * 8, 4 * 8 + 7)], 0x00);
    }
}
