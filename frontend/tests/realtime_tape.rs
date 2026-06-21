//! End-to-end real-time tape loading: fetch a standard `.tap` from World of
//! Spectrum, drive it as a signal (no ROM trap), and confirm the ROM loader
//! reads the edges and the game appears. Network + slow → `#[ignore]`.
//! Run with: `SPECTRUM_ROM=testroms/48.rom cargo test -p frontend --test realtime_tape -- --ignored --nocapture`

use spectrum::Spectrum;

#[test]
#[ignore]
fn realtime_loads_a_standard_tap() {
    let rom_path = std::env::var("SPECTRUM_ROM").expect("set SPECTRUM_ROM");
    let rom = std::fs::read(rom_path).expect("read ROM");

    let game = wos::fetch("Jet Set Willy").expect("fetch a game");
    assert_eq!(game.format, "tap", "expected a standard tape");

    let mut spec = Spectrum::new_48k(&rom);
    for _ in 0..250 {
        spec.run_frame(); // boot to the BASIC prompt
    }
    spec.autoload_tape(); // type LOAD ""
    spec.play_tape("tap", &game.data).expect("start the signal");

    // Run real-time until the tape finishes (capped), then let it settle.
    let mut frames = 0;
    while spec.tape_playing() && frames < 80_000 {
        spec.run_frame();
        frames += 1;
    }
    for _ in 0..200 {
        spec.run_frame();
    }

    let idx = spec.screen_indexed();
    let fill = idx.iter().filter(|&&b| b != 0).count();
    println!("ran {frames} frames; screen fill {fill}/{}", idx.len());
    assert!(fill > 1000, "the loaded game should fill the screen (fill={fill})");
}
