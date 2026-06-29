//! Demo games built on [`speccy_sdk`] — content that *consumes* the SDK, kept out
//! of the library crate. Each is a [`speccy_sdk::Game`]; a head installs one and
//! pumps frames (`speccy-gui <rom> <name>`). They double as manual test ROMs:
//! `keytest` (input), `typing` (font), `mover` (controls + remap), `snake` (a game).

use speccy_sdk::{
    Button, Cell, Colour, Controls, Entities, Frame, Game, Input, Rng, Spectrum, Tile, BLOCK,
};

/// Demo names — the single source of truth (no magic strings at the call site).
pub const SNAKE: &str = "snake";
pub const KEYTEST: &str = "keytest";
pub const TYPING: &str = "typing";
pub const MOVER: &str = "mover";
pub const SPRITES: &str = "sprites";

const W: u8 = 32; // playfield is the full 32×24 grid; row 0 shows the score
const TOP: u8 = 1;
const BOTTOM: u8 = 24;
const MAX: usize = 768; // 32 × 24 cells — the playfield ceiling, so the body never overflows
const DEFAULT_SEED: u32 = 0x1234_5678;

/// An empty 8×8 tile — clears a cell (used to erase a sprite's old position).
const ERASE: Tile = Tile { rows: [0; 8] };

// --- baked art (the asset pipeline, end to end) ----------------------------
// `SHOWCASE` was *baked from a PNG* by `chuk-speccy-assets`, not hand-typed:
//   speccy-asset bake sprites.png --name SHOWCASE -o sprites.rs
// A 24×8 source image → three 8×8 `Tile`s (face · heart · star). This is the
// authoring loop's payoff: art → `const Tile` → drawn with `frame.tile(..)`.
// (`Frame::tile` is host-only — const bitmap relocation is a cell80 feature — so
// baked tiles live in these host demos, not the pure dual-compile `samples/`.)
const SHOWCASE: [Tile; 3] = [
    // (0,0) ink BrightCyan — a smiley
    Tile {
        rows: [
            0b00111100, 0b01000010, 0b10100101, 0b10000001, 0b10100101, 0b10011001, 0b01000010,
            0b00111100,
        ],
    },
    // (1,0) ink BrightRed — a heart
    Tile {
        rows: [
            0b00000000, 0b01100110, 0b11111111, 0b11111111, 0b11111111, 0b01111110, 0b00111100,
            0b00011000,
        ],
    },
    // (2,0) ink BrightYellow — a star
    Tile {
        rows: [
            0b00011000, 0b00011000, 0b11111111, 0b01111110, 0b00111100, 0b01100110, 0b11000011,
            0b00000000,
        ],
    },
];

/// A grid Snake (`speccy-gui <rom> snake`). State is fully deterministic (RNG seeded
/// from state, frames counted) so it rewinds/replays/RLs correctly — the substrate
/// contract. Built on the subset-clean primitives ([`Entities`], [`Rng`], [`Cell`])
/// and exposing the env surface (`reset`/`reward`/`done`).
#[derive(Clone)]
pub struct Snake {
    body: Entities<Cell, MAX>,
    dir: Button,
    food: Cell,
    rng: Rng,
    tick: u8,
    alive: bool,
    score: u16,
}

impl Default for Snake {
    fn default() -> Self {
        Snake::seeded(DEFAULT_SEED)
    }
}

impl Snake {
    pub fn new() -> Self {
        Self::default()
    }

    fn seeded(seed: u32) -> Self {
        let mut body = Entities::new();
        body.push(Cell::new(8, 12));
        body.push(Cell::new(7, 12));
        body.push(Cell::new(6, 12));
        let mut s = Snake {
            body,
            dir: Button::Right,
            food: Cell::new(0, 0),
            rng: Rng::seed(seed),
            tick: 0,
            alive: true,
            score: 0,
        };
        s.food = s.spawn();
        s
    }

    fn spawn(&mut self) -> Cell {
        loop {
            // Power-of-two masks keep this subset-clean (no u32 `%`): draw a full
            // 0..32 column/row and reject rows outside the playfield — the loop
            // already rejects body hits. W is 32, so x is always in range.
            let x = self.rng.below_mask(31) as u8; // 0..32 == 0..W
            let y = self.rng.below_mask(31) as u8; // 0..32, rejected to [TOP, BOTTOM)
            let c = Cell::new(x, y);
            if (TOP..BOTTOM).contains(&y) && !self.body.contains(&c) {
                return c;
            }
        }
    }
}

