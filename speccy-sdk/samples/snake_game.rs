// Snake as an `impl Game` — the "prove the seam" sample, and a real little game: ONE
// source, two compilers.
//
//   host:  an ordinary `speccy-sdk` Game (see tests/dial.rs).
//   pure:  speccy-compile speccy-sdk/samples/snake_game.rs  ->  a bootable .tap
//          speccy-gui testroms/48.rom snake_game.tap
//
// Steer with the cursor keys (or QAOP); eat the red food to grow. Row 0 is a **score
// bar** — one cyan cell per food eaten. Hitting a wall or your own body is game over
// (the head flashes yellow), then it restarts: press Fire = 0 / Space, or it
// auto-restarts after a short pause (which also keeps a no-input run animating for the
// demo/tests). Food never spawns on the snake (it re-rolls onto a free cell).
//
// Drawing is **incremental** — each move erases the vacated tail cell and draws the
// new head cell (O(1)), never the whole body — so speed stays constant as you grow.
// The typed state — `len`, `score`, `dead`, `food_x`, `bx[32]`, … — is what the
// emitted `.sym.toml` exposes, so an env reads (and scores) the live game off RAM.
//
// Within the dialect envelope: no text HUD (font-by-address is a cell80 feature; the
// score is a cell bar instead), no `Entities<Cell>`/`u32` (struct fields are 16-bit
// slots — the body is parallel `[u16; 32]` arrays, the RNG a `u16` xorshift). No
// `use`/`fn main`, long-form ops. The playfield is rows 1..24 (row 0 is the score bar).

#[derive(Default)]
struct Snake {
    started: u16,
    len: u16,
    score: u16, // foods eaten (drives the row-0 score bar)
    dir: u16,   // 0 right, 1 down, 2 left, 3 up
    tick: u16,
    rng: u16,
    food_x: u16,
    food_y: u16,
    dead: u16, // 0 = alive; otherwise a countdown to auto-restart after a crash
    bx: [u16; 32],
    by: [u16; 32],
}

impl Snake {
    // Draw the score as two white digits in cell (0,0) — a 3×5 pixel font via
    // `frame.pixel` (no font-by-address needed). Redrawn only when the score changes.
    fn draw_score(&self, frame: &mut Frame) {
        // One u16 per digit: a 3×5 glyph, bit (row*3 + col) set = pixel on. Small
        // (10 words) so it stays well inside the dialect's local-scratch budget.
        let font = [
            31599u16, 9362u16, 29671u16, 31207u16, 18925u16, 31183u16, 31695u16, 9383u16,
            31727u16, 31215u16,
        ];
        frame.clear_cell(0u8, 0u8);
        let dt = font[((self.score / 10u16) % 10u16) as usize]; // tens
        let du = font[(self.score % 10u16) as usize]; // units
        // Walk a mask over bits 0..15 in (row, col) order (col inner) — `<< 1` is a
        // constant shift (the dialect has no variable-amount shift).
        let mut m = 1u16;
        let mut r = 0u16;
        while r < 5u16 {
            let mut c = 0u16;
            while c < 3u16 {
                let y = (r + 1u16) as u8;
                if (dt & m) != 0u16 {
                    frame.pixel(c as u8, y, true); // tens at x = col
                }
                if (du & m) != 0u16 {
                    frame.pixel((c + 4u16) as u8, y, true); // units at x = 4 + col
                }
                m = m << 1u16;
                c = c + 1u16;
            }
            r = r + 1u16;
        }
    }
}

impl Game for Snake {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        // (Re)start: clear, seed the body/food, draw the opening frame. Setting
        // `started = 0` (on Fire, or when the crash countdown ends) routes back here.
        if self.started == 0u16 {
            frame.clear(Colour::Black);
            self.len = 3u16;
            self.score = 0u16;
            self.dir = 0u16;
            self.tick = 0u16;
            self.dead = 0u16;
            self.rng = 1u16;
            self.bx[0] = 8u16;
            self.by[0] = 12u16;
            self.bx[1] = 7u16;
            self.by[1] = 12u16;
            self.bx[2] = 6u16;
            self.by[2] = 12u16;
            self.food_x = 16u16;
            self.food_y = 10u16;
            frame.fill_cell(self.bx[0] as u8, self.by[0] as u8, Colour::BrightGreen);
            frame.fill_cell(self.bx[1] as u8, self.by[1] as u8, Colour::BrightGreen);
            frame.fill_cell(self.bx[2] as u8, self.by[2] as u8, Colour::BrightGreen);
            frame.fill_cell(self.food_x as u8, self.food_y as u8, Colour::BrightRed);
            self.draw_score(frame);
            self.started = 1u16;
        }

