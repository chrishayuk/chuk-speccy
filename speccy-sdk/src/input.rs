//! This frame's input: logical buttons and held/pressed edges.

/// Logical buttons (keyboard or joystick, pre-mapped). Bit values double as a
/// small bitset used internally.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Button {
    Up = 1,
    Down = 2,
    Left = 4,
    Right = 8,
    Fire = 16,
}

/// This frame's input: which buttons are held, and which became held this frame.
pub struct Input {
    pub(crate) cur: u8,
    pub(crate) prev: u8,
}

impl Input {
    /// Held right now.
    pub fn held(&self, b: Button) -> bool {
        self.cur & b as u8 != 0
    }
    /// Newly pressed this frame (rising edge).
    pub fn pressed(&self, b: Button) -> bool {
        self.cur & b as u8 != 0 && self.prev & b as u8 == 0
    }
    /// No input — for tests / headless stepping.
    pub fn none() -> Self {
        Input { cur: 0, prev: 0 }
    }
    /// Construct from a set of currently-held buttons (for testing a `Game`).
    pub fn held_now(buttons: &[Button]) -> Self {
        let mut cur = 0u8;
        for &b in buttons {
            cur |= b as u8;
        }
        Input { cur, prev: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_edges() {
        let down = Input {
            cur: Button::Fire as u8,
            prev: 0,
        };
        assert!(down.held(Button::Fire) && down.pressed(Button::Fire));
        let still = Input {
            cur: Button::Fire as u8,
            prev: Button::Fire as u8,
        };
        assert!(still.held(Button::Fire) && !still.pressed(Button::Fire));
    }
}
