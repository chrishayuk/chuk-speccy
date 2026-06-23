// "Reach the target" — a tiny reward-bearing game for the agent benchmark
// (docs/08 §9). Move the player blob onto the target; each hit scores a point and
// the target jumps to a new spot. Pure dialect (scalar u16 state only, so it
// compiles straight to a tape AND its fields land in the symbol map):
//
//   cargo run -p rustz80 --bin speccy-compile -- rustz80/samples/reach.rs -o reach.tap
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
        if self.started == 0u16 {
            frame.clear(Colour::Black); // one-time
            self.px = 16u16;
            self.py = 12u16;
            self.tx = 5u16;
            self.ty = 5u16;
            self.seed = 1u16;
            self.started = 1u16;
        }

        // Erase the player blob at its current (last-drawn) cell.
        let ex = self.px * 8u16;
        let ey = self.py * 8u16;
        let mut r = 0u16;
        while r < 6u16 {
            let mut c = 0u16;
            while c < 6u16 {
                frame.pixel(ex + c, ey + r, false);
                c = c + 1u16;
            }
            r = r + 1u16;
        }

        // One cell per frame per held direction.
        if input.held(Button::Left) {
            if self.px > 0u16 {
                self.px = self.px - 1u16;
            }
        }
        if input.held(Button::Right) {
            if self.px < 31u16 {
                self.px = self.px + 1u16;
            }
        }
        if input.held(Button::Up) {
            if self.py > 0u16 {
                self.py = self.py - 1u16;
            }
        }
        if input.held(Button::Down) {
            if self.py < 23u16 {
                self.py = self.py + 1u16;
            }
        }

        // Within one cell of the target? Score, erase it, then jump it via a small
        // LCG. A ±1 range (not exact equality) keeps scoring robust to the coarse,
        // multi-frame update cadence (no overshoot-and-miss).
        let mut dx = 0u16;
        if self.px > self.tx {
            dx = self.px - self.tx;
        } else {
            dx = self.tx - self.px;
        }
        let mut dy = 0u16;
        if self.py > self.ty {
            dy = self.py - self.ty;
        } else {
            dy = self.ty - self.py;
        }
        if dx < 2u16 {
            if dy < 2u16 {
                let ox = self.tx * 8u16;
                let oy = self.ty * 8u16;
                let mut er = 0u16;
                while er < 6u16 {
                    let mut ec = 0u16;
                    while ec < 6u16 {
                        frame.pixel(ox + ec, oy + er, false);
                        ec = ec + 1u16;
                    }
                    er = er + 1u16;
                }
                self.score = self.score + 1u16;
                self.seed = self.seed * 75u16 + 74u16;
                self.tx = self.seed % 30u16 + 1u16;
                self.ty = self.seed % 20u16 + 1u16;
            }
        }

        // Draw the target blob.
        let tpx = self.tx * 8u16;
        let tpy = self.ty * 8u16;
        let mut tr = 0u16;
        while tr < 6u16 {
            let mut tc = 0u16;
            while tc < 6u16 {
                frame.pixel(tpx + tc, tpy + tr, true);
                tc = tc + 1u16;
            }
            tr = tr + 1u16;
        }

        // Draw the player blob at its new cell.
        let ppx = self.px * 8u16;
        let ppy = self.py * 8u16;
        let mut pr = 0u16;
        while pr < 6u16 {
            let mut pc = 0u16;
            while pc < 6u16 {
                frame.pixel(ppx + pc, ppy + pr, true);
                pc = pc + 1u16;
            }
            pr = pr + 1u16;
        }
    }
}
