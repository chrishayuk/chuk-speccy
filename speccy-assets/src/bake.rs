//! Bake an image into subset-clean `const Tile` data for the SDK's `Frame::tile`
//! (`docs/08` §5, the L0 asset pipeline). The screen converter ([`crate::convert`])
//! turns a full 256×192 image into a runtime `.scr` *blob*; this turns art of any
//! size (a multiple of 8 in each dimension) into **authored game data** —
//! `const Tile { rows: [u8; 8] }` you paste into a `Game` and draw with
//! `frame.ink(Colour::..).tile(&TILE, cx, cy)`.
//!
//! Each 8×8 source cell goes through the *same* two-colour reducer the screen
//! converter uses, so a baked tile renders identically to the same art on a `.scr`,
//! and every tile that wanted more than two colours is flagged as an attribute clash
//! — the "where will this break on hardware" report, now at sprite granularity.
//!
//! ```no_run
//! let rgb = vec![[0u8, 0, 0]; 16 * 16];                 // a 16×16 (2×2-tile) sprite
//! let sheet = speccy_assets::bake::bake(&rgb, 16, 16).unwrap();
//! println!("{}", sheet.to_rust("HERO"));                // -> `pub const HERO: [Tile; 4] = …`
//! println!("{} of {} tiles clash", sheet.clashes(), sheet.tiles.len());
//! ```

use crate::{cell_at, reduce_cell};

/// One baked 8×8 tile: the bitmap `rows` (bit 7 = leftmost; a set bit is an `ink`
/// pixel — the convention `Frame::tile` draws), the recommended `ink`/`paper` hue
/// (`0..8`) with shared `bright`, and whether the source wanted more than two colours
/// here (an unavoidable attribute clash).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileArt {
    pub rows: [u8; 8],
    pub ink: u8,
    pub paper: u8,
    pub bright: bool,
    pub clash: bool,
}

impl TileArt {
    /// The SDK `Colour` variant name for the recommended ink (`frame.ink(Colour::..)`).
    pub fn ink_name(&self) -> &'static str {
        colour_name(self.ink, self.bright)
    }
    /// The SDK `Colour` variant name for the recommended paper.
    pub fn paper_name(&self) -> &'static str {
        colour_name(self.paper, self.bright)
    }
}

/// A grid of baked tiles (row-major, `cols` across × `rows` down) — the result of
/// baking one image. A single 8×8 image yields a 1×1 sheet.
pub struct TileSheet {
    pub cols: usize,
    pub rows: usize,
    pub tiles: Vec<TileArt>,
}

impl TileSheet {
    /// How many tiles carry an unavoidable attribute clash.
    pub fn clashes(&self) -> usize {
        self.tiles.iter().filter(|t| t.clash).count()
    }

    /// Emit the sheet as Rust source: a `const NAME: Tile` for a single tile, or a
    /// `const NAME: [Tile; N]` (row-major) for a sheet. Rows are binary literals so
    /// the pixel art stays legible in source. Paste into a game with
    /// `use speccy_sdk::Tile;` and draw with `frame.ink(Colour::..).tile(&NAME, cx, cy)`.
    pub fn to_rust(&self, name: &str) -> String {
        let id = ident(name);
        let mut out = String::new();
        if let [t] = self.tiles.as_slice() {
            out.push_str(&format!(
                "/// `{id}` — 8×8 tile, ink `{}` on `{}`{}.\n\
                 /// Draw with `frame.ink(Colour::{}).tile(&{id}, cx, cy);` (needs `use speccy_sdk::Tile;`).\n",
                t.ink_name(),
                t.paper_name(),
                clash_note(t.clash),
                t.ink_name(),
            ));
            out.push_str(&format!(
                "pub const {id}: Tile = {};\n",
                tile_literal(t, "")
            ));
        } else {
            // A sheet can mix colours per tile, so the recommended ink is annotated
            // per element (not once in the header) — `frame.ink(..)` per tile drawn.
            out.push_str(&format!(
                "/// `{id}` — {}×{} tiles ({}×{}px), row-major. {} clash(es). Each tile's\n\
                 /// recommended ink is in its comment. Needs `use speccy_sdk::{{Tile, Colour}};`.\n",
                self.cols,
                self.rows,
                self.cols * 8,
                self.rows * 8,
                self.clashes(),
            ));
            out.push_str(&format!(
                "pub const {id}: [Tile; {}] = [\n",
                self.tiles.len()
            ));
            for (i, t) in self.tiles.iter().enumerate() {
                let (tx, ty) = (i % self.cols, i / self.cols);
                out.push_str(&format!(
                    "    // ({tx},{ty}) ink {}{}\n",
                    t.ink_name(),
                    clash_note(t.clash)
                ));
                out.push_str(&format!("    {},\n", tile_literal(t, "    ")));
            }
            out.push_str("];\n");
        }
        out
    }
}

