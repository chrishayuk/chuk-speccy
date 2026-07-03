// An item-fetch adventure — ONE source, two compilers. The pure **inventory /
// use-on-obstacle** core (spec 08 §9, capability C8 — the Dizzy/Booty mechanic): pick
// up an item, carry it, and bump it into the one obstacle it clears.
//
//   host:  an ordinary `speccy-sdk` Game (see tests/dial.rs).
//   pure:  speccy-compile speccy-sdk/samples/island.rs  ->  a bootable .tap
//          speccy-gui testroms/48.rom island.tap
//
// Move with the cursor keys (or QAOP). Two rooms, each split by an obstacle column:
// room 0's green **vine** needs the yellow **axe**; room 1's red **door** needs the
// magenta **key**. Walking onto an item auto-picks it up (one at a time, Dizzy-style —
// no separate "take" button); bumping the matching obstacle while carrying it clears
// the obstacle *and* consumes the item, in the same step that walks you through. Reach
// the far side of room 1 (where the door was) to win.
//
// This is the SDK's first sample with **stateful** obstacles: `chase`/`maze`/`platform`'s
// `solid`/`wall` map is a pure function of position, but here whether column 16 blocks
// depends on `self.vine_cleared`/`self.door_open` — so `wall` is an `&self` method, not a
// free function. No arrays needed (a single item slot, not a pool), so this sample is
// structurally simpler than `chase`/`snake_game` despite the new mechanic. Typed state
// (`held`, `axe_taken`, `vine_cleared`, `key_taken`, `door_open`, `won`) is exactly what
// an env needs to score *subgoals*, not just movement — the puzzle/adventure cognitive
// axis, not exploration or evasion.

#[derive(Default)]
struct Island {
    started: bool,
    x: u16,
    y: u16,
    room: u16,
    tick: u16,
    held: u16, // 0 = empty-handed, 1 = axe, 2 = key
    axe_taken: bool,
    vine_cleared: bool,
    key_taken: bool,
    door_open: bool,
    won: bool,
}

impl Island {
    // Is level cell (cx, cy) blocked? A bordered room (row 0 is the inventory bar, row
    // 1 the wall) split by one obstacle column at cx 16 — green vine in room 0 (needs
    // the axe), red door in room 1 (needs the key) — solid until cleared.
    fn wall(&self, cx: u16, cy: u16) -> bool {
        let mut s = false;
        if cx == 0 {
            s = true;
        }
        if cx >= 31 {
            s = true;
        }
        if cy <= 1 {
            s = true;
        }
        if cy >= 23 {
            s = true;
        }
        if cx == 16 {
            if self.room == 0 {
                if !self.vine_cleared {
                    s = true;
                }
            } else {
                if !self.door_open {
                    s = true;
                }
            }
        }
        s
    }

    // Draw the room's static geometry: border + obstacle column (green/red until
    // cleared, then it just stops being drawn — the cell reads as open floor).
    fn draw_room(&self, frame: &mut Frame) {
        let mut cy = 2;
        while cy < 23 {
            let mut cx = 0;
            while cx < 32 {
                if self.wall(cx, cy) {
                    let mut c = Colour::White;
                    if cx == 16 {
                        if self.room == 0 {
                            c = Colour::Green;
                        } else {
                            c = Colour::Red;
                        }
                    }
                    frame.fill_cell(cx as u8, cy as u8, c);
                }
                cx = cx + 1;
            }
            cy = cy + 1;
        }
    }

    // The item (if not yet taken) and the far-side marker (exit / treasure).
    fn draw_markers(&self, frame: &mut Frame) {
        if self.room == 0 {
            if !self.axe_taken {
                frame.fill_cell(5, 5, Colour::BrightYellow);
            }
        } else {
            if !self.key_taken {
                frame.fill_cell(5, 5, Colour::BrightMagenta);
            }
        }
        frame.fill_cell(29, 12, Colour::BrightGreen);
    }

    // A single cell in the reserved row 0: black empty-handed, yellow carrying the
    // axe, magenta carrying the key — the inventory "slot", Dizzy has one too.
    fn draw_inventory(&self, frame: &mut Frame) {
        let mut c = Colour::Black;
        if self.held == 1 {
            c = Colour::BrightYellow;
        }
        if self.held == 2 {
            c = Colour::BrightMagenta;
        }
        frame.fill_cell(0, 0, c);
    }

