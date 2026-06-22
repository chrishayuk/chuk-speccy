//! The fidelity dial, closed end to end: `samples/bounce.rs` is compiled **both**
//! by `rustc` (as a `speccy-sdk` `Game`) and by `rustz80` (to a bootable tape) —
//! one source, two compilers.

const SAMPLE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/samples/bounce.rs"));

/// Host side: the same sample text, compiled here by `rustc` against `speccy-sdk`.
/// If it type-checks as a `Game`, this test passes (a compile-time assertion).
#[test]
fn host_game_is_valid_rust() {
    // The sample uses the dialect's long form (`x = x + 1`, no `+=`) so the same
    // text compiles under rustz80 too — silence clippy's host-only suggestion.
    #[allow(clippy::assign_op_pattern)]
    mod game {
        use speccy_sdk::*;
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/samples/bounce.rs"));
        pub fn assert_is_game() {
            fn is_game<T: Game + Default>() {}
            is_game::<Bounce>();
        }
    }
    game::assert_is_game();
}

/// Pure side: the same text compiles through `rustz80` to a `.tap`.
#[test]
fn game_compiles_pure() {
    assert!(rustz80::has_game(SAMPLE), "should be recognised as a Game");
    rustz80::compile_game(SAMPLE, "BOUNCE").expect("compiles to a tap");
}

/// Pure side, end to end: boot the compiled game on the real ROM and confirm it
/// draws and animates (the bouncing pixel moves).
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p rustz80 --test dial -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn game_boots_and_animates() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let tap = rustz80::compile_game(SAMPLE, "BOUNCE").expect("compile");

    let mut spec = spectrum::Spectrum::new_48k(&rom);
    for _ in 0..250 {
        spec.run_frame();
    }
    spec.load_tap(&tap).unwrap();
    spec.autoload_tape();

    let lit = |s: &spectrum::Spectrum| -> u32 {
        s.read_memory(0x4000, 0x1800).iter().map(|b| b.count_ones()).sum()
    };
    let hash = |s: &spectrum::Spectrum| -> u64 {
        s.read_memory(0x4000, 0x1800)
            .iter()
            .enumerate()
            .fold(0u64, |a, (i, &b)| a.wrapping_add((b as u64 + 1).wrapping_mul(i as u64 + 1)))
    };

    for _ in 0..600 {
        spec.run_frame();
    }
    let (lit_a, hash_a) = (lit(&spec), hash(&spec));
    for _ in 0..600 {
        spec.run_frame();
    }
    let hash_b = hash(&spec);

    assert!(lit_a > 0, "the bouncing pixel should be drawn");
    assert!(lit_a < 100, "only the single pixel should be lit, not loader junk");
    assert_ne!(hash_a, hash_b, "the pixel should be moving (screen changes)");
}
