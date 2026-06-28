//! Convert an RGB image into ZX Spectrum screen data — the host-side asset half of
//! the authoring plane (`docs/roadmap.md` E.2). The Spectrum can show only **two
//! colours per 8×8 cell** (one `ink`, one `paper`, sharing one `bright` bit), so a
//! conversion is a *reduction*: each cell is mapped to the two-colour pair that best
//! matches its 64 source pixels, and every cell that wanted **more than two** colours
//! is reported as an **attribute clash** — the "where will this art break on real
//! hardware" report that's the cheap demo-magnet.
//!
//! The 16 logical colours come from [`display::AUTHENTIC`] (the one palette the heads
//! already render), so a converted image looks identical on screen and in this report.
//!
//! ```no_run
//! let rgb = vec![[0u8, 0, 0]; 256 * 192];                     // your decoded image
//! let img = speccy_assets::convert(&rgb, 256, 192).unwrap();
//! std::fs::write("art.scr", img.to_scr()).unwrap();           // drop-in 6912-byte screen
//! println!("{} attribute clashes", img.clashes.len());
//! ```

use display::{screen_byte_index, AUTHENTIC};

/// The native Spectrum screen, in pixels (re-exported from `display` — one source).
pub const SCREEN_W: usize = display::SCREEN_W;
pub const SCREEN_H: usize = display::SCREEN_H;
/// …and in 8×8 character cells.
pub const COLS: usize = SCREEN_W / 8; // 32
pub const ROWS: usize = SCREEN_H / 8; // 24

const PIXELS: usize = 6144; // 0x4000..0x5800, ZX-interleaved bitmap
const ATTRS: usize = COLS * ROWS; // 768, 0x5800..0x5B00

/// A cell whose source art used **more than two** distinct Spectrum colours, so the
/// reduction to ink/paper had to drop some — a classic "colour clash" the Spectrum
/// can't avoid. Authoring tools surface these so the artist can fix the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clash {
    pub cx: u8,
    pub cy: u8,
    /// How many distinct logical colours the source wanted here (always > 2).
    pub colours: u8,
}

/// A converted image in Spectrum screen format: the interleaved 1-bit `pixels` (drop
/// in at `0x4000`), the per-cell `attrs` (drop in at `0x5800`), and the clash report.
pub struct SpectrumImage {
    pub pixels: [u8; PIXELS],
    pub attrs: [u8; ATTRS],
    pub clashes: Vec<Clash>,
}

impl SpectrumImage {
    /// The standard **6912-byte `.scr`** (pixels then attributes) — loadable by any
    /// emulator, or pokeable straight to screen RAM.
    pub fn to_scr(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(PIXELS + ATTRS);
        v.extend_from_slice(&self.pixels);
        v.extend_from_slice(&self.attrs);
        v
    }
}

/// Convert a **256×192** RGB image (row-major, `w*h` `[r,g,b]` triples) to Spectrum
/// screen data. Each 8×8 cell is reduced to the (ink, paper, bright) that minimises
/// the per-pixel colour error, and cells that wanted more than two colours are
/// reported in [`SpectrumImage::clashes`]. Errors only on a wrong-sized buffer.
pub fn convert(rgb: &[[u8; 3]], w: usize, h: usize) -> Result<SpectrumImage, String> {
    if (w, h) != (SCREEN_W, SCREEN_H) {
        return Err(format!("expected {SCREEN_W}x{SCREEN_H}, got {w}x{h}"));
    }
    if rgb.len() != w * h {
        return Err(format!("buffer has {} pixels, expected {}", rgb.len(), w * h));
    }

    let mut pixels = [0u8; PIXELS];
    let mut attrs = [0u8; ATTRS];
    let mut clashes = Vec::new();

    for cy in 0..ROWS {
        for cx in 0..COLS {
            // Snapshot the cell's 64 source pixels once.
            let mut cell = [[0u8; 3]; 64];
            for r in 0..8 {
                for c in 0..8 {
                    cell[r * 8 + c] = rgb[(cy * 8 + r) * w + (cx * 8 + c)];
                }
            }

            // Report a clash if the source wanted more than two colours here.
            let mut present = [false; 16];
            for &p in &cell {
                present[nearest(p) as usize] = true;
            }
            let distinct = present.iter().filter(|&&b| b).count();
            if distinct > 2 {
                clashes.push(Clash {
                    cx: cx as u8,
                    cy: cy as u8,
                    colours: distinct as u8,
                });
            }

            // Pick the best two-colour pair, then set the bitmap + attribute.
            let (ink, paper, bright) = choose_attr(&cell);
            let ink_rgb = AUTHENTIC[bright * 8 + ink];
            let paper_rgb = AUTHENTIC[bright * 8 + paper];
            for r in 0..8 {
                let mut byte = 0u8;
                for c in 0..8 {
                    let p = cell[r * 8 + c];
                    // Ink bit set when the pixel is nearer the ink colour.
                    if dist2(p, ink_rgb) <= dist2(p, paper_rgb) {
                        byte |= 0x80 >> c;
                    }
                }
                pixels[screen_byte_index(cx * 8, cy * 8 + r)] = byte;
            }
            attrs[cy * COLS + cx] = ((bright as u8) << 6) | ((paper as u8) << 3) | ink as u8;
        }
    }

    Ok(SpectrumImage {
        pixels,
        attrs,
        clashes,
    })
}

