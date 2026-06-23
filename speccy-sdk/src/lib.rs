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

use spectrum::host::{HostCalls, HostCtx};
use spectrum::keyboard;
pub use spectrum::Spectrum;

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

/// Install `game` as the host's `GAME_TICK` handler with the default [`Controls`].
/// Pair with [`load_runtime`].
pub fn install(spec: &mut Spectrum, game: impl Game + Send + 'static) {
    install_with_controls(spec, game, Controls::default());
}

/// Install `game` with a custom key mapping (e.g. WASD). Pair with [`load_runtime`].
pub fn install_with_controls(
    spec: &mut Spectrum,
    game: impl Game + Send + 'static,
    controls: Controls,
) {
    spec.set_host_dispatcher(Box::new(Dispatcher::new(game, controls)));
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

/// What an agent observes each step (spec 08 §3). `Screen` = the framebuffer;
/// typed-feature observations come later (read host-side, or off tape RAM via the
/// compiler-emitted symbol map).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Obs {
    Screen,
}

/// A game: pure logic + rendering, called once per 50 Hz frame.
///
/// The **env surface** — `observe`/`reward`/`done`/`reset` — has defaults so every
/// existing game compiles unchanged; override to instrument for agents (spec 08
/// §3). `reward`/`done`/`observe` must be **pure functions of `(self, prev)`**:
/// they run env-side (host, or over a `Self` reconstructed from tape RAM via the
/// symbol map), never inside the pure tape.
pub trait Game {
    fn update(&mut self, input: &Input, frame: &mut Frame);

    /// What to observe this step. Default: the screen.
    fn observe(&self) -> Obs {
        Obs::Screen
    }

    /// Reward for the transition `prev -> self`. Default: none.
    fn reward(&self, prev: &Self) -> i16
    where
        Self: Sized,
    {
        let _ = prev;
        0
    }

    /// Has the episode terminated? Default: never.
    fn done(&self) -> bool {
        false
    }

    /// Start a fresh episode from `seed` (the episode boundary; seeds the [`Rng`]).
    /// Defaults to `Self::default()` so games that derive `Default` need not
    /// implement it — override to actually use the seed.
    fn reset(seed: u64) -> Self
    where
        Self: Sized + Default,
    {
        let _ = seed;
        Self::default()
    }
}

/// A grid point (cell or pixel coordinate) — a small, `Copy` element type for
/// [`Entities`] and grid games.
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

/// A small deterministic PRNG — **seed it from game state, never the clock** (spec
/// 08 §1: the determinism contract, as a type). xorshift32; seeding the env from a
/// known value makes every episode reproducible.
#[derive(Copy, Clone)]
pub struct Rng {
    state: u32,
}

impl Default for Rng {
    fn default() -> Self {
        Rng::seed(0)
    }
}

impl Rng {
    /// Seed the generator. Zero is mapped to a fixed non-zero constant (xorshift
    /// must never have a zero state).
    pub fn seed(seed: u32) -> Self {
        Rng {
            state: if seed == 0 { 0x9E37_79B9 } else { seed },
        }
    }

    /// Next 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// A value in `[0, n)` (`n` must be non-zero).
    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }
}

/// A fixed-capacity, allocation-free vec — the subset-clean replacement for `Vec`
/// (spec 08 §1). Holds up to `N` `T`s inline; `push`/`insert_front` past capacity
/// are dropped and report `false`. `T: Copy + Default`.
#[derive(Copy, Clone)]
pub struct Entities<T: Copy + Default, const N: usize> {
    items: [T; N],
    len: usize,
}

impl<T: Copy + Default, const N: usize> Default for Entities<T, N> {
    fn default() -> Self {
        Entities {
            items: [T::default(); N],
            len: 0,
        }
    }
}

impl<T: Copy + Default, const N: usize> Entities<T, N> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn capacity(&self) -> usize {
        N
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len]
    }
    pub fn get(&self, i: usize) -> Option<T> {
        if i < self.len {
            Some(self.items[i])
        } else {
            None
        }
    }
    /// Append to the back. Returns `false` if full.
    pub fn push(&mut self, v: T) -> bool {
        if self.len < N {
            self.items[self.len] = v;
            self.len += 1;
            true
        } else {
            false
        }
    }
    /// Remove and return the back element.
    pub fn pop(&mut self) -> Option<T> {
        if self.len > 0 {
            self.len -= 1;
            Some(self.items[self.len])
        } else {
            None
        }
    }
    /// Insert at the front, shifting the rest right (drops the last if full).
    pub fn insert_front(&mut self, v: T) -> bool {
        if N == 0 {
            return false;
        }
        let top = self.len.min(N - 1);
        let mut i = top;
        while i > 0 {
            self.items[i] = self.items[i - 1];
            i -= 1;
        }
        self.items[0] = v;
        self.len = (self.len + 1).min(N);
        true
    }
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }
    pub fn contains(&self, v: &T) -> bool
    where
        T: PartialEq,
    {
        self.as_slice().contains(v)
    }
}