    // Enter `self.room` fresh: clear the screen, draw it, place the player at the
    // start, and show the current inventory. Called from both `!started` init and
    // the room-transition path — the *second* call runs an extra ~20 frames slower on
    // real hardware than the first (a known `rustz80` codegen stall on a repeat call
    // to a method containing a looping prelude call — see `dial.rs`'s
    // `rustz80_stalls_on_a_second_call_to_a_looping_prelude_routine`); it still
    // completes correctly, just late, so nothing here needs to work around it.
    fn enter_room(&mut self, frame: &mut Frame) {
        frame.clear(Colour::Black);
        self.draw_room(frame);
        self.draw_markers(frame);
        self.x = 2;
        self.y = 12;
        self.draw_inventory(frame);
    }

    // Move on a held direction into a free cell. Bumping *toward* the obstacle column
    // (from the left, the only approach in this layout) tries to use the held item
    // first — clearing it in the same step lets the wall check right after fall
    // through, so a single press both uses the item and walks you through.
    fn move_player(&mut self, input: &Input, frame: &mut Frame) {
        if input.held(Button::Left) {
            if self.x > 0 {
                if !self.wall(self.x - 1, self.y) {
                    self.x = self.x - 1;
                }
            }
        }
        if input.held(Button::Right) {
            if self.x + 1 == 16 {
                self.try_use(frame);
            }
            if !self.wall(self.x + 1, self.y) {
                self.x = self.x + 1;
            }
        }
        if input.held(Button::Up) {
            if self.y > 0 {
                if !self.wall(self.x, self.y - 1) {
                    self.y = self.y - 1;
                }
            }
        }
        if input.held(Button::Down) {
            if !self.wall(self.x, self.y + 1) {
                self.y = self.y + 1;
            }
        }
    }

    // Standing empty-handed on an uncollected item takes it — no separate button, the
    // way Dizzy does it.
    fn try_pickup(&mut self, frame: &mut Frame) {
        if self.held == 0 {
            if self.room == 0 {
                if !self.axe_taken {
                    if self.x == 5 {
                        if self.y == 5 {
                            self.axe_taken = true;
                            self.held = 1;
                            frame.clear_cell(5, 5);
                            self.draw_inventory(frame);
                        }
                    }
                }
            } else {
                if !self.key_taken {
                    if self.x == 5 {
                        if self.y == 5 {
                            self.key_taken = true;
                            self.held = 2;
                            frame.clear_cell(5, 5);
                            self.draw_inventory(frame);
                        }
                    }
                }
            }
        }
    }

    // Bumping the obstacle column while holding the item it needs clears it and
    // consumes the item — a no-op once already cleared, or if holding the wrong thing.
    fn try_use(&mut self, frame: &mut Frame) {
        if self.room == 0 {
            if !self.vine_cleared {
                if self.held == 1 {
                    self.vine_cleared = true;
                    self.held = 0;
                    self.draw_room(frame);
                    self.draw_inventory(frame);
                }
            }
        } else {
            if !self.door_open {
                if self.held == 2 {
                    self.door_open = true;
                    self.held = 0;
                    self.draw_room(frame);
                    self.draw_inventory(frame);
                }
            }
        }
    }
}

impl Game for Island {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if !self.started {
            self.room = 0;
            self.held = 0;
            self.axe_taken = false;
            self.vine_cleared = false;
            self.key_taken = false;
            self.door_open = false;
            self.won = false;
            self.tick = 0;
            self.enter_room(frame);
            frame.fill_cell(2, 12, Colour::BrightCyan);
            self.started = true;
        }

        if self.won {
            // YOU WIN — a green band across the middle (no text in the pure envelope).
            let mut wx = 6;
            while wx < 26 {
                frame.fill_cell(wx as u8, 11, Colour::BrightGreen);
                frame.fill_cell(wx as u8, 12, Colour::BrightGreen);
                wx = wx + 1;
            }
            if input.held(Button::Fire) {
                self.started = false;
            }
        } else {
            // Constant speed: step once every few frames regardless of the host's rate.
            self.tick = self.tick + 1;
            if self.tick >= 3 {
                self.tick = 0;

                frame.clear_cell(self.x as u8, self.y as u8);
                self.move_player(input, frame);
                self.try_pickup(frame);

                if self.room == 0 {
                    if self.x == 29 {
                        if self.y == 12 {
                            self.room = 1;
                            self.enter_room(frame);
                        }
                    }
                } else {
                    if self.x == 29 {
                        if self.y == 12 {
                            self.won = true;
                        }
                    }
                }

                if !self.won {
                    frame.fill_cell(self.x as u8, self.y as u8, Colour::BrightCyan);
                }
            }
        }
    }
}
