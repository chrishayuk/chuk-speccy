// Snake as an `impl Game` — the "prove the seam" sample: ONE source, two compilers.
//
//   rustc  (host):  an ordinary `speccy-sdk` Game — see tests/dial.rs, which
//                   `include!`s this file with `use speccy_sdk::*;`.
//   rustz80 (pure): compile it straight to a bootable tape + a `.sym.toml` —
//      cargo run -p chuk-speccy-sdk --features compile --bin speccy-compile -- speccy-sdk/samples/snake_game.rs
//      cargo run --release --bin speccy-gui -- testroms/48.rom snake_game.tap
//
// A grid snake: steer with the keys, eat the red food to grow, wrap at the edges.
// Drawn entirely with the data-free `fill_cell`/`clear_cell` primitives (colour by
// value), so nothing needs a `&Tile` relocated; food placement uses a `u16`
// xorshift RNG (constant shifts + `^`, all subset-clean). The typed state — `len`,
// `food_x`, `bx[32]`, … — is exactly what the emitted `.sym.toml` exposes, so an env
// reads the live game off the tape's RAM.
//
// Deliberately within the dialect's envelope (vs the richer `chuk-speccy-games`
// Snake): no text HUD (font/string-by-address is a cell80 compiler feature), no
// `Entities<Cell>`/`u32` (struct fields are 16-bit slots) — the body is parallel
// `[u16; 32]` arrays — and no self-collision death (it wraps), keeping it an
// always-animating seam proof. No `use`/`fn main`, long-form ops, so it compiles
// both ways.

#[derive(Default)]
struct Snake {
    started: u16,
    len: u16,
    dir: u16, // 0 right, 1 down, 2 left, 3 up
    tick: u16,
    rng: u16,
    food_x: u16,
    food_y: u16,
    bx: [u16; 32],
    by: [u16; 32],
}

impl Game for Snake {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if self.started == 0u16 {
            frame.clear(Colour::Black);
            self.rng = 1u16;
            self.len = 3u16;
            self.dir = 0u16;
            self.bx[0] = 8u16;
            self.by[0] = 12u16;
            self.bx[1] = 7u16;
            self.by[1] = 12u16;
            self.bx[2] = 6u16;
            self.by[2] = 12u16;
            self.food_x = 16u16;
            self.food_y = 10u16;
            self.started = 1u16;
        }

        // Steer (no reversing straight back onto the neck).
        if input.held(Button::Up) {
            if self.dir != 1u16 {
                self.dir = 3u16;
            }
        }
        if input.held(Button::Down) {
            if self.dir != 3u16 {
                self.dir = 1u16;
            }
        }
        if input.held(Button::Left) {
            if self.dir != 0u16 {
                self.dir = 2u16;
            }
        }
        if input.held(Button::Right) {
            if self.dir != 2u16 {
                self.dir = 0u16;
            }
        }

        self.tick = self.tick + 1u16;
        if self.tick >= 4u16 {
            self.tick = 0u16;

            // Next head cell, wrapping at the playfield edges.
            let mut nx = self.bx[0];
            let mut ny = self.by[0];
            if self.dir == 0u16 {
                nx = (nx + 1u16) % 32u16;
            }
            if self.dir == 1u16 {
                ny = (ny + 1u16) % 24u16;
            }
            if self.dir == 2u16 {
                nx = (nx + 31u16) % 32u16;
            }
            if self.dir == 3u16 {
                ny = (ny + 23u16) % 24u16;
            }

            let mut ate = 0u16;
            if nx == self.food_x {
                if ny == self.food_y {
                    ate = 1u16;
                }
            }

            // On eating: respawn food (u16 xorshift) and grow up to the array bound.
            let mut grew = 0u16;
            if ate == 1u16 {
                self.rng = self.rng ^ (self.rng << 7u16);
                self.rng = self.rng ^ (self.rng >> 9u16);
                self.rng = self.rng ^ (self.rng << 8u16);
                self.food_x = self.rng % 32u16;
                self.food_y = (self.rng / 32u16) % 24u16;
                if self.len < 32u16 {
                    self.len = self.len + 1u16;
                    grew = 1u16;
                }
            }

            // Erase the vacated tail cell whenever the body didn't grow this tick.
            if grew == 0u16 {
                let t = self.len - 1u16;
                frame.clear_cell(self.bx[t as usize] as u8, self.by[t as usize] as u8);
            }

            // Shift the body toward the head, then place the new head.
            let mut j = self.len - 1u16;
            while j > 0u16 {
                self.bx[j as usize] = self.bx[(j - 1u16) as usize];
                self.by[j as usize] = self.by[(j - 1u16) as usize];
                j = j - 1u16;
            }
            self.bx[0] = nx;
            self.by[0] = ny;
        }

        // Draw the food and every body segment as solid cells.
        frame.fill_cell(self.food_x as u8, self.food_y as u8, Colour::BrightRed);
        let mut i = 0u16;
        while i < self.len {
            frame.fill_cell(
                self.bx[i as usize] as u8,
                self.by[i as usize] as u8,
                Colour::BrightGreen,
            );
            i = i + 1u16;
        }
    }
}
