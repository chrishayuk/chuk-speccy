//! End-to-end: boot the baked-art `sprites` demo on a real Spectrum and confirm the
//! baked tiles render and a GIF can be produced.
//! `SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-games -- --ignored`

use speccy_games::Sprites;

#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn sprites_demo_renders_baked_tiles() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("set SPECTRUM_ROM")).unwrap();
    let mut spec = speccy_sdk::boot(&rom, Sprites::default());
    for _ in 0..12 {
        spec.run_frame();
    }

    let screen = spec.read_memory(0x4000, 6144);
    let set = screen.iter().filter(|&&b| b != 0).count();
    assert!(set > 50, "baked tiles should be drawn, set bytes = {set}");
    assert!(
        spec.screen_text().contains("BAKED SPRITES"),
        "expected the HUD label, got:\n{}",
        spec.screen_text()
    );
}

#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn sprites_demo_renders_to_a_gif() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("set SPECTRUM_ROM")).unwrap();
    let gif = speccy_sdk::render::render_gif(&rom, Sprites::default(), 24, 2, 16);
    assert_eq!(&gif[..3], b"GIF", "a real GIF was produced");
    assert!(gif.len() > 100, "non-trivial gif ({} bytes)", gif.len());
}
