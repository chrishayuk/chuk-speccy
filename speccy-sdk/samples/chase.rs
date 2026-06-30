// A chase/dungeon game — ONE source, two compilers. The pure **actor/AI** core:
// several enemies that home in on the player, drawn blocky with `fill_cell`.
//
//   host:  an ordinary `speccy-sdk` Game (see tests/dial.rs).
//   pure:  speccy-compile speccy-sdk/samples/chase.rs  ->  a bootable .tap
//          speccy-gui testroms/48.rom chase.tap
//
// Move with the cursor keys (or QAOP) around a walled room with pillars; grab all four
// yellow coins to win. Three magenta enemies greedily chase you each tick (horizontal
// first, then vertical, routing around walls) — touch one and you die, then it restarts.
//
// The actor layer is **parallel `[u16; 3]` arrays** (`ex`/`ey`) — the dialect has no
// `Vec`/`Entities` of structs, so a pool is parallel arrays, exactly like `snake_game`'s
// body. The chase is deterministic (a pure function of positions), so the episode
// rewinds/replays/RLs. Drawing is incremental: the room is drawn once, then each tick
// erases the old actor cells and redraws coins/enemies/player. Typed state
// (`x`,`y`,`score`,`won`,`dead`,`ex[]`,`ey[]`) is what the `.sym.toml` exposes, so an env
// reads + scores it off RAM (avoid-the-enemies is a clean reward). Flags are `bool`s; the
// coin-collected flags stay `[u16; 4]` (the dialect has no `bool` arrays yet).

// Is level cell (cx, cy) a wall? A bordered room (row 0 is the score bar) with three
// pillars. One map for the draw *and* both the player and enemy movement.
fn solid(cx: u16, cy: u16) -> bool {
    let mut s = false;
    if cx == 0u16 {
        s = true;
    }
    if cx >= 31u16 {
        s = true;
    }
    if cy <= 1u16 {
        s = true; // top wall (row 0 is the score bar, row 1 the wall)
    }
    if cy >= 23u16 {
        s = true; // bottom wall
    }
    if cy == 8u16 {
        if cx >= 8u16 {
            if cx <= 12u16 {
                s = true; // pillar
            }
        }
    }
    if cy == 15u16 {
        if cx >= 18u16 {
            if cx <= 24u16 {
                s = true; // pillar
            }
        }
    }
    if cx == 16u16 {
        if cy >= 10u16 {
            if cy <= 14u16 {
                s = true; // pillar
            }
        }
    }
    s
}

#[derive(Default)]
struct Chase {
    started: bool,
    x: u16,
    y: u16,
    tick: u16,
    score: u16, // coins collected
    won: bool,
    dead: u16, // 0 = alive; otherwise a restart countdown
    cgx: [u16; 4],
    cgy: [u16; 4],
    got: [u16; 4], // coin collected? (no `bool` arrays in the dialect yet)
    ex: [u16; 3],  // enemy x's (parallel arrays = the actor pool)
    ey: [u16; 3],  // enemy y's
}

impl Chase {
    // Draw the static room once (white walls).
    fn draw_room(&self, frame: &mut Frame) {
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
    }

    // Redraw the dynamic layer: uncollected coins (yellow), enemies (magenta), player
    // (cyan). Cheap (a handful of cells), so it runs every tick on a real Z80.
    fn draw_actors(&self, frame: &mut Frame) {
        let mut k = 0u16;
        while k < 4u16 {
            if self.got[k as usize] == 0u16 {
                frame.fill_cell(
                    self.cgx[k as usize] as u8,
                    self.cgy[k as usize] as u8,
                    Colour::BrightYellow,
                );
            }
            k = k + 1u16;
        }
        let mut e = 0u16;
        while e < 3u16 {
            frame.fill_cell(self.ex[e as usize] as u8, self.ey[e as usize] as u8, Colour::BrightMagenta);
            e = e + 1u16;
        }
        frame.fill_cell(self.x as u8, self.y as u8, Colour::BrightCyan);
    }

    // Step one enemy toward the player: greedy chase — horizontal gap first, then vertical,
    // into free cells only (routes around walls). A clean `&mut self` method; rustz80 inlines
    // its single call site (argument substitution + slot reuse), so the tape is as compact
    // as hand-inlining — no per-call cost.
    fn step_enemy(&mut self, e: u16) {
        let i = e as usize;
        let mut moved = false;
        if self.ex[i] < self.x {
            if !solid(self.ex[i] + 1u16, self.ey[i]) {
                self.ex[i] = self.ex[i] + 1u16;
                moved = true;
            }
        }
        if !moved {
            if self.ex[i] > self.x {
                if !solid(self.ex[i] - 1u16, self.ey[i]) {
                    self.ex[i] = self.ex[i] - 1u16;
                    moved = true;
                }
            }
        }
        if !moved {
            if self.ey[i] < self.y {
                if !solid(self.ex[i], self.ey[i] + 1u16) {
                    self.ey[i] = self.ey[i] + 1u16;
                    moved = true;
                }
            }
        }
        if !moved {
            if self.ey[i] > self.y {
                if !solid(self.ex[i], self.ey[i] - 1u16) {
                    self.ey[i] = self.ey[i] - 1u16;
                }
            }
        }
    }