        if self.dead != 0u16 {
            // Crashed: Fire restarts at once, otherwise count down to an auto-restart.
            if input.held(Button::Fire) {
                self.started = 0u16;
            } else {
                self.dead = self.dead - 1u16;
                if self.dead == 0u16 {
                    self.started = 0u16;
                }
            }
        } else {
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

                // Next head cell. Walls are the playfield edges: columns 0..32, rows
                // 1..24 (row 0 is the score bar). A move into a wall is game over.
                let mut nx = self.bx[0];
                let mut ny = self.by[0];
                let mut wall = 0u16;
                if self.dir == 0u16 {
                    if nx >= 31u16 {
                        wall = 1u16;
                    } else {
                        nx = nx + 1u16;
                    }
                }
                if self.dir == 1u16 {
                    if ny >= 23u16 {
                        wall = 1u16;
                    } else {
                        ny = ny + 1u16;
                    }
                }
                if self.dir == 2u16 {
                    if nx == 0u16 {
                        wall = 1u16;
                    } else {
                        nx = nx - 1u16;
                    }
                }
                if self.dir == 3u16 {
                    if ny <= 1u16 {
                        wall = 1u16;
                    } else {
                        ny = ny - 1u16;
                    }
                }

                // Self-collision: does the new head hit a body segment? Skip segment 0
                // (the current head) and the last (the tail, which vacates this move).
                let mut hit = wall;
                if wall == 0u16 {
                    let mut k = 1u16;
                    while k < self.len - 1u16 {
                        if self.bx[k as usize] == nx {
                            if self.by[k as usize] == ny {
                                hit = 1u16;
                            }
                        }
                        k = k + 1u16;
                    }
                }

                if hit != 0u16 {
                    // Crash: flash the head and start the restart countdown (~1s).
                    self.dead = 50u16;
                    frame.fill_cell(self.bx[0] as u8, self.by[0] as u8, Colour::BrightYellow);
                } else {
                    let mut ate = 0u16;
                    if nx == self.food_x {
                        if ny == self.food_y {
                            ate = 1u16;
                        }
                    }

                    let mut grew = 0u16;
                    if ate == 1u16 {
                        // Score the food and redraw the row-0 number.
                        self.score = self.score + 1u16;
                        self.draw_score(frame);

                        // Respawn food on a FREE cell (rows 1..24): re-roll the xorshift
                        // until it lands off the body, so it can't spawn under the snake.
                        loop {
                            self.rng = self.rng ^ (self.rng << 7u16);
                            self.rng = self.rng ^ (self.rng >> 9u16);
                            self.rng = self.rng ^ (self.rng << 8u16);
                            let fx = self.rng % 32u16;
                            let fy = (self.rng / 32u16) % 23u16 + 1u16;
                            let mut on_body = 0u16;
                            let mut b = 0u16;
                            while b < self.len {
                                if self.bx[b as usize] == fx {
                                    if self.by[b as usize] == fy {
                                        on_body = 1u16;
                                    }
                                }
                                b = b + 1u16;
                            }
                            if on_body == 0u16 {
                                self.food_x = fx;
                                self.food_y = fy;
                                break;
                            }
                        }

                        if self.len < 32u16 {
                            self.len = self.len + 1u16;
                            grew = 1u16;
                        }
                    }

                    // Erase the vacated tail cell unless the body grew this tick.
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

                    // Draw only what changed: the new head (and the new food if eaten).
                    frame.fill_cell(nx as u8, ny as u8, Colour::BrightGreen);
                    if ate == 1u16 {
                        frame.fill_cell(self.food_x as u8, self.food_y as u8, Colour::BrightRed);
                    }
                }
            }
        }
    }
}