impl Game for Snake {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        // Steer (no reversing onto yourself).
        if input.held(Button::Up) && self.dir != Button::Down {
            self.dir = Button::Up;
        } else if input.held(Button::Down) && self.dir != Button::Up {
            self.dir = Button::Down;
        } else if input.held(Button::Left) && self.dir != Button::Right {
            self.dir = Button::Left;
        } else if input.held(Button::Right) && self.dir != Button::Left {
            self.dir = Button::Right;
        }

        if self.alive {
            self.tick += 1;
        }
        if self.alive && self.tick >= 6 {
            self.tick = 0;
            let h = self.body[0];
            let head = match self.dir {
                Button::Up => Cell::new(h.x, h.y.wrapping_sub(1)),
                Button::Down => Cell::new(h.x, h.y + 1),
                Button::Left => Cell::new(h.x.wrapping_sub(1), h.y),
                _ => Cell::new(h.x + 1, h.y),
            };
            if head.x >= W || head.y < TOP || head.y >= BOTTOM || self.body.contains(&head) {
                self.alive = false;
            } else {
                self.body.insert_front(head);
                if head == self.food {
                    self.score += 1;
                    self.food = self.spawn();
                } else {
                    self.body.pop();
                }
            }
        }
        if !self.alive && input.pressed(Button::Fire) {
            *self = Snake::seeded(DEFAULT_SEED);
        }

        frame.clear(Colour::Black);
        // Solid-cell sprites (data-free, colour by value) — the subset-clean draw
        // primitive, so the gameplay render is pure-tape shaped.
        for i in 0..self.body.len() {
            let c = self.body[i];
            frame.fill_cell(c.x, c.y, Colour::BrightGreen);
        }
        frame.fill_cell(self.food.x, self.food.y, Colour::BrightRed);
        frame.ink(Colour::White);
        frame.text(0, 0, "SNAKE   LEN");
        frame.text_u16(12, 0, self.body.len() as u16);
        if !self.alive {
            frame.ink(Colour::BrightYellow);
            frame.text(8, 12, "GAME OVER - FIRE");
        }
    }

    /// Episode boundary: a fresh snake seeded from `seed`.
    fn reset(seed: u64) -> Self {
        Snake::seeded(seed as u32)
    }

    /// Score gained this step (one per food eaten) — the dense reward.
    fn reward(&self, prev: &Self) -> i16 {
        self.score as i16 - prev.score as i16
    }

    /// The episode ends when the snake dies.
    fn done(&self) -> bool {
        !self.alive
    }
}

/// **Input visualiser** (`speccy-gui <rom> keytest`): lights up each logical button
/// while it's held — a manual check that the controls (and any remap) reach the game.
#[derive(Default)]
pub struct KeyTest;

impl Game for KeyTest {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        frame.clear(Colour::Blue);
        frame.ink(Colour::BrightWhite);
        frame.text(10, 2, "INPUT TEST");
        frame.ink(Colour::Cyan);
        frame.text(4, 4, "hold a direction or fire");

        let rows = [
            ("UP", Button::Up),
            ("DOWN", Button::Down),
            ("LEFT", Button::Left),
            ("RIGHT", Button::Right),
            ("FIRE", Button::Fire),
        ];
        for (i, (label, b)) in rows.iter().enumerate() {
            let y = 8 + i as u8 * 2;
            let held = input.held(*b);
            frame.ink(if held {
                Colour::BrightYellow
            } else {
                Colour::White
            });
            frame.text(8, y, label);
            if held {
                frame.ink(Colour::BrightGreen);
                frame.text(16, y, "<== HELD");
            }
        }
    }
}

/// **Typing test** (`speccy-gui <rom> typing`): types out the whole printable
/// character set and sweeps a cursor through it — a manual check that every ROM font
/// glyph renders.
#[derive(Default)]
pub struct Typing {
    pos: u16, // 0..94, the currently-highlighted char index
    tick: u8,
}