    // Did any enemy land on the player? A value-returning `&self` method (its result binds to
    // a local at the call site, so it inlines too).
    fn caught(&self) -> bool {
        let mut hit = false;
        let mut e = 0u16;
        while e < 3u16 {
            if self.ex[e as usize] == self.x {
                if self.ey[e as usize] == self.y {
                    hit = true;
                }
            }
            e = e + 1u16;
        }
        hit
    }
}

impl Game for Chase {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if !self.started {
            frame.clear(Colour::Black);
            self.draw_room(frame);
            self.x = 15u16;
            self.y = 12u16;
            self.tick = 0u16;
            self.score = 0u16;
            self.won = false;
            self.dead = 0u16;
            self.cgx[0] = 5u16;
            self.cgy[0] = 5u16;
            self.cgx[1] = 26u16;
            self.cgy[1] = 5u16;
            self.cgx[2] = 5u16;
            self.cgy[2] = 20u16;
            self.cgx[3] = 26u16;
            self.cgy[3] = 20u16;
            self.got[0] = 0u16;
            self.got[1] = 0u16;
            self.got[2] = 0u16;
            self.got[3] = 0u16;
            self.ex[0] = 2u16;
            self.ey[0] = 2u16;
            self.ex[1] = 29u16;
            self.ey[1] = 2u16;
            self.ex[2] = 15u16;
            self.ey[2] = 22u16;
            self.draw_actors(frame);
            self.started = true;
        }

        if self.dead != 0u16 {
            // GAME OVER — a red band across the middle (no text in the pure envelope).
            let mut bx = 6u16;
            while bx < 26u16 {
                frame.fill_cell(bx as u8, 11u8, Colour::BrightRed);
                frame.fill_cell(bx as u8, 12u8, Colour::BrightRed);
                bx = bx + 1u16;
            }
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
                // YOU WIN — a green band.
                let mut wx = 6u16;
                while wx < 26u16 {
                    frame.fill_cell(wx as u8, 11u8, Colour::BrightGreen);
                    frame.fill_cell(wx as u8, 12u8, Colour::BrightGreen);
                    wx = wx + 1u16;
                }
                if input.held(Button::Fire) {
                    self.started = false;
                }
            } else {
                self.tick = self.tick + 1u16;
                if self.tick >= 4u16 {
                    self.tick = 0u16;

                    // Erase the player and enemies at their old cells.
                    frame.clear_cell(self.x as u8, self.y as u8);
                    let mut e0 = 0u16;
                    while e0 < 3u16 {
                        frame.clear_cell(self.ex[e0 as usize] as u8, self.ey[e0 as usize] as u8);
                        e0 = e0 + 1u16;
                    }

                    // Move the player on a held direction (into free cells only).
                    if input.held(Button::Left) {
                        if !solid(self.x - 1u16, self.y) {
                            self.x = self.x - 1u16;
                        }
                    }
                    if input.held(Button::Right) {
                        if !solid(self.x + 1u16, self.y) {
                            self.x = self.x + 1u16;
                        }
                    }
                    if input.held(Button::Up) {
                        if !solid(self.x, self.y - 1u16) {
                            self.y = self.y - 1u16;
                        }
                    }
                    if input.held(Button::Down) {
                        if !solid(self.x, self.y + 1u16) {
                            self.y = self.y + 1u16;
                        }
                    }

                    // Collect a coin under the player.
                    let mut k = 0u16;
                    while k < 4u16 {
                        if self.got[k as usize] == 0u16 {
                            if self.cgx[k as usize] == self.x {
                                if self.cgy[k as usize] == self.y {
                                    self.got[k as usize] = 1u16;
                                    self.score = self.score + 1u16;
                                    // Score bar: one cyan cell per coin along row 0.
                                    frame.fill_cell((self.score - 1u16) as u8, 0u8, Colour::BrightCyan);
                                }
                            }
                        }
                        k = k + 1u16;
                    }
                    if self.score >= 4u16 {
                        self.won = true;
                    }

                    // Step every enemy toward the player (clean method per enemy; the single
                    // call site inlines with arg substitution + slot reuse, so the tape is as
                    // compact as hand-inlining).
                    let mut e = 0u16;
                    while e < 3u16 {
                        self.step_enemy(e);
                        e = e + 1u16;
                    }

                    // A touch is fatal — flash the player cell yellow (like snake).
                    if self.caught() {
                        self.dead = 40u16;
                        frame.fill_cell(self.x as u8, self.y as u8, Colour::BrightYellow);
                    }

                    // Redraw the dynamic layer.
                    self.draw_actors(frame);
                }
            }
        }
    }
}
