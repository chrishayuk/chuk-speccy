#![cfg(feature = "compile")]

//! The fidelity dial, closed end to end: `samples/bounce.rs` and `samples/move.rs`
//! are each compiled **both** by `rustc` (as `speccy-sdk` `Game`s) and by `rustz80`
//! (to bootable tapes) — one source, two compilers.

const BOUNCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../rustz80/samples/bounce.rs"
));
const MOVE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../rustz80/samples/move.rs"
));

/// Host side: the same sample texts, compiled here by `rustc` against `speccy-sdk`.
/// If they type-check as `Game`s, this passes (a compile-time assertion).
#[test]
fn host_games_are_valid_rust() {
    // The samples use the dialect's long form (`x = x + 1`, no `+=`) so the same
    // text compiles under rustz80 too — silence clippy's host-only suggestion.
    #[allow(clippy::assign_op_pattern, clippy::collapsible_if)]
    mod bounce {
        use speccy_sdk::*;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../rustz80/samples/bounce.rs"
        ));
        pub fn check() {
            fn is_game<T: Game + Default>() {}
            is_game::<Bounce>();
        }
    }
    #[allow(clippy::assign_op_pattern, clippy::collapsible_if)]
    mod mover {
        use speccy_sdk::*;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../rustz80/samples/move.rs"
        ));
        pub fn check() {
            fn is_game<T: Game + Default>() {}
            is_game::<Mover>();
        }
    }
    bounce::check();
    mover::check();
}

/// Pure side: the same texts compile through `rustz80` to `.tap`s.
#[test]
fn games_compile_pure() {
    assert!(speccy_sdk::compile::has_game(BOUNCE) && speccy_sdk::compile::has_game(MOVE));
    speccy_sdk::compile::compile_game(BOUNCE, "BOUNCE").expect("bounce compiles");
    speccy_sdk::compile::compile_game(MOVE, "MOVE").expect("move compiles");
}

fn boot(rom: &[u8], tap: &[u8]) -> spectrum::Spectrum {
    let mut spec = spectrum::Spectrum::new_48k(rom);
    for _ in 0..250 {
        spec.run_frame();
    }
    spec.load_tap(tap).unwrap();
    spec.autoload_tape();
    for _ in 0..400 {
        spec.run_frame();
    }
    spec
}

/// Pure, end to end: boot the bouncing-blob game and confirm it's **visible**
/// (white ink on black paper — the bug that made the first cut invisible) and
/// **animating**.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p rustz80 --test dial -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn bounce_boots_visible_and_animates() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let mut spec = boot(
        &rom,
        &speccy_sdk::compile::compile_game(BOUNCE, "BOUNCE").expect("compile"),
    );

    // clear(Black) must set white ink on black paper, else pixels are invisible.
    assert_eq!(
        spec.read_memory(0x5800, 1)[0],
        0x07,
        "attrs = white ink on black"
    );

    let bitmap = |s: &spectrum::Spectrum| s.read_memory(0x4000, 0x1800);
    // Sample across frames: the blob's update overruns a frame, so a single
    // snapshot can catch it mid-draw — track the fullest blob and distinct frames.
    let mut max_lit = 0u32;
    let mut frames = std::collections::HashSet::new();
    for _ in 0..400 {
        spec.run_frame();
        let b = bitmap(&spec);
        max_lit = max_lit.max(b.iter().map(|x| x.count_ones()).sum());
        frames.insert(b);
    }
    assert!(
        max_lit >= 24,
        "the 6x6 blob should be fully drawn at some frame, max {max_lit}"
    );
    assert!(
        frames.len() > 3,
        "the blob should be moving (distinct frames)"
    );
}

/// Pure, end to end: the *playable* game reads the keyboard — holding a key moves
/// the blob (the new `inport` intrinsic + the `Input` prelude).
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn move_responds_to_keys() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, sym) = speccy_sdk::compile::compile_game_with_symbols(MOVE, "MOVE").expect("compile");
    let mut spec = boot(&rom, &tap);

    // Read Mover's `x` field at the address the compiler emitted in the symbol map.
    let x_addr = sym.addr_of("x").expect("x in the symbol map");
    let read_x = |s: &spectrum::Spectrum| -> u16 {
        let m = s.read_memory(x_addr, 2);
        u16::from_le_bytes([m[0], m[1]])
    };
    let x0 = read_x(&spec);

    // Hold "P" (mapped to Right) and let it run.
    let p = spectrum::keyboard::key_for_char('p').unwrap().0;
    spec.set_key(p, true);
    for _ in 0..120 {
        spec.run_frame();
    }
    let x_right = read_x(&spec);
    spec.set_key(p, false);

    assert!(
        x_right > x0,
        "holding Right should grow x: {x0} -> {x_right}"
    );

    // Now hold "O" (Left) and confirm it comes back.
    let o = spectrum::keyboard::key_for_char('o').unwrap().0;
    spec.set_key(o, true);
    for _ in 0..120 {
        spec.run_frame();
    }
    let x_left = read_x(&spec);
    spec.set_key(o, false);

    assert!(
        x_left < x_right,
        "holding Left should shrink x: {x_right} -> {x_left}"
    );
}