impl Game for Typing {
    fn update(&mut self, _input: &Input, frame: &mut Frame) {
        self.tick = self.tick.wrapping_add(1);
        if self.tick >= 4 {
            self.tick = 0;
            self.pos += 1;
            if self.pos > 94 {
                self.pos = 0;
            }
        }

        frame.clear(Colour::Black);
        frame.ink(Colour::BrightCyan);
        frame.text(10, 1, "TYPING TEST");

        // The 95 printable chars (32..=126) in three rows of 32.
        frame.ink(Colour::White);
        for r in 0..3u8 {
            let mut buf = [b' '; 32];
            let mut n = 0usize;
            while n < 32 {
                let code = 32u16 + r as u16 * 32 + n as u16;
                if code <= 126 {
                    buf[n] = code as u8;
                }
                n += 1;
            }
            if let Ok(s) = core::str::from_utf8(&buf) {
                frame.text(0, 5 + r * 2, s);
            }
        }

        // The char the cursor is currently on.
        let cur = 32u8 + self.pos as u8;
        frame.ink(Colour::BrightYellow);
        frame.text(8, 14, "NOW:");
        let one = [cur];
        if let Ok(s) = core::str::from_utf8(&one) {
            frame.text(13, 14, s);
        }
        frame.text(15, 14, "= CODE");
        frame.text_u16(22, 14, cur as u16);
    }
}

/// **Mover** (`speccy-gui <rom> mover`): move a blob with the controls — installed
/// with a *remapped* `Controls` (WASD) to demo "redefine keys". Shows the classic
/// erase-old → move → draw-new sprite loop (a pattern destined for the kit).
pub struct Mover {
    x: u8,
    y: u8,
    started: bool,
}

impl Default for Mover {
    fn default() -> Self {
        Mover {
            x: 16,
            y: 12,
            started: false,
        }
    }
}

impl Game for Mover {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if !self.started {
            frame.clear(Colour::Black);
            self.started = true;
        }

        // Erase the blob at its old cell.
        frame.ink(Colour::Black);
        frame.tile(&ERASE, self.x, self.y);

        // Move (clamped to keep clear of the 2-row HUD).
        if input.held(Button::Left) && self.x > 0 {
            self.x -= 1;
        }
        if input.held(Button::Right) && self.x < 31 {
            self.x += 1;
        }
        if input.held(Button::Up) && self.y > 3 {
            self.y -= 1;
        }
        if input.held(Button::Down) && self.y < 23 {
            self.y += 1;
        }

        // Draw the blob at its new cell.
        frame.ink(Colour::BrightMagenta);
        frame.tile(&BLOCK, self.x, self.y);

        // HUD.
        frame.ink(Colour::BrightCyan);
        frame.text(1, 0, "MOVER  (WASD)");
        frame.ink(Colour::White);
        frame.text(16, 0, "X");
        frame.text_u16(18, 0, self.x as u16);
        frame.text(22, 0, "Y");
        frame.text_u16(24, 0, self.y as u16);
    }
}

/// **Sprites** (`speccy-gui <rom> sprites`): a showcase for the **asset pipeline** —
/// every sprite here was baked from a PNG into a [`SHOWCASE`] `const Tile` by
/// `chuk-speccy-assets`. A row of the three baked tiles cycles colour (so the GIF is
/// lively with no input), and a movable face cursor (the first baked tile) responds to
/// the controls. Pure host-composite: `frame.tile` draws baked bitmaps.
pub struct Sprites {
    tick: u16,
    x: u8,
    y: u8,
}

impl Default for Sprites {
    fn default() -> Self {
        Sprites {
            tick: 0,
            x: 16,
            y: 14,
        }
    }
}

impl Game for Sprites {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        frame.clear(Colour::Black); // full redraw each tick — simplest, demo-clear

        // The three baked tiles, side by side, cycling through the bright palette so
        // the showcase shimmers without any input driving it.
        const CYCLE: [Colour; 6] = [
            Colour::BrightCyan,
            Colour::BrightGreen,
            Colour::BrightYellow,
            Colour::BrightMagenta,
            Colour::BrightRed,
            Colour::BrightWhite,
        ];
        let phase = (self.tick / 8) as usize;
        for (i, tile) in SHOWCASE.iter().enumerate() {
            frame
                .ink(CYCLE[(phase + i) % CYCLE.len()])
                .tile(tile, 13 + i as u8 * 2, 6);
        }

