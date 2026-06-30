// A blank starter game — ONE source, two compilers (the `speccy new` default).
//
//   host:  an ordinary `speccy-sdk` Game (see tests/dial.rs).
//   pure:  speccy-compile speccy-sdk/samples/blank.rs  ->  a bootable .tap
//          speccy-gui testroms/48.rom blank.tap
//
// Move a coloured blob around the 32×24 grid with the cursor keys (or QAOP); it
// leaves no trail. This is the minimal shape that still crosses the fidelity dial:
// no `use`/`fn main`, long-form ops, `u16` state, and the data-free `fill_cell` /
// `clear_cell` draw primitives. Edit me into your game.

#[derive(Default)]
struct Starter {
    started: u16,
    x: u16,
    y: u16,
}

impl Game for Starter {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if self.started == 0 {
            frame.clear(Colour::Black);
            self.x = 16;
            self.y = 12;
            self.started = 1;
        }

        // Erase the blob at its current cell (so it leaves no trail).
        frame.clear_cell(self.x as u8, self.y as u8);

        // Move on held keys, clamped to the grid.
        if input.held(Button::Left) {
            if self.x > 0 {
                self.x = self.x - 1;
            }
        }
        if input.held(Button::Right) {
            if self.x < 31 {
                self.x = self.x + 1;
            }
        }
        if input.held(Button::Up) {
            if self.y > 0 {
                self.y = self.y - 1;
            }
        }
        if input.held(Button::Down) {
            if self.y < 23 {
                self.y = self.y + 1;
            }
        }

        // Draw the blob at its new cell.
        frame.fill_cell(self.x as u8, self.y as u8, Colour::BrightCyan);
    }
}
