//! Native Rust game SDK — write a game in Rust; it runs on the Spectrum
//! substrate over the `ED FE` trap ABI (`docs/03-sdk-spec.md`, host-composite
//! backend). The emulated Z80 is a ~11-byte frame pump: it syncs to the 50 Hz
//! interrupt and traps to the host (`GAME_TICK`), which runs your
//! [`Game::update`] and writes the screen. Everything — input, logic, rendering
//! — is host Rust, so the result still *is* a Spectrum (themes, CRT, screenshots,
//! MCP, snapshots, RL all apply) rather than a window with a retro filter.
//!
//! Author one [`Game`]; install it on a [`spectrum::Spectrum`] with
//! [`install`] + [`load_runtime`], then spin any head (the native window, TUI,
//! headless over MCP, …). The game's logic must be a pure function of
//! `(state, input)` — seed RNG from state, count frames, no host I/O — so rewind,
//! replay and RL stay correct.

pub mod demo;

use spectrum::host::{HostCalls, HostCtx};
use spectrum::keyboard;
use spectrum::Spectrum;

/// The per-frame host syscall id (`docs/03` id map, `0x60` = game).
pub const GAME_TICK: u8 = 0x60;

/// Where the runtime pump is loaded / entered.
pub const RUNTIME_ORG: u16 = 0x8000;

/// The entire Z80 guest program (§1): `di; im 1; ei; loop: halt; ld a,0x60;
/// HOSTCALL; jr loop`. It contributes the authentic frame clock + display + I/O
/// model; the host does the rest.
pub const RUNTIME: [u8; 11] = [
    0xF3, // di
    0xED, 0x56, // im 1
    0xFB, // ei
    0x76, // loop: halt
    0x3E, 0x60, // ld a, GAME_TICK
    0xED, 0xFE, // HOSTCALL
    0x18, 0xF9, // jr loop
];

/// Load the runtime pump and point the CPU at it. Boot the ROM first (the runtime
/// relies on the ROM's IM 1 interrupt handler for the frame sync).
pub fn load_runtime(spec: &mut Spectrum) {
    spec.write_memory(RUNTIME_ORG, &RUNTIME);
    spec.cpu.regs.pc = RUNTIME_ORG;
}

/// Install `game` as the host's `GAME_TICK` handler. Pair with [`load_runtime`].
pub fn install(spec: &mut Spectrum, game: impl Game + Send + 'static) {
    spec.set_host_dispatcher(Box::new(Dispatcher::new(game)));
}

/// Convenience: boot the ROM, load the runtime, and install `game` — ready for a
/// head to step. (`rom` is the 16K system ROM.)
pub fn boot(rom: &[u8], game: impl Game + Send + 'static) -> Spectrum {
    let mut spec = Spectrum::new_48k(rom);
    for _ in 0..200 {
        spec.run_frame(); // bring up the ROM (IM 1 handler + system vars)
    }
    install(&mut spec, game);
    load_runtime(&mut spec);
    spec
}

// --- author API -------------------------------------------------------------

/// A game: pure logic + rendering, called once per 50 Hz frame.
pub trait Game {
    fn update(&mut self, input: &Input, frame: &mut Frame);
}

/// Logical buttons (keyboard or joystick, pre-mapped). Bit values double as a
/// small bitset used internally.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Button {
    Up = 1,
    Down = 2,
    Left = 4,
    Right = 8,
    Fire = 16,
}

/// This frame's input: which buttons are held, and which became held this frame.
pub struct Input {
    cur: u8,
    prev: u8,
}

impl Input {
    /// Held right now.
    pub fn held(&self, b: Button) -> bool {
        self.cur & b as u8 != 0
    }
    /// Newly pressed this frame (rising edge).
    pub fn pressed(&self, b: Button) -> bool {
        self.cur & b as u8 != 0 && self.prev & b as u8 == 0
    }
}

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

/// An 8×8 cell-aligned tile (one byte per pixel row, bit 7 = leftmost).
#[derive(Copy, Clone)]
pub struct Tile {
    pub rows: [u8; 8],
}

