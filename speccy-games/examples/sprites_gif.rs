//! Render the **baked-art `sprites` demo** to an animated GIF — the asset pipeline,
//! end to end and headless: a PNG was baked into `const Tile`s (see `Sprites` /
//! `SHOWCASE` in the lib), and here those tiles run on a real 48K and are captured to
//! a GIF you can open.
//!
//! ```text
//! cargo run -p chuk-speccy-games --example sprites_gif -- testroms/48.rom sprites.gif
//! ```

use speccy_games::Sprites;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(rom_path) = args.next() else {
        eprintln!("usage: sprites_gif <48.rom> [out.gif] [frames]");
        return ExitCode::FAILURE;
    };
    let out = args.next().unwrap_or_else(|| "sprites.gif".to_string());
    let frames = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(speccy_sdk::render::DEFAULT_FRAMES);

    let rom = match std::fs::read(&rom_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: cannot read ROM {rom_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let gif = speccy_sdk::render::render_gif(
        &rom,
        Sprites::default(),
        frames,
        speccy_sdk::render::DEFAULT_EVERY,
        speccy_sdk::render::DEFAULT_SETTLE,
    );
    if let Err(e) = std::fs::write(&out, &gif) {
        eprintln!("error: cannot write {out}: {e}");
        return ExitCode::FAILURE;
    }
    println!("wrote {out} ({} bytes, {frames} frames)", gif.len());
    ExitCode::SUCCESS
}
