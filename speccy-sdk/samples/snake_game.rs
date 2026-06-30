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
            31599, 9362, 29671, 31207, 18925, 31183, 31695, 9383,
            31727, 31215,
        ];
        frame.clear_cell(0, 0);
        let dt = font[((self.score / 10) % 10) as usize]; // tens
        let du = font[(self.score % 10) as usize]; // units
        // Walk a mask over bits 0..15 in (row, col) order (col inner) — `<< 1` is a
        // constant shift (the dialect has no variable-amount shift).
        let mut m = 1;
        let mut r = 0;
        while r < 5 {
            let mut c = 0;
            while c < 3 {
                let y = (r + 1) as u8;
                if (dt & m) != 0 {
                    frame.pixel(c as u8, y, true); // tens at x = col
                }
                if (du & m) != 0 {
                    frame.pixel((c + 4) as u8, y, true); // units at x = 4 + col
                }
                m = m << 1;
                c = c + 1;
            }
            r = r + 1;
        }
    }
}

impl Game for Snake {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        // (Re)start: clear, seed the body/food, draw the opening frame. Setting
        // `started = 0` (on Fire, or when the crash countdown ends) routes back here.
        if self.started == 0 {
            frame.clear(Colour::Black);
            self.len = 3;
            self.score = 0;
            self.dir = 0;
            self.tick = 0;
            self.dead = 0;
            self.rng = 1;
            self.bx[0] = 8;
            self.by[0] = 12;
            self.bx[1] = 7;
            self.by[1] = 12;
            self.bx[2] = 6;
            self.by[2] = 12;
            self.food_x = 16;
            self.food_y = 10;
            frame.fill_cell(self.bx[0] as u8, self.by[0] as u8, Colour::BrightGreen);
            frame.fill_cell(self.bx[1] as u8, self.by[1] as u8, Colour::BrightGreen);
            frame.fill_cell(self.bx[2] as u8, self.by[2] as u8, Colour::BrightGreen);
            frame.fill_cell(self.food_x as u8, self.food_y as u8, Colour::BrightRed);
            self.draw_score(frame);
            self.started = 1;
        }

        if self.dead != 0 {
            // Crashed: Fire restarts at once, otherwise count down to an auto-restart.
            if input.held(Button::Fire) {
                self.started = 0;
            } else {
                self.dead = self.dead - 1;
                if self.dead == 0 {
                    self.started = 0;
                }
            }
        } else {
            // Steer (no reversing straight back onto the neck).
            if input.held(Button::Up) {
                if self.dir != 1 {
                    self.dir = 3;
                }
            }
            if input.held(Button::Down) {
                if self.dir != 3 {
                    self.dir = 1;
                }
            }
            if input.held(Button::Left) {
                if self.dir != 0 {
                    self.dir = 2;
                }
            }
            if input.held(Button::Right) {
                if self.dir != 2 {
                    self.dir = 0;
                }
            }

            self.tick = self.tick + 1;
            if self.tick >= 4 {
                self.tick = 0;

                // Next head cell. Walls are the playfield edges: columns 0..32, rows
                // 1..24 (row 0 is the score bar). A move into a wall is game over.
                let mut nx = self.bx[0];
                let mut ny = self.by[0];
                let mut wall = 0;
                if self.dir == 0 {
                    if nx >= 31 {
                        wall = 1;
                    } else {
                        nx = nx + 1;
                    }
                }
                if self.dir == 1 {
                    if ny >= 23 {
                        wall = 1;
                    } else {
                        ny = ny + 1;
                    }
                }
                if self.dir == 2 {
                    if nx == 0 {
                        wall = 1;
                    } else {
                        nx = nx - 1;
                    }
                }
                if self.dir == 3 {
                    if ny <= 1 {
                        wall = 1;
                    } else {
                        ny = ny - 1;
                    }
                }

                // Self-collision: does the new head hit a body segment? Skip segment 0
                // (the current head) and the last (the tail, which vacates this move).
                let mut hit = wall;
                if wall == 0 {
                    let mut k = 1;
                    while k < self.len - 1 {
                        if self.bx[k as usize] == nx {
                            if self.by[k as usize] == ny {
                                hit = 1;
                            }
                        }
                        k = k + 1;
                    }
                }

                if hit != 0 {
                    // Crash: flash the head and start the restart countdown (~1s).
                    self.dead = 50;
                    frame.fill_cell(self.bx[0] as u8, self.by[0] as u8, Colour::BrightYellow);
                } else {
                    let mut ate = 0;
                    if nx == self.food_x {
                        if ny == self.food_y {
                            ate = 1;
                        }
                    }

                    let mut grew = 0;
                    if ate == 1 {
                        // Score the food and redraw the row-0 number.
                        self.score = self.score + 1;
                        self.draw_score(frame);

                        // Respawn food on a FREE cell (rows 1..24): re-roll the xorshift
                        // until it lands off the body, so it can't spawn under the snake.
                        loop {
                            self.rng = self.rng ^ (self.rng << 7);
                            self.rng = self.rng ^ (self.rng >> 9);
                            self.rng = self.rng ^ (self.rng << 8);
                            let fx = self.rng % 32;
                            let fy = (self.rng / 32) % 23 + 1;
                            let mut on_body = 0;
                            let mut b = 0;
                            while b < self.len {
                                if self.bx[b as usize] == fx {
                                    if self.by[b as usize] == fy {
                                        on_body = 1;
                                    }
                                }
                                b = b + 1;
                            }
                            if on_body == 0 {
                                self.food_x = fx;
                                self.food_y = fy;
                                break;
                            }
                        }

                        if self.len < 32 {
                            self.len = self.len + 1;
                            grew = 1;
                        }
                    }

                    // Erase the vacated tail cell unless the body grew this tick.
                    if grew == 0 {
                        let t = self.len - 1;
                        frame.clear_cell(self.bx[t as usize] as u8, self.by[t as usize] as u8);
                    }

                    // Shift the body toward the head, then place the new head.
                    let mut j = self.len - 1;
                    while j > 0 {
                        self.bx[j as usize] = self.bx[(j - 1) as usize];
                        self.by[j as usize] = self.by[(j - 1) as usize];
                        j = j - 1;
                    }
                    self.bx[0] = nx;
                    self.by[0] = ny;

                    // Draw only what changed: the new head (and the new food if eaten).
                    frame.fill_cell(nx as u8, ny as u8, Colour::BrightGreen);
                    if ate == 1 {
                        frame.fill_cell(self.food_x as u8, self.food_y as u8, Colour::BrightRed);
                    }
                }
            }
        }
    }
}
