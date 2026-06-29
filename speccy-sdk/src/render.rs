//! Render a host [`Game`](crate::Game) running to an animated GIF, headless — the
//! host-composite counterpart of [`run`](crate::run) (which renders a pure `.tap`).
//! Boots the ROM, installs the game over the trap ABI, captures indexed frames, and
//! encodes via [`display::gif`]. **Feature-free** (it never touches the compiler), so
//! any authored host game gets a shareable GIF in one call — the "see it run" payoff
//! of the fast host-iteration loop.
//!
//! ```no_run
//! # fn demo(rom: &[u8], game: impl speccy_sdk::Game + Send + 'static) {
//! let gif = speccy_sdk::render::render_gif(rom, game, 120, 2, 16);
//! std::fs::write("out.gif", gif).unwrap();
//! # }
//! ```

use crate::{boot, Game};

/// Sensible defaults (frames captured, emulated frames between captures, settle
/// frames after boot before the first capture).
pub const DEFAULT_FRAMES: usize = 120;
pub const DEFAULT_EVERY: usize = 2;
pub const DEFAULT_SETTLE: usize = 16;

/// Boot `rom`, install `game`, settle `settle` frames, then capture `frames` indexed
/// 256×192 screens — one every `every` emulated frames (a raw observation; the host
/// applies the palette). The same machine any head would pump, just headless.
pub fn capture_indexed(
    rom: &[u8],
    game: impl Game + Send + 'static,
    frames: usize,
    every: usize,
    settle: usize,
) -> Vec<Vec<u8>> {
    let mut spec = boot(rom, game); // ROM boot + install + runtime pump
    for _ in 0..settle {
        spec.run_frame(); // let the game draw its first real frames
    }
    let every = every.max(1);
    let mut out = Vec::with_capacity(frames);
    for _ in 0..frames {
        for _ in 0..every {
            spec.run_frame();
        }
        out.push(spec.screen_indexed());
    }
    out
}

/// Render a host `Game` running to an animated GIF (256×192, the authentic palette).
pub fn render_gif(
    rom: &[u8],
    game: impl Game + Send + 'static,
    frames: usize,
    every: usize,
    settle: usize,
) -> Vec<u8> {
    let screens = capture_indexed(rom, game, frames, every, settle);
    let delay_cs = (every.max(1) * 2) as u16; // 50 Hz → `every`*2 centiseconds/frame
    display::gif::encode_indexed_to_vec(&screens, 256, 192, &display::AUTHENTIC, delay_cs)
}
