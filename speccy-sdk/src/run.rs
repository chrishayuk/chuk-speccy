//! `speccy run` core: compile a dialect game and render it *running* to an animated
//! GIF — the one-command "source → see it run" path (L0). Behind the `compile`
//! feature (it can invoke the compiler). Built from the headless [`spectrum`] machine
//! and [`display::gif`], so it never reaches into the windowed frontend.

use crate::compile;

/// Default render knobs (frames captured, emulated frames between captures, warm-up).
pub const DEFAULT_FRAMES: usize = 120;
pub const DEFAULT_EVERY: usize = 2;
pub const DEFAULT_BOOT: usize = 420;

/// Compile a dialect game `src` to a bootable `.tap`. An `impl Game` goes through the
/// SDK game flow; anything else needs a no-arg `fn main` (the generic compiler path) —
/// the same dispatch as `speccy-compile`.
pub fn compile_source(src: &str, name: &str) -> Result<Vec<u8>, String> {
    if compile::has_game(src) {
        compile::compile_game(src, name)
    } else {
        rustz80::compile_to_tap(src, "main", name)
    }
}

/// Boot `tap` on `rom` and capture `frames` indexed 256×192 screens, one every `every`
/// emulated frames, after `boot` warm-up frames (ROM boot + tape autoload + settle) —
/// the same boot sequence the dial ROM tests use.
pub fn capture_indexed(
    tap: &[u8],
    rom: &[u8],
    frames: usize,
    every: usize,
    boot: usize,
) -> Result<Vec<Vec<u8>>, String> {
    let mut spec = spectrum::Spectrum::new_48k(rom);
    // Boot the ROM + trap-load + auto-run the tape (the core's BOOT_FRAMES + LOAD "").
    spec.load_media(spectrum::format::TAP, tap)
        .map_err(|e| format!("load tape: {e:?}"))?;
    for _ in 0..boot {
        spec.run_frame(); // settle into the frame loop
    }

    let every = every.max(1);
    let mut out = Vec::with_capacity(frames);
    for _ in 0..frames {
        for _ in 0..every {
            spec.run_frame();
        }
        out.push(spec.screen_indexed());
    }
    Ok(out)
}

/// Render `tap` running to an animated GIF (256×192, the authentic palette).
pub fn render_gif(
    tap: &[u8],
    rom: &[u8],
    frames: usize,
    every: usize,
    boot: usize,
) -> Result<Vec<u8>, String> {
    let screens = capture_indexed(tap, rom, frames, every, boot)?;
    let delay_cs = (every.max(1) * 2) as u16; // 50 Hz → `every`*2 centiseconds/frame
    Ok(display::gif::encode_indexed_to_vec(
        &screens,
        256,
        192,
        &display::AUTHENTIC,
        delay_cs,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLANK: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/samples/blank.rs"));

    #[test]
    fn compile_source_accepts_an_impl_game() {
        assert!(compile_source(BLANK, "BLANK").is_ok());
    }

    /// End to end on the real ROM: compile the blank starter, render it running, and
    /// confirm the GIF shows the drawn blob.
    ///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-sdk --features compile run:: -- --ignored
    #[test]
    #[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
    fn renders_a_running_game_to_a_gif() {
        let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
        let tap = compile_source(BLANK, "BLANK").unwrap();

        let screens = capture_indexed(&tap, &rom, 30, 2, DEFAULT_BOOT).unwrap();
        assert_eq!(screens.len(), 30);
        assert!(
            screens.last().unwrap().iter().any(|&px| px != 0),
            "the blob should be drawn on screen"
        );

        let gif = render_gif(&tap, &rom, 30, 2, DEFAULT_BOOT).unwrap();
        assert_eq!(&gif[..3], b"GIF", "a real GIF was produced");
        assert!(gif.len() > 100, "non-trivial gif ({} bytes)", gif.len());
    }
}
