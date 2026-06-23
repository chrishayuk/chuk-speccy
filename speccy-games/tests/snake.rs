//! End-to-end: boot Snake on a real Spectrum (runtime pump + GAME_TICK + the
//! Frame rasteriser) and confirm it draws to the screen each frame.
//! `SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-games -- --ignored`

use speccy_games::Snake;

#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn snake_renders_on_the_machine() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("set SPECTRUM_ROM")).unwrap();
    let mut spec = speccy_sdk::boot(&rom, Snake::new());

    for _ in 0..12 {
        spec.run_frame(); // each frame: HALT → GAME_TICK → Snake draws the screen
    }

    let screen = spec.read_memory(0x4000, 6144);
    let set = screen.iter().filter(|&&b| b != 0).count();
    assert!(set > 50, "the game should be drawing pixels, set bytes = {set}");

    let text = spec.screen_text();
    assert!(text.contains("SNAKE"), "expected the score line on screen, got:\n{text}");
    assert!(text.contains("LEN 3"), "expected initial length 3, got:\n{text}");
}

#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn snake_runs_long_without_overflow() {
    // With no input the snake hits a wall (~140 frames) and dies; keep running
    // well past death so a runaway frame counter would have panicked (regression
    // for the dead-state tick overflow).
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("set SPECTRUM_ROM")).unwrap();
    let mut spec = speccy_sdk::boot(&rom, Snake::new());
    for _ in 0..600 {
        spec.run_frame();
    }
    assert!(spec.screen_text().contains("SNAKE"), "still drawing after a long run");
}
