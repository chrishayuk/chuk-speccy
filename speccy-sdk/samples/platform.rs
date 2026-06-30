// A platformer — ONE source, two compilers. The pure *arcade core* (gravity, jump,
// tile collision), drawn blocky with `fill_cell`, all inside the dialect envelope.
//
//   host:  an ordinary `speccy-sdk` Game (see tests/dial.rs).
//   pure:  speccy-compile speccy-sdk/samples/platform.rs  ->  a bootable .tap
//          speccy-gui testroms/48.rom platform.tap
//
// Walk with Left/Right, jump with Up (or Fire). Stand on the white platforms, collect
// the yellow coins (row 0 fills cyan, one cell per coin), reach the green exit. Fall in
// the pit = death, then it restarts (Fire, or after a short pause).
//
// No signed ints / fixed-point (the dialect has neither): the jump is a **phase
// counter** (rise N cells, then gravity pulls you down), exactly the trick `bounce.rs`
// uses for its direction flags. Collision reads one level map — the `solid(cx, cy)`
// free function — shared by the one-time level draw and the per-step physics. The level
// is drawn once; each step only erases/redraws the moving player (O(1), so it stays fast
// on a real Z80). Typed state (`x`, `y`, `score`, `won`, `dead`, …) is what the emitted
// `.sym.toml` exposes, so an env reads and scores the live game off RAM. Flags are `bool`s
// (`won`, `started`, `onground`); `jump`/`dead` are u16 counters; coins stay `[u16; 3]`.

// Is level cell (cx, cy) solid? Walls (cols 0 / 31), a floor (rows 22+) with a pit at
// cols 14..16, and three platforms. One map for drawing *and* physics — no drift.
fn solid(cx: u16, cy: u16) -> bool {
    let mut s = false;
    if cx == 0u16 {
        s = true;
    }
    if cx >= 31u16 {
        s = true;
    }
    if cy >= 22u16 {
        s = true;
        if cx >= 14u16 {
            if cx <= 15u16 {
                s = false; // the pit (a gap in the floor)
            }
        }
    }
    if cy == 18u16 {
        if cx >= 6u16 {
            if cx <= 11u16 {
                s = true; // low platform
            }
        }
    }
    if cy == 14u16 {
        if cx >= 16u16 {
            if cx <= 23u16 {
                s = true; // mid platform
            }
        }
    }
    if cy == 10u16 {
        if cx >= 24u16 {
            if cx <= 29u16 {
                s = true; // high platform (the exit sits here)
            }
        }
    }
    s
}

#[derive(Default)]
struct Platform {
    started: bool,
    x: u16,
    y: u16,
    jump: u16, // up-moves left in the current jump (0 = falling / grounded)
    tick: u16,
    score: u16, // coins collected
    won: bool,
    dead: u16, // 0 = alive; otherwise a restart countdown
    cgx: [u16; 3],
    cgy: [u16; 3],
    got: [u16; 3], // coin collected? (no `bool` arrays in the dialect yet)
}

impl Platform {
    // Draw the static world once: platforms (white), coins (yellow), exit (green).
    fn draw_level(&mut self, frame: &mut Frame) {
        let mut cy = 1u16;
        while cy < 24u16 {
            let mut cx = 0u16;
            while cx < 32u16 {
                if solid(cx, cy) {
                    frame.fill_cell(cx as u8, cy as u8, Colour::White);
                }
                cx = cx + 1u16;
            }
            cy = cy + 1u16;
        }
        self.cgx[0] = 8u16;
        self.cgy[0] = 17u16;
        self.cgx[1] = 20u16;
        self.cgy[1] = 13u16;
        self.cgx[2] = 27u16;
        self.cgy[2] = 9u16;
        let mut k = 0u16;
        while k < 3u16 {
            self.got[k as usize] = 0u16;
            frame.fill_cell(self.cgx[k as usize] as u8, self.cgy[k as usize] as u8, Colour::BrightYellow);
            k = k + 1u16;
        }
        frame.fill_cell(28u8, 9u8, Colour::BrightGreen); // the exit
    }
}

impl Game for Platform {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if !self.started {
            frame.clear(Colour::Black);
            self.draw_level(frame);
            self.x = 2u16;
            self.y = 21u16;
            self.jump = 0u16;
            self.tick = 0u16;
            self.score = 0u16;
            self.won = false;
            self.dead = 0u16;
            frame.fill_cell(self.x as u8, self.y as u8, Colour::BrightCyan);
            frame.text_u16(0u8, 0u8, self.score); // numeric score (ROM-font, shared routine)
            self.started = true;
        }

        if self.dead != 0u16 {
            if input.held(Button::Fire) {
                self.started = false;
            } else {
                self.dead = self.dead - 1u16;
                if self.dead == 0u16 {
                    self.started = false;
                }
            }
        } else {
            if self.won {
                if input.held(Button::Fire) {
                    self.started = false;
                }
            } else {
                self.tick = self.tick + 1u16;
                if self.tick >= 3u16 {
                    self.tick = 0u16;

                    // Erase the player at the old (empty) cell.
                    frame.clear_cell(self.x as u8, self.y as u8);

                    // Walk left / right when the target cell is free.
                    if input.held(Button::Left) {
                        if self.x > 0u16 {
                            if !solid(self.x - 1u16, self.y) {
                                self.x = self.x - 1u16;
                            }
                        }
                    }
                    if input.held(Button::Right) {
                        if self.x < 31u16 {
                            if !solid(self.x + 1u16, self.y) {
                                self.x = self.x + 1u16;
                            }
                        }
                    }

                    // On the ground if the cell below is solid (or we're at the bottom).
                    let mut onground = false;
                    if self.y >= 23u16 {
                        onground = true;
                    }
                    if solid(self.x, self.y + 1u16) {
                        onground = true;
                    }

                    // Start a jump (rise 5 cells) from the ground.
                    if self.jump == 0u16 {
                        if onground {
                            if input.held(Button::Up) {
                                self.jump = 5u16;
                            }
                            if input.held(Button::Fire) {
                                self.jump = 5u16;
                            }
                        }
                    }

                    // Rise while jumping (until a ceiling), else fall under gravity.
                    if self.jump > 0u16 {
                        if self.y > 1u16 {
                            if !solid(self.x, self.y - 1u16) {
                                self.y = self.y - 1u16;
                                self.jump = self.jump - 1u16;
                            } else {
                                self.jump = 0u16;
                            }
                        } else {
                            self.jump = 0u16;
                        }
                    } else {
                        if !solid(self.x, self.y + 1u16) {
                            if self.y >= 23u16 {
                                self.dead = 40u16; // fell off the bottom (the pit)
                            } else {
                                self.y = self.y + 1u16;
                            }
                        }
                    }

                    // Collect a coin at the new cell.
                    let mut k = 0u16;
                    while k < 3u16 {
                        if self.got[k as usize] == 0u16 {
                            if self.cgx[k as usize] == self.x {
                                if self.cgy[k as usize] == self.y {
                                    self.got[k as usize] = 1u16;
                                    self.score = self.score + 1u16;
                                    frame.text_u16(0u8, 0u8, self.score); // numeric score
                                }
                            }
                        }
                        k = k + 1u16;
                    }

                    // Reach the exit cell → win.
                    if self.x == 28u16 {
                        if self.y == 9u16 {
                            self.won = true;
                        }
                    }

                    // Redraw the player (unless this step killed it).
                    if self.dead == 0u16 {
                        frame.fill_cell(self.x as u8, self.y as u8, Colour::BrightCyan);
                    }
                }
            }
        }
    }
}
