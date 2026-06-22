// A *playable* dial game: move a blob with the keys. ONE source, two compilers.
//
//   rustz80 (pure):  speccy-compile rustz80/samples/move.rs -o move.tap
//                    speccy-gui testroms/48.rom move.tap
//   rustc  (host):   an ordinary speccy-sdk Game (see rustz80/tests/dial.rs).
//
// Controls: cursor keys 5/6/7/8 or Q/A/O/P. The blob slides; it stays on screen.

#[derive(Default)]
struct Mover {
    x: u8,
    y: u8,
    started: u8,
}

impl Game for Mover {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if self.started == 0u8 {
            frame.clear(Colour::Blue);
            self.x = 120u8;
            self.y = 88u8;
            self.started = 1u8;
        }

        // Erase the old blob.
        let mut r = 0u8;
        while r < 8u8 {
            let mut c = 0u8;
            while c < 8u8 {
                frame.pixel(self.x + c, self.y + r, false);
                c = c + 1u8;
            }
            r = r + 1u8;
        }

        // Move on held keys, clamped to the screen.
        if input.held(Button::Left) {
            if self.x > 2u8 {
                self.x = self.x - 2u8;
            }
        }
        if input.held(Button::Right) {
            if self.x < 246u8 {
                self.x = self.x + 2u8;
            }
        }
        if input.held(Button::Up) {
            if self.y > 2u8 {
                self.y = self.y - 2u8;
            }
        }
        if input.held(Button::Down) {
            if self.y < 182u8 {
                self.y = self.y + 2u8;
            }
        }

        // Draw the blob at the new position.
        let mut r2 = 0u8;
        while r2 < 8u8 {
            let mut c2 = 0u8;
            while c2 < 8u8 {
                frame.pixel(self.x + c2, self.y + r2, true);
                c2 = c2 + 1u8;
            }
            r2 = r2 + 1u8;
        }
    }
}