/// The (ink hue, paper hue, bright) that minimises the cell's total colour error — the
/// sum over its 64 pixels of the distance to whichever of the two chosen colours is
/// nearer. `bright` is shared (a Spectrum cell can't mix bright and non-bright), so all
/// three are searched together: 2 × 8 × 8 candidate attributes per cell.
fn choose_attr(cell: &[[u8; 3]; 64]) -> (usize, usize, usize) {
    let mut best = (7usize, 0usize, 0usize); // white ink on black paper
    let mut best_err = u64::MAX;
    for bright in 0..2usize {
        let base = bright * 8;
        for ink in 0..8usize {
            for paper in ink..8usize {
                let ink_rgb = AUTHENTIC[base + ink];
                let paper_rgb = AUTHENTIC[base + paper];
                let mut err = 0u64;
                for &p in cell {
                    err += dist2(p, ink_rgb).min(dist2(p, paper_rgb)) as u64;
                }
                if err < best_err {
                    best_err = err;
                    best = (ink, paper, bright);
                }
            }
        }
    }
    best
}

/// Nearest of the 16 logical colours to an RGB pixel (squared Euclidean distance).
fn nearest(p: [u8; 3]) -> u8 {
    let mut best = 0u8;
    let mut best_d = u32::MAX;
    for (i, &c) in AUTHENTIC.iter().enumerate() {
        let d = dist2(p, c);
        if d < best_d {
            best_d = d;
            best = i as u8;
        }
    }
    best
}

/// Squared Euclidean distance between two RGB colours.
fn dist2(a: [u8; 3], b: [u8; 3]) -> u32 {
    let dr = a[0] as i32 - b[0] as i32;
    let dg = a[1] as i32 - b[1] as i32;
    let db = a[2] as i32 - b[2] as i32;
    (dr * dr + dg * dg + db * db) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `w×h` solid-colour buffer.
    fn solid(c: [u8; 3]) -> Vec<[u8; 3]> {
        vec![c; SCREEN_W * SCREEN_H]
    }

    #[test]
    fn wrong_size_is_an_error() {
        assert!(convert(&[[0, 0, 0]], 8, 8).is_err());
        assert!(convert(&solid([0, 0, 0]), SCREEN_W, SCREEN_H).is_ok());
    }

    #[test]
    fn solid_image_has_no_clash_and_one_colour_per_cell() {
        // Bright green everywhere → no cell ever needs more than one colour.
        let img = convert(&solid([0x00, 0xFF, 0x00]), SCREEN_W, SCREEN_H).unwrap();
        assert!(img.clashes.is_empty(), "a solid image cannot clash");
        // Every cell renders bright green — green (hue 4) is the ink or the paper, with
        // bright set. (Which of the two slots a solid colour lands in is arbitrary; both
        // render identically.)
        let attr = img.attrs[0];
        let (ink, paper) = (attr & 0x07, (attr >> 3) & 0x07);
        assert!(ink == 4 || paper == 4, "green is one of the two cell colours");
        assert_eq!(attr & 0x40, 0x40, "bright bit set");
        assert!(img.attrs.iter().all(|&a| a == attr), "uniform image, uniform attrs");
    }

    #[test]
    fn two_colours_per_cell_never_clash() {
        // Left half of every cell red, right half white — exactly two colours.
        let mut buf = solid([0, 0, 0]);
        for y in 0..SCREEN_H {
            for x in 0..SCREEN_W {
                buf[y * SCREEN_W + x] = if x % 8 < 4 {
                    [0xD7, 0x00, 0x00] // red
                } else {
                    [0xD7, 0xD7, 0xD7] // white
                };
            }
        }
        let img = convert(&buf, SCREEN_W, SCREEN_H).unwrap();
        assert!(img.clashes.is_empty(), "two colours fit a cell with no clash");
        // The left 4 pixels and right 4 pixels split into the two colours → 0xF0.
        assert_eq!(img.pixels[screen_byte_index(0, 0)], 0xF0);
    }

    #[test]
    fn three_colours_in_a_cell_report_a_clash() {
        // Cell (0,0) uses red, green and blue; the rest is black.
        let mut buf = solid([0, 0, 0]);
        let cols = [[0xD7, 0, 0], [0, 0xD7, 0], [0, 0, 0xD7]];
        for c in 0..8 {
            buf[c] = cols[c % 3]; // row 0, cols 0..8 → all in cell (0,0)
        }
        let img = convert(&buf, SCREEN_W, SCREEN_H).unwrap();
        let clash = img
            .clashes
            .iter()
            .find(|c| c.cx == 0 && c.cy == 0)
            .expect("cell (0,0) clashes");
        assert!(clash.colours >= 3, "at least three distinct colours wanted");
        // Only that one cell clashes.
        assert_eq!(img.clashes.len(), 1);
    }

    #[test]
    fn to_scr_is_a_6912_byte_screen() {
        let img = convert(&solid([0, 0, 0]), SCREEN_W, SCREEN_H).unwrap();
        assert_eq!(img.to_scr().len(), PIXELS + ATTRS);
        assert_eq!(img.to_scr().len(), 6912);
    }

    #[test]
    fn nearest_recovers_each_non_black_palette_entry() {
        // Index 8 is bright-black == black, so it maps to 0; every other entry is unique.
        for i in 1..16u8 {
            if i == 8 {
                continue;
            }
            assert_eq!(nearest(AUTHENTIC[i as usize]), i, "palette[{i}] round-trips");
        }
    }
}
