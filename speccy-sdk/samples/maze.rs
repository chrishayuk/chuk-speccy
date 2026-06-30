// A maze game — ONE source, two compilers. A pure **scene-flow** core: walk a walled
// maze to the green exit; reaching it loads the next room, and clearing the last room
// wins.
//
//   host:  an ordinary `speccy-sdk` Game (see tests/dial.rs).
//   pure:  speccy-compile speccy-sdk/samples/maze.rs  ->  a bootable .tap
//          speccy-gui testroms/48.rom maze.tap
//
// Move with the cursor keys (or QAOP). The wall map is one function `wall(cx, cy, room)`
// driving **both** the draw and the player collision — and it's parameterised by `room`,
// so the same code renders and gates several mazes (the scene-flow dimension). Each room
// is a serpentine of corridors with alternating side-gaps, so a start→exit path always
// exists. It's deterministic (walls are a pure function of the cell), so the episode
// rewinds/replays/RLs; typed state (`x`, `y`, `room`, `won`) reads off the `.sym.toml` —
// which is exactly what a pathfinding agent reads to solve the maze off RAM (no pixels).
//
// This is the SDK's first **decomposed** sample: `draw_maze`/`enter_room`/`move_player`
// are clean `&self`/`&mut self` methods. Each has a single call site, so rustz80's inliner
// folds them — the tape is as compact as if it were hand-inlined into `update`. Flags are
// real `bool`s (`if !self.started { self.started = true; }`), so the flow reads naturally.

// The wall map for `room`: a bordered grid plus a serpentine of corridors. Room 0 uses
// horizontal walls with alternating right/left gaps; room 1 uses vertical walls with
// alternating bottom/top gaps. Both keep the start (2,2) and exit (29,21) cells open and
// connected. One map for the draw *and* the player collision — returns `true` for a wall.
fn wall(cx: u16, cy: u16, room: u16) -> bool {
    let mut s = false;
    if cx == 0 {
        s = true;
    }
    if cx >= 31 {
        s = true;
    }
    if cy == 0 {
        s = true;
    }
    if cy >= 23 {
        s = true;
    }
    if room == 0 {
        if cy == 6 {
            if cx <= 27 {
                s = true; // wall across, gap at cx 28..30
            }
        }
        if cy == 12 {
            if cx >= 4 {
                s = true; // wall across, gap at cx 1..3
            }
        }
        if cy == 18 {
            if cx <= 27 {
                s = true; // wall across, gap at cx 28..30
            }
        }
    } else {
        if cx == 8 {
            if cy <= 18 {
                s = true; // wall down, gap at cy 19..22
            }
        }
        if cx == 16 {
            if cy >= 5 {
                s = true; // wall down, gap at cy 1..4
            }
        }
        if cx == 24 {
            if cy <= 18 {
                s = true; // wall down, gap at cy 19..22
            }
        }
    }
    s
}

#[derive(Default)]
struct Maze {
    started: bool,
    x: u16,
    y: u16,
    room: u16,
    tick: u16,
    won: bool,
}

impl Maze {
    // Draw the static maze for the current room (white walls) + the green exit cell.
    fn draw_maze(&self, frame: &mut Frame) {
        let mut cy = 0;
        while cy < 24 {
            let mut cx = 0;
            while cx < 32 {
                if wall(cx, cy, self.room) {
                    frame.fill_cell(cx as u8, cy as u8, Colour::White);
                }
                cx = cx + 1;
            }
            cy = cy + 1;
        }
        frame.fill_cell(29, 21, Colour::BrightGreen);
    }

    // Enter `self.room`: clear the screen, draw its maze, and place the player at the start.
    fn enter_room(&mut self, frame: &mut Frame) {
        frame.clear(Colour::Black);
        self.draw_maze(frame);
        self.x = 2;
        self.y = 2;
    }

    // Step the player on a held direction, into free (non-wall) cells only. No signed
    // ints in the dialect, so we guard the `- 1` at the border (which is a wall anyway).
    fn move_player(&mut self, input: &Input) {
        if input.held(Button::Left) {
            if self.x > 0 {
                if !wall(self.x - 1, self.y, self.room) {
                    self.x = self.x - 1;
                }
            }
        }
        if input.held(Button::Right) {
            if !wall(self.x + 1, self.y, self.room) {
                self.x = self.x + 1;
            }
        }
        if input.held(Button::Up) {
            if self.y > 0 {
                if !wall(self.x, self.y - 1, self.room) {
                    self.y = self.y - 1;
                }
            }
        }
        if input.held(Button::Down) {
            if !wall(self.x, self.y + 1, self.room) {
                self.y = self.y + 1;
            }
        }
    }
}

impl Game for Maze {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if !self.started {
            self.room = 0;
            self.won = false;
            self.tick = 0;
            self.enter_room(frame);
            self.started = true;
        }

        if !self.won {
            // Constant speed: step once every few frames regardless of the host's rate.
            self.tick = self.tick + 1;
            if self.tick >= 3 {
                self.tick = 0;

                // Erase the player's old cell (always a free/black cell), then move.
                frame.clear_cell(self.x as u8, self.y as u8);
                self.move_player(input);

                // Reached the exit? Advance a room, or win on the last one.
                if self.x == 29 {
                    if self.y == 21 {
                        if self.room == 0 {
                            self.room = 1;
                            self.enter_room(frame);
                        } else {
                            self.won = true;
                        }
                    }
                }

                // Redraw the exit (the player may have stepped onto/off it) and the player.
                frame.fill_cell(29, 21, Colour::BrightGreen);
                frame.fill_cell(self.x as u8, self.y as u8, Colour::BrightCyan);
            }
        } else {
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
        }
    }
}
