#![cfg(feature = "compile")]

//! The fidelity dial, closed end to end: `samples/bounce.rs` and `samples/move.rs`
//! are each compiled **both** by `rustc` (as `speccy-sdk` `Game`s) and by `rustz80`
//! (to bootable tapes) — one source, two compilers.

const BOUNCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/samples/bounce.rs"
));
const MOVE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/samples/move.rs"
));
const SNAKE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/samples/snake_game.rs"
));
const BLANK: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/samples/blank.rs"
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
            "/samples/bounce.rs"
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
            "/samples/move.rs"
        ));
        pub fn check() {
            fn is_game<T: Game + Default>() {}
            is_game::<Mover>();
        }
    }
    #[allow(clippy::assign_op_pattern, clippy::collapsible_if)]
    mod snake {
        use speccy_sdk::*;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/samples/snake_game.rs"
        ));
        pub fn check() {
            fn is_game<T: Game + Default>() {}
            is_game::<Snake>();
        }
    }
    #[allow(clippy::assign_op_pattern, clippy::collapsible_if)]
    mod blank {
        use speccy_sdk::*;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/samples/blank.rs"
        ));
        pub fn check() {
            fn is_game<T: Game + Default>() {}
            is_game::<Starter>();
        }
    }
    bounce::check();
    mover::check();
    snake::check();
    blank::check();
}

/// Pure side: the same texts compile through `rustz80` to `.tap`s.
#[test]
fn games_compile_pure() {
    assert!(speccy_sdk::compile::has_game(BOUNCE) && speccy_sdk::compile::has_game(MOVE));
    assert!(speccy_sdk::compile::has_game(SNAKE) && speccy_sdk::compile::has_game(BLANK));
    speccy_sdk::compile::compile_game(BOUNCE, "BOUNCE").expect("bounce compiles");
    speccy_sdk::compile::compile_game(MOVE, "MOVE").expect("move compiles");
    speccy_sdk::compile::compile_game(SNAKE, "SNAKE").expect("snake compiles");
    speccy_sdk::compile::compile_game(BLANK, "BLANK").expect("blank compiles");
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

/// Pure, end to end: the `impl Game` Snake boots from tape, **draws + animates** on
/// the real ROM, and its **typed state reads back off the tape** via the emitted
/// symbol map (`len`, `food_x`) — the seam, on a real game.
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn snake_game_boots_animates_and_reads_back() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, sym) =
        speccy_sdk::compile::compile_game_with_symbols(SNAKE, "SNAKE").expect("compile");
    let mut spec = boot(&rom, &tap);

    // Hash the 32×24 grid of filled cells to detect animation.
    let cell_hash = |s: &spectrum::Spectrum| -> u64 {
        let mut h = 0u64;
        for cy in 0..24u16 {
            for cx in 0..32u16 {
                let py = cy * 8;
                let a = 0x4000 + ((py & 0xC0) << 5) + ((py & 0x38) << 2) + cx;
                let lit = (s.read_memory(a, 1)[0] == 0xFF) as u64;
                h = h.wrapping_mul(0x100000001B3).wrapping_add(lit + 1);
            }
        }
        h
    };

    for _ in 0..200 {
        spec.run_frame();
    }
    let lit = (0x4000u16..0x5800)
        .filter(|&p| spec.read_memory(p, 1)[0] == 0xFF)
        .count();
    assert!(lit > 0, "the snake should draw filled cells");

    // Sample across a long window: with no input the snake crawls into the wall, dies,
    // and auto-restarts — so it visits many distinct frames (robust to the freeze).
    let mut frames = std::collections::HashSet::new();
    for _ in 0..30 {
        for _ in 0..25 {
            spec.run_frame();
        }
        frames.insert(cell_hash(&spec));
    }
    assert!(
        frames.len() > 3,
        "the snake should animate (move + restart), distinct frames = {}",
        frames.len()
    );

    // The seam: read the game's typed fields straight off the tape's RAM.
    let read_u16 = |s: &spectrum::Spectrum, field: &str| -> u16 {
        let addr = sym.addr_of(field).expect("field in the symbol map");
        let m = s.read_memory(addr, 2);
        u16::from_le_bytes([m[0], m[1]])
    };
    assert_eq!(read_u16(&spec, "len"), 3, "initial body length read off the tape");
    assert_eq!(read_u16(&spec, "food_x"), 16, "initial food x read off the tape");
}

/// `Frame::fill_cell` (host Rust) and `__frame_fill_cell` (the dialect prelude) are two
/// implementations of the same draw — guard them against drift on real hardware: a
/// pure tape that fills one cell must produce the **same attribute byte** the host
/// `Frame` would (the bright/ink encoding is where a divergence would hide).
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn fill_cell_host_and_pure_agree() {
    use speccy_sdk::{Attr, Colour};
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let src = r#"
#[derive(Default)]
struct Dot { started: u16 }
impl Game for Dot {
    fn update(&mut self, _i: &Input, frame: &mut Frame) {
        if self.started == 0u16 { frame.clear(Colour::Black); self.started = 1u16; }
        frame.fill_cell(5u8, 6u8, Colour::BrightRed);
    }
}
"#;
    let tap = speccy_sdk::compile::compile_game(src, "DOT").expect("compile");
    let mut spec = boot(&rom, &tap);
    for _ in 0..50 {
        spec.run_frame();
    }

    // Attribute area is linear (0x5800 + cy*32 + cx) — no interleave needed.
    let attr_pure = spec.read_memory(0x5800 + 6 * 32 + 5, 1)[0];
    let attr_host = Attr::ink(Colour::BrightRed).0;
    assert_eq!(attr_host, 0x42, "bright red ink on black = bright<<6 | ink(2)");
    assert_eq!(
        attr_pure, attr_host,
        "host & pure fill_cell must encode the same attribute"
    );
}
