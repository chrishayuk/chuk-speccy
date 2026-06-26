//! Cross-component integration: compile a dialect program with `rustz80` (now the standalone
//! `cell80` crate) to a `.tap` and **boot it on the real 48K ROM** via `spectrum`, confirming
//! the compiled output runs on the full machine. Lives here (not in `rustz80`) because it
//! needs the Spectrum emulator + ROM, which stay in chuk-speccy. Needs the `compile` feature
//! (which pulls in `rustz80`) and `SPECTRUM_ROM`:
//!   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-sdk --features compile --test tap_boot -- --ignored
#![cfg(feature = "compile")]

/// Boot a dialect program from tape on the real ROM and confirm it executed.
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn boots_on_real_rom() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();

    // main(): write a 0xADDE sentinel above RAMTOP and set the top-left screen byte.
    let src = "
        fn main() {
            poke(40704u16, 222u8);   // 0x9F00 = 0xDE
            poke(40705u16, 173u8);   // 0x9F01 = 0xAD
            poke(16384u16, 255u8);   // top-left 8 pixels
        }
    ";
    let tap = rustz80::compile_to_tap(src, "main", "GAME").expect("compile");

    let mut spec = spectrum::Spectrum::new_48k(&rom);
    for _ in 0..250 {
        spec.run_frame(); // boot to the K cursor
    }
    spec.load_tap(&tap).unwrap();
    spec.autoload_tape(); // types LOAD ""
    for _ in 0..400 {
        spec.run_frame(); // trap-load BASIC + CODE, auto-run, USR main
    }

    let sentinel = spec.read_memory(0x9F00, 2);
    assert_eq!(
        sentinel,
        vec![222, 173],
        "main() ran from tape and wrote its sentinel"
    );
    assert_eq!(
        spec.read_memory(0x4000, 1)[0],
        0xFF,
        "top-left screen byte poked"
    );
}

/// Boot `fixtures/snake.rs` from tape on the real ROM and confirm the snake both draws and
/// *animates* — i.e. the compiled, interrupt-disabled game runs its loop correctly on the
/// full machine (this fails if the entry forgets to `DI`, since the ROM interrupt clobbers
/// `BC`/`DE` mid-arithmetic).
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-sdk --features compile --test tap_boot -- --ignored snake
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn snake_sample_animates() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/samples/snake.rs"))
            .unwrap();
    let tap = rustz80::compile_to_tap(&src, "main", "SNAKE").expect("compile");

    let mut spec = spectrum::Spectrum::new_48k(&rom);
    for _ in 0..250 {
        spec.run_frame();
    }
    spec.load_tap(&tap).unwrap();
    spec.autoload_tape();

    // Hash which of the 32x24 cells are filled (each segment is a filled 8x8 cell).
    let cell_hash = |spec: &spectrum::Spectrum| -> u64 {
        let mut h = 0u64;
        for cy in 0..24u16 {
            for cx in 0..32u16 {
                let py = cy * 8;
                let a = 0x4000 + ((py & 0xC0) << 5) + ((py & 0x38) << 2) + cx;
                let lit = (spec.read_memory(a, 1)[0] == 0xFF) as u64;
                h = h.wrapping_mul(0x100000001B3).wrapping_add(lit + 1);
            }
        }
        h
    };

    for _ in 0..400 {
        spec.run_frame();
    }
    let lit_a = (0x4000u16..0x5800)
        .filter(|&p| spec.read_memory(p, 1)[0] == 0xFF)
        .count();
    let a = cell_hash(&spec);
    for _ in 0..1200 {
        spec.run_frame();
    }
    let b = cell_hash(&spec);

    assert!(lit_a > 0, "the snake should have drawn filled cells");
    assert_ne!(a, b, "the snake should be animating, not frozen");
}