/// Bake a `w×h` RGB image (row-major `[r, g, b]`, both dimensions a non-zero multiple
/// of 8) into a [`TileSheet`]. Errors on an empty / odd-sized buffer.
pub fn bake(rgb: &[[u8; 3]], w: usize, h: usize) -> Result<TileSheet, String> {
    if w == 0 || h == 0 || !w.is_multiple_of(8) || !h.is_multiple_of(8) {
        return Err(format!(
            "size {w}x{h}: both dimensions must be a non-zero multiple of 8"
        ));
    }
    if rgb.len() != w * h {
        return Err(format!(
            "buffer has {} pixels, expected {}",
            rgb.len(),
            w * h
        ));
    }

    let (cols, rows) = (w / 8, h / 8);
    let mut tiles = Vec::with_capacity(cols * rows);
    for ty in 0..rows {
        for tx in 0..cols {
            tiles.push(resolve(&reduce_cell(&cell_at(rgb, w, tx, ty))));
        }
    }
    Ok(TileSheet { cols, rows, tiles })
}

/// Resolve one reduced cell into a tile under the **sprite convention**: a set bit is
/// a *foreground* pixel and the recommended `ink` is the foreground colour, so
/// `frame.ink(Colour::<ink>).tile(&T, ..)` draws the shape as authored. The screen
/// reducer just minimises colour error and parks the lower hue in `ink` — fine for a
/// `.scr` (the attr carries both colours) but inverted for a tile, where the *current*
/// ink paints the set bits. Background is black when the cell contains it (the usual
/// sprite case), else the colour covering the most pixels; a solid cell is all-ink.
fn resolve(art: &crate::CellArt) -> TileArt {
    let set = art.rows.iter().map(|b| b.count_ones()).sum::<u32>();
    let bright = art.bright != 0;

    // Solid: one present colour fills the cell — make it the ink, every bit set.
    if art.distinct == 1 {
        let hue = if set == 0 { art.paper } else { art.ink };
        return TileArt {
            rows: [0xFF; 8],
            ink: hue as u8,
            paper: 0,
            bright,
            clash: false,
        };
    }

    // Two+ colours: pick the background. Black is background if present (hue 0);
    // otherwise the larger (currently-set) area is. Foreground becomes the set bits.
    let invert = |r: [u8; 8]| r.map(|b| !b);
    let (ink, paper, rows) = if art.ink == 0 {
        (art.paper, art.ink, invert(art.rows)) // black parked in `ink` → it's the bg
    } else if art.paper == 0 || set <= 32 {
        (art.ink, art.paper, art.rows) // bg already cleared (paper), or fg is the minority
    } else {
        (art.paper, art.ink, invert(art.rows)) // the majority set area is the bg
    };
    TileArt {
        rows,
        ink: ink as u8,
        paper: paper as u8,
        bright,
        clash: art.distinct > 2,
    }
}

fn clash_note(clash: bool) -> &'static str {
    if clash {
        " — ⚠ attribute clash (wanted >2 colours)"
    } else {
        ""
    }
}

/// One tile as a `Tile { rows: [..] }` literal, rows as binary literals (the art
/// stays readable in source), wrapped/indented for the enclosing context.
fn tile_literal(t: &TileArt, indent: &str) -> String {
    let mut s = String::from("Tile {\n");
    s.push_str(&format!("{indent}    rows: [\n"));
    for &b in &t.rows {
        s.push_str(&format!("{indent}        0b{b:08b},\n"));
    }
    s.push_str(&format!("{indent}    ],\n"));
    s.push_str(&format!("{indent}}}"));
    s
}

/// The SDK `Colour` variant name for a hue (`0..8`) + `bright`. Hue 0 is always
/// `Black` (there is no `BrightBlack` in the palette or the SDK enum).
fn colour_name(hue: u8, bright: bool) -> &'static str {
    const BASE: [&str; 8] = [
        "Black", "Blue", "Red", "Magenta", "Green", "Cyan", "Yellow", "White",
    ];
    const BRIGHT: [&str; 8] = [
        "Black",
        "BrightBlue",
        "BrightRed",
        "BrightMagenta",
        "BrightGreen",
        "BrightCyan",
        "BrightYellow",
        "BrightWhite",
    ];
    let h = (hue & 7) as usize;
    if bright {
        BRIGHT[h]
    } else {
        BASE[h]
    }
}