/// A solid 8×8 block — handy for grid games.
pub const BLOCK: Tile = Tile { rows: [0xFF; 8] };

const PIXELS: usize = 6144; // 0x4000..0x5800
const ATTRS: usize = 768; // 0x5800..0x5B00

/// A frame to draw into: 256×192 1-bit pixels + 32×24 attributes, in the
/// Spectrum's interleaved screen layout (so handing it to the machine is a
/// straight copy). Reused across frames; call [`Frame::clear`] each tick.
pub struct Frame {
    pixels: [u8; PIXELS],
    attrs: [u8; ATTRS],
    ink: Colour,
    paper: Colour,
    font: [u8; ATTRS], // 96 glyphs × 8 bytes, lifted from the ROM
}

impl Frame {
    fn new() -> Self {
        Frame {
            pixels: [0; PIXELS],
            attrs: [0; ATTRS],
            ink: Colour::White,
            paper: Colour::Black,
            font: [0; ATTRS],
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
}

/// Pixel byte index + bit mask for `(x, y)` in the interleaved screen layout.
fn pixel_at(x: usize, y: usize) -> (usize, u8) {
    (byte_index(x, y), 0x80 >> (x & 7))
}

/// Byte index into the 6144-byte pixel area for the byte containing pixel `(x,y)`.
/// The ZX layout interleaves: third of screen, then pixel-row, then char-row.
fn byte_index(x: usize, y: usize) -> usize {
    (y / 64) * 2048 + (y % 8) * 256 + ((y % 64) / 8) * 32 + x / 8
}

// --- the GAME_TICK dispatcher -----------------------------------------------

/// Button → key map: cursor keys *and* QAOP+Space, so any common scheme works.
const KEYMAP: &[(Button, char)] = &[
    (Button::Up, '7'),
    (Button::Up, 'q'),
    (Button::Down, '6'),
    (Button::Down, 'a'),
    (Button::Left, '5'),
    (Button::Left, 'o'),
    (Button::Right, '8'),
    (Button::Right, 'p'),
    (Button::Fire, '0'),
];

struct Dispatcher<G> {
    game: G,
    frame: Frame,
    prev: u8,
    font_loaded: bool,
}

impl<G: Game> Dispatcher<G> {
    fn new(game: G) -> Self {
        Dispatcher { game, frame: Frame::new(), prev: 0, font_loaded: false }
    }

    fn read_buttons(ctx: &HostCtx) -> u8 {
        let mut b = 0u8;
        for &(button, ch) in KEYMAP {
            if let Some((pos, _, _)) = keyboard::key_for_char(ch) {
                if ctx.key(pos) {
                    b |= button as u8;
                }
            }
        }
        if ctx.key(keyboard::SPACE) {
            b |= Button::Fire as u8;
        }
        b
    }
}

impl<G: Game + Send + 'static> HostCalls for Dispatcher<G> {
    fn dispatch(&mut self, ctx: &mut HostCtx) -> u32 {
        if ctx.id() != GAME_TICK {
            ctx.fail();
            return 0;
        }
        // Lift the 8×8 font from the ROM once (chars 32..127 at $3D00).
        if !self.font_loaded {
            self.frame.font.copy_from_slice(&ctx.read(0x3D00, ATTRS as u16));
            self.font_loaded = true;
        }
        let cur = Self::read_buttons(ctx);
        let input = Input { cur, prev: self.prev };
        self.prev = cur;

        self.game.update(&input, &mut self.frame);

        ctx.write(0x4000, &self.frame.pixels);
        ctx.write(0x5800, &self.frame.attrs);
        ctx.ok();
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn input_edges() {
        let down = Input { cur: Button::Fire as u8, prev: 0 };
        assert!(down.held(Button::Fire) && down.pressed(Button::Fire));
        let still = Input { cur: Button::Fire as u8, prev: Button::Fire as u8 };
        assert!(still.held(Button::Fire) && !still.pressed(Button::Fire));
    }
}