        // A movable face cursor (the first baked tile), nudged by the controls.
        if input.held(Button::Left) && self.x > 0 {
            self.x -= 1;
        }
        if input.held(Button::Right) && self.x < 31 {
            self.x += 1;
        }
        if input.held(Button::Up) && self.y > 3 {
            self.y -= 1;
        }
        if input.held(Button::Down) && self.y < 23 {
            self.y += 1;
        }
        frame
            .ink(Colour::BrightWhite)
            .tile(&SHOWCASE[0], self.x, self.y);

        // HUD — names the trick.
        frame.ink(Colour::BrightCyan);
        frame.text(1, 0, "BAKED SPRITES");
        frame.ink(Colour::White);
        frame.text(1, 1, "PNG -> const Tile");

        self.tick = self.tick.wrapping_add(1);
    }
}

// --- the registry: name → installer (heads stay game-agnostic) -------------

/// One installable demo: its name, a one-line description, and how to install it on
/// a machine (including any custom [`Controls`]). A head just picks a name.
pub struct Demo {
    pub name: &'static str,
    pub about: &'static str,
    install: fn(&mut Spectrum),
}

fn install_snake(s: &mut Spectrum) {
    speccy_sdk::install(s, Snake::new());
}
fn install_keytest(s: &mut Spectrum) {
    speccy_sdk::install(s, KeyTest);
}
fn install_typing(s: &mut Spectrum) {
    speccy_sdk::install(s, Typing::default());
}
fn install_mover(s: &mut Spectrum) {
    // "Redefine keys" — the mover runs on a remapped WASD scheme.
    let mut c = Controls::new();
    c.set(Button::Up, &['w'])
        .set(Button::Down, &['s'])
        .set(Button::Left, &['a'])
        .set(Button::Right, &['d']);
    speccy_sdk::install_with_controls(s, Mover::default(), c);
}
fn install_sprites(s: &mut Spectrum) {
    speccy_sdk::install(s, Sprites::default());
}

/// Every installable demo — the registry. Add a game here and every head picks it up.
pub const DEMOS: &[Demo] = &[
    Demo {
        name: SNAKE,
        about: "a grid snake",
        install: install_snake,
    },
    Demo {
        name: KEYTEST,
        about: "input visualiser",
        install: install_keytest,
    },
    Demo {
        name: TYPING,
        about: "font / typing test",
        install: install_typing,
    },
    Demo {
        name: MOVER,
        about: "move a blob (WASD — remapped controls)",
        install: install_mover,
    },
    Demo {
        name: SPRITES,
        about: "baked-art showcase (PNG -> const Tile)",
        install: install_sprites,
    },
];

/// Is `name` a known demo? (For arg parsing, before the machine exists.)
pub fn is_demo(name: &str) -> bool {
    DEMOS.iter().any(|d| d.name == name)
}

/// Install the named demo on `spec` (with its own controls). Returns `false` if the
/// name is unknown. Pair with `speccy_sdk::load_runtime`.
pub fn install(spec: &mut Spectrum, name: &str) -> bool {
    match DEMOS.iter().find(|d| d.name == name) {
        Some(d) => {
            (d.install)(spec);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_from_seed() {
        // Same seed → identical initial state (the determinism contract).
        let a = Snake::seeded(42);
        let b = Snake::seeded(42);
        assert_eq!(a.food, b.food, "same seed → same food placement");
    }

    #[test]
    fn eating_grows_and_rewards() {
        let mut s = Snake::seeded(7);
        let before = s.clone();
        // Drive the head onto the food so it eats this tick.
        s.body.clear();
        s.body.push(Cell::new(s.food.x.wrapping_sub(1), s.food.y));
        s.dir = Button::Right;
        s.tick = 5; // next update steps the snake
        let input = Input::none();
        let mut frame = Frame::new();
        s.update(&input, &mut frame);
        assert_eq!(s.score, 1, "ate the food");
        assert_eq!(s.reward(&before), 1, "reward = score delta");
        assert!(!s.done());
    }

    #[test]
    fn body_uses_fixed_capacity() {
        let s = Snake::new();
        assert_eq!(s.body.len(), 3);
        assert_eq!(s.body.capacity(), MAX);
    }

    #[test]
    fn registry_names_are_unique() {
        let mut names: Vec<&str> = DEMOS.iter().map(|d| d.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), DEMOS.len(), "no duplicate demo names");
        assert!(is_demo(SNAKE) && !is_demo("nope"));
    }
}
