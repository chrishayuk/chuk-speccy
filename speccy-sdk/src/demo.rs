//! First light: **Snake** in ~70 lines of [`Game`] — the canonical SDK demo.
//! Run it in the native window with `speccy-gui <rom> snake`.

use crate::{Button, Colour, Frame, Game, Input, BLOCK};

const W: u8 = 32; // playfield is the full 32×24 grid; row 0 shows the score
const TOP: u8 = 1;
const BOTTOM: u8 = 24;

/// A grid Snake. State is fully deterministic (RNG seeded from state, frames
/// counted) so it rewinds/replays/RLs correctly — the substrate contract.
pub struct Snake {
    body: Vec<(u8, u8)>,
    dir: Button,
    food: (u8, u8),
    rng: u32,
    tick: u8,
    alive: bool,
}

impl Default for Snake {
    fn default() -> Self {
        Self::new()
    }
}

impl Snake {
    pub fn new() -> Self {
        let mut s = Snake {
            body: vec![(8, 12), (7, 12), (6, 12)],
            dir: Button::Right,
            food: (0, 0),
            rng: 0x9E37_79B9,
            tick: 0,
            alive: true,
        };
        s.food = s.spawn();
        s
    }

    fn rnd(&mut self) -> u32 {
        // xorshift32 — deterministic, seeded from game state.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    fn spawn(&mut self) -> (u8, u8) {
        loop {
            let x = (self.rnd() % W as u32) as u8;
            let y = TOP + (self.rnd() % (BOTTOM - TOP) as u32) as u8;
            if !self.body.contains(&(x, y)) {
                return (x, y);
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
            let (hx, hy) = self.body[0];
            let head = match self.dir {
                Button::Up => (hx, hy.wrapping_sub(1)),
                Button::Down => (hx, hy + 1),
                Button::Left => (hx.wrapping_sub(1), hy),
                _ => (hx + 1, hy),
            };
            if head.0 >= W || head.1 < TOP || head.1 >= BOTTOM || self.body.contains(&head) {
                self.alive = false;
            } else {
                self.body.insert(0, head);
                if head == self.food {
                    self.food = self.spawn();
                } else {
                    self.body.pop();
                }
            }
        }
        if !self.alive && input.pressed(Button::Fire) {
            *self = Snake::new();
        }

        frame.clear(Colour::Black);
        frame.ink(Colour::BrightGreen);
        for &(x, y) in &self.body {
            frame.tile(&BLOCK, x, y);
        }
        frame.ink(Colour::BrightRed).tile(&BLOCK, self.food.0, self.food.1);
        frame.ink(Colour::White);
        frame.text(0, 0, &format!("SNAKE   LEN {}", self.body.len()));
        if !self.alive {
            frame.ink(Colour::BrightYellow);
            frame.text(8, 12, "GAME OVER - FIRE");
        }
    }
}
