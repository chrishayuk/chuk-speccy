// The fidelity dial, closed: ONE source, two compilers.
//
//   rustc  (host): an ordinary `speccy-sdk` Game — see rustz80/tests/dial.rs,
//                  which `include!`s this file with `use speccy_sdk::*;`.
//   rustz80 (pure): compile it straight to a bootable tape —
//      cargo run -p chuk-speccy-sdk --features compile --bin speccy-compile -- speccy-sdk/samples/bounce.rs -o bounce.tap
//      cargo run --release --bin speccy-gui -- testroms/48.rom bounce.tap
//
// A single pixel bounces around the screen. Self-playing (ignores input). No
// `use`/`fn main` here so the same text compiles both ways; the host test adds the
// imports, `speccy-compile` supplies the prelude + frame loop.

#[derive(Default)]
struct Bounce {
    x: u8,
    y: u8,
    dx: u8, // 0 = moving right/down, 1 = left/up
    dy: u8,
    started: u8,
}

impl Game for Bounce {
    fn update(&mut self, _input: &Input, frame: &mut Frame) {
        if self.started == 0 {
            frame.clear(Colour::Black);
            self.x = 120;
            self.y = 88;
            self.started = 1;
        }

        // Erase the 6x6 blob at the old position.
        let mut r = 0;
        while r < 6 {
            let mut c = 0;
            while c < 6 {
                frame.pixel(self.x + c, self.y + r, false);
                c = c + 1;
            }
            r = r + 1;
        }

        if self.dx == 0 {
            self.x = self.x + 1;
            if self.x >= 240 {
                self.dx = 1;
            }
        } else {
            self.x = self.x - 1;
            if self.x <= 4 {
                self.dx = 0;
            }
        }
        if self.dy == 0 {
            self.y = self.y + 1;
            if self.y >= 178 {
                self.dy = 1;
            }
        } else {
            self.y = self.y - 1;
            if self.y <= 4 {
                self.dy = 0;
            }
        }

        // Draw the blob at the new position.
        let mut r2 = 0;
        while r2 < 6 {
            let mut c2 = 0;
            while c2 < 6 {
                frame.pixel(self.x + c2, self.y + r2, true);
                c2 = c2 + 1;
            }
            r2 = r2 + 1;
        }
    }
}
