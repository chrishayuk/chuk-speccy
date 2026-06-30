// "Reach the target" — a tiny reward-bearing game for the agent benchmark
// (docs/08 §9). Move the player blob onto the target; each hit scores a point and
// the target jumps to a new spot. Pure dialect (scalar u16 state only, so it
// compiles straight to a tape AND its fields land in the symbol map):
//
//   cargo run -p chuk-speccy-sdk --features compile --bin speccy-compile -- speccy-sdk/samples/reach.rs -o reach.tap
//   cargo run --release --bin speccy-gui -- testroms/48.rom reach.tap   # O/P/Q/A to move
//
// `score` is the reward signal an agent maximises; `chuk-speccy-env` reads it off
// the running tape via the emitted `reach.sym.toml`. Like `bounce`, it erases the
// old blobs rather than clearing the whole screen each frame — so `update` stays
// well under one frame and the game steps at 50 Hz (one move per held frame).

struct Reach {
    px: u16,
    py: u16,
    tx: u16,
    ty: u16,
    score: u16,
    seed: u16,
    started: u16,
}

impl Game for Reach {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if self.started == 0 {
            frame.clear(Colour::Black); // one-time
            self.px = 16;
            self.py = 12;
            self.tx = 5;
            self.ty = 5;
            self.seed = 1;
            self.started = 1;
        }

        // Erase the player blob at its current (last-drawn) cell.
        let ex = self.px * 8;
        let ey = self.py * 8;
        let mut r = 0;
        while r < 6 {
            let mut c = 0;
            while c < 6 {
                frame.pixel(ex + c, ey + r, false);
                c = c + 1;
            }
            r = r + 1;
        }

        // One cell per frame per held direction.
        if input.held(Button::Left) {
            if self.px > 0 {
                self.px = self.px - 1;
            }
        }
        if input.held(Button::Right) {
            if self.px < 31 {
                self.px = self.px + 1;
            }
        }
        if input.held(Button::Up) {
            if self.py > 0 {
                self.py = self.py - 1;
            }
        }
        if input.held(Button::Down) {
            if self.py < 23 {
                self.py = self.py + 1;
            }
        }

        // Within one cell of the target? Score, erase it, then jump it via a small
        // LCG. A ±1 range (not exact equality) keeps scoring robust to the coarse,
        // multi-frame update cadence (no overshoot-and-miss).
        let mut dx = 0;
        if self.px > self.tx {
            dx = self.px - self.tx;
        } else {
            dx = self.tx - self.px;
        }
        let mut dy = 0;
        if self.py > self.ty {
            dy = self.py - self.ty;
        } else {
            dy = self.ty - self.py;
        }
        if dx < 2 {
            if dy < 2 {
                let ox = self.tx * 8;
                let oy = self.ty * 8;
                let mut er = 0;
                while er < 6 {
                    let mut ec = 0;
                    while ec < 6 {
                        frame.pixel(ox + ec, oy + er, false);
                        ec = ec + 1;
                    }
                    er = er + 1;
                }
                self.score = self.score + 1;
                self.seed = self.seed * 75 + 74;
                self.tx = self.seed % 30 + 1;
                self.ty = self.seed % 20 + 1;
            }
        }

        // Draw the target blob.
        let tpx = self.tx * 8;
        let tpy = self.ty * 8;
        let mut tr = 0;
        while tr < 6 {
            let mut tc = 0;
            while tc < 6 {
                frame.pixel(tpx + tc, tpy + tr, true);
                tc = tc + 1;
            }
            tr = tr + 1;
        }

        // Draw the player blob at its new cell.
        let ppx = self.px * 8;
        let ppy = self.py * 8;
        let mut pr = 0;
        while pr < 6 {
            let mut pc = 0;
            while pc < 6 {
                frame.pixel(ppx + pc, ppy + pr, true);
                pc = pc + 1;
            }
            pr = pr + 1;
        }
    }
}