impl<T: Copy + Default, const N: usize> core::ops::Index<usize> for Entities<T, N> {
    type Output = T;
    fn index(&self, i: usize) -> &T {
        &self.items[i]
    }
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
    /// No input — for tests / headless stepping.
    pub fn none() -> Self {
        Input { cur: 0, prev: 0 }
    }
    /// Construct from a set of currently-held buttons (for testing a `Game`).
    pub fn held_now(buttons: &[Button]) -> Self {
        let mut cur = 0u8;
        for &b in buttons {
            cur |= b as u8;
        }
        Input { cur, prev: 0 }
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

// --- controls: the one place key bindings live -----------------------------

/// Maps each logical [`Button`] to the physical key(s) that trigger it — the
/// single source of truth for input, shared by the host head ([`install`]) and
/// reusable by the agent env. Remappable: build a custom scheme with
/// [`Controls::set`] / [`Controls::bind`] and pass it to [`install_with_controls`].
/// The default is cursor keys **and** QAOP + `0`/Space, so any common scheme works.
#[derive(Clone)]
pub struct Controls {
    bindings: Vec<(Button, char)>,
}

impl Default for Controls {
    fn default() -> Self {
        // Cursor key listed first per button, so `key_pos` prefers it.
        Controls {
            bindings: vec![
                (Button::Up, '7'),
                (Button::Up, 'q'),
                (Button::Down, '6'),
                (Button::Down, 'a'),
                (Button::Left, '5'),
                (Button::Left, 'o'),
                (Button::Right, '8'),
                (Button::Right, 'p'),
                (Button::Fire, '0'),
                (Button::Fire, ' '),
            ],
        }
    }
}

impl Controls {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a key for `button` (additive).
    pub fn bind(&mut self, button: Button, key: char) -> &mut Self {
        self.bindings.push((button, key));
        self
    }

    /// Replace all keys for `button` with `keys`.
    pub fn set(&mut self, button: Button, keys: &[char]) -> &mut Self {
        self.bindings.retain(|(b, _)| *b != button);
        for &k in keys {
            self.bindings.push((button, k));
        }
        self
    }

    /// The keys currently bound to `button`.
    pub fn keys_for(&self, button: Button) -> impl Iterator<Item = char> + '_ {
        self.bindings
            .iter()
            .filter(move |(b, _)| *b == button)
            .map(|(_, k)| *k)
    }

    /// The primary physical key for `button` (first binding) — the one the env
    /// presses to drive the button.
    pub fn key_pos(&self, button: Button) -> Option<keyboard::KeyPos> {
        self.bindings
            .iter()
            .find(|(b, _)| *b == button)
            .and_then(|(_, ch)| keyboard::key_for_char(*ch).map(|(p, _, _)| p))
    }

    /// Read the held-button bitset from the live keyboard (host side).
    fn read(&self, ctx: &HostCtx) -> u8 {
        let mut b = 0u8;
        for &(button, ch) in &self.bindings {
            if let Some((pos, _, _)) = keyboard::key_for_char(ch) {
                if ctx.key(pos) {
                    b |= button as u8;
                }
            }
        }
        b
    }
}

// --- the GAME_TICK dispatcher -----------------------------------------------

struct Dispatcher<G> {
    game: G,
    frame: Frame,
    controls: Controls,
    prev: u8,
    font_loaded: bool,
}

impl<G: Game> Dispatcher<G> {
    fn new(game: G, controls: Controls) -> Self {
        Dispatcher {
            game,
            frame: Frame::new(),
            controls,
            prev: 0,
            font_loaded: false,
        }
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
            self.frame
                .font
                .copy_from_slice(&ctx.read(0x3D00, ATTRS as u16));
            self.font_loaded = true;
        }
        let cur = self.controls.read(ctx);
        let input = Input {
            cur,
            prev: self.prev,
        };
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
        let down = Input {
            cur: Button::Fire as u8,
            prev: 0,
        };
        assert!(down.held(Button::Fire) && down.pressed(Button::Fire));
        let still = Input {
            cur: Button::Fire as u8,
            prev: Button::Fire as u8,
        };
        assert!(still.held(Button::Fire) && !still.pressed(Button::Fire));
    }

    #[test]
    fn rng_is_deterministic_and_seedable() {
        let mut a = Rng::seed(12345);
        let mut b = Rng::seed(12345);
        let seq_a: Vec<u32> = (0..8).map(|_| a.next_u32()).collect();
        let seq_b: Vec<u32> = (0..8).map(|_| b.next_u32()).collect();
        assert_eq!(seq_a, seq_b, "same seed → same sequence");
        let mut c = Rng::seed(54321);
        assert_ne!(c.next_u32(), seq_a[0], "different seed → different stream");
        let mut d = Rng::seed(7);
        for _ in 0..100 {
            assert!(d.below(6) < 6, "below(n) stays in range");
        }
    }

    #[test]
    fn entities_is_a_fixed_capacity_vec() {
        let mut e: Entities<u16, 4> = Entities::new();
        assert!(e.is_empty() && e.capacity() == 4);
        assert!(e.push(10) && e.push(20) && e.push(30));
        assert_eq!(e.len(), 3);
        assert_eq!(e[0], 10);
        assert!(e.contains(&20) && !e.contains(&99));
        assert_eq!(e.pop(), Some(30));

        // insert_front shifts right; over-capacity drops the last.
        e.insert_front(5); // [5, 10, 20]
        assert_eq!(e[0], 5);
        assert_eq!(e.as_slice(), &[5, 10, 20]);
        assert!(e.push(40)); // [5,10,20,40] full
        assert!(!e.push(50), "push past capacity reports false");
        assert_eq!(e.len(), 4);
    }
}