/// Turn a free-form name into a SCREAMING_SNAKE_CASE Rust const identifier
/// (`"my-sprite 2"` → `MY_SPRITE_2`); prefixed `T_` if it would start with a digit,
/// `TILE` if empty.
fn ident(name: &str) -> String {
    let mut out = String::new();
    let mut sep = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if sep && !out.is_empty() {
                out.push('_');
            }
            out.extend(ch.to_uppercase());
            sep = false;
        } else {
            sep = true; // word boundary
        }
    }
    if out.is_empty() {
        "TILE".to_string()
    } else if out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("T_{out}")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w×h` solid-colour buffer.
    fn solid(c: [u8; 3], w: usize, h: usize) -> Vec<[u8; 3]> {
        vec![c; w * h]
    }

    const GREEN: [u8; 3] = [0x00, 0xC0, 0x00];
    const RED: [u8; 3] = [0xC0, 0x00, 0x00];
    const WHITE: [u8; 3] = [0xC0, 0xC0, 0xC0];

    #[test]
    fn solid_tile_is_clean_and_solid() {
        let sheet = bake(&solid(GREEN, 8, 8), 8, 8).unwrap();
        assert_eq!(sheet.tiles.len(), 1);
        assert_eq!((sheet.cols, sheet.rows), (1, 1));
        let t = sheet.tiles[0];
        assert!(!t.clash, "one colour cannot clash");
        assert_eq!(t.rows, [0xFF; 8], "a solid cell is all ink pixels");
        assert!(t.ink_name().contains("Green"));
    }

    #[test]
    fn two_colours_split_a_row_without_clash() {
        // Left 4 columns red, right 4 white — exactly two colours.
        let mut buf = solid([0, 0, 0], 8, 8);
        for y in 0..8 {
            for x in 0..8 {
                buf[y * 8 + x] = if x < 4 { RED } else { WHITE };
            }
        }
        let sheet = bake(&buf, 8, 8).unwrap();
        let t = sheet.tiles[0];
        assert!(!t.clash);
        // Which half is "ink" depends on the chosen pair, but it's a clean 4/4 split.
        assert!(
            t.rows[0] == 0xF0 || t.rows[0] == 0x0F,
            "left/right 4-pixel split, got {:08b}",
            t.rows[0]
        );
    }

    #[test]
    fn three_colours_flag_a_clash() {
        let mut buf = solid([0, 0, 0], 8, 8);
        let cols = [RED, GREEN, [0, 0, 0xC0]];
        for c in 0..8 {
            buf[c] = cols[c % 3]; // row 0 alone already wants 3 colours
        }
        let sheet = bake(&buf, 8, 8).unwrap();
        assert!(sheet.tiles[0].clash);
        assert_eq!(sheet.clashes(), 1);
    }

    #[test]
    fn sheet_is_row_major() {
        // 16×8: left tile green, right tile red.
        let mut buf = solid([0, 0, 0], 16, 8);
        for y in 0..8 {
            for x in 0..16 {
                buf[y * 16 + x] = if x < 8 { GREEN } else { RED };
            }
        }
        let sheet = bake(&buf, 16, 8).unwrap();
        assert_eq!((sheet.cols, sheet.rows), (2, 1));
        assert!(sheet.tiles[0].ink_name().contains("Green"));
        assert!(sheet.tiles[1].ink_name().contains("Red"));
    }

    #[test]
    fn rejects_sizes_that_are_not_multiples_of_8() {
        assert!(bake(&solid(GREEN, 8, 7), 8, 7).is_err());
        assert!(
            bake(&solid(GREEN, 8, 8), 10, 8).is_err(),
            "buffer/size mismatch"
        );
        assert!(bake(&[], 0, 0).is_err());
    }

    #[test]
    fn to_rust_single_tile_is_a_pasteable_const() {
        let sheet = bake(&solid(GREEN, 8, 8), 8, 8).unwrap();
        let src = sheet.to_rust("hero");
        assert!(src.contains("pub const HERO: Tile = Tile {"), "{src}");
        assert!(src.contains("0b11111111"), "binary rows present: {src}");
        assert!(src.contains("frame.ink(Colour::"), "usage hint present");
    }

    #[test]
    fn to_rust_sheet_is_an_array_const() {
        let buf = solid(GREEN, 16, 8);
        let src = bake(&buf, 16, 8).unwrap().to_rust("tiles");
        assert!(src.contains("pub const TILES: [Tile; 2] = ["), "{src}");
        assert!(
            src.contains("// (0,0)") && src.contains("// (1,0)"),
            "row-major comments"
        );
    }

    #[test]
    fn ident_makes_screaming_snake_case() {
        assert_eq!(ident("hero"), "HERO");
        assert_eq!(ident("my-sprite 2"), "MY_SPRITE_2");
        assert_eq!(ident("2bad"), "T_2BAD");
        assert_eq!(ident("!!!"), "TILE");
    }
}
