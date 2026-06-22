//! First light: **Snake** in ~90 lines of [`Game`] — the canonical SDK demo, now
//! built on the subset-clean primitives ([`Entities`] instead of `Vec`, [`Rng`],
//! [`Cell`]) and exposing the env surface (`reset`/`reward`/`done`). Run it in the
//! native window with `speccy-gui <rom> snake`.

use crate::{Button, Cell, Colour, Entities, Frame, Game, Input, Rng, BLOCK};

const W: u8 = 32; // playfield is the full 32×24 grid; row 0 shows the score
const TOP: u8 = 1;
const BOTTOM: u8 = 24;
const MAX: usize = 768; // 32 × 24 cells — the playfield ceiling, so the body never overflows
const DEFAULT_SEED: u32 = 0x1234_5678;

/// A grid Snake. State is fully deterministic (RNG seeded from state, frames
/// counted) so it rewinds/replays/RLs correctly — the substrate contract.
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
            let x = self.rng.below(W as u32) as u8;
            let y = TOP + self.rng.below((BOTTOM - TOP) as u32) as u8;
            let c = Cell::new(x, y);
            if !self.body.contains(&c) {
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
        frame.ink(Colour::BrightGreen);
        for i in 0..self.body.len() {
            let c = self.body[i];
            frame.tile(&BLOCK, c.x, c.y);
        }
        frame.ink(Colour::BrightRed).tile(&BLOCK, self.food.x, self.food.y);
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
        let input = Input { cur: 0, prev: 0 };
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
}
