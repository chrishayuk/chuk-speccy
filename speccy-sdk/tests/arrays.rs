#![cfg(feature = "compile")]

//! Array struct fields: a `[u16; N]` field lives inline in the game state, gets a
//! proper symbol-map entry (with `count`), and reads/writes correctly on hardware.
//! This is what lets a game hold its own grid/level instead of poking raw memory.

const ARR_GAME: &str = r#"
struct Arr {
    sum: u16,
    cells: [u16; 8],
    started: u16,
}
impl Game for Arr {
    fn update(&mut self, _input: &Input, frame: &mut Frame) {
        if self.started == 0u16 {
            frame.clear(Colour::Black);
            let mut i = 0u16;
            while i < 8u16 {
                self.cells[i as usize] = i * 2u16;   // write array elements
                i = i + 1u16;
            }
            self.started = 1u16;
        }
        let mut s = 0u16;
        let mut j = 0u16;
        while j < 8u16 {
            s = s + self.cells[j as usize];          // read array elements
            j = j + 1u16;
        }
        self.sum = s;
    }
}
"#;

#[test]
fn array_field_layout_in_symbol_map() {
    let (tap, sym) =
        speccy_sdk::compile::compile_game_with_symbols(ARR_GAME, "ARR").expect("compile");
    assert!(!tap.is_empty());
    let base = sym.base; // read the base from the map, not a compiler constant
    assert_eq!(sym.addr_of("sum"), Some(base), "scalar at slot 0");
    assert_eq!(
        sym.addr_of("cells"),
        Some(base + 2),
        "array starts at slot 1"
    );
    assert_eq!(
        sym.addr_of("started"),
        Some(base + 18),
        "next scalar shifted past 8 elems"
    );
    assert_eq!(sym.size, 20, "1 + 8 + 1 slots = 10 slots = 20 bytes");

    let cells = sym.fields.iter().find(|f| f.field == "cells").unwrap();
    assert_eq!(cells.count, 8, "array field reserves N elements");
    assert_eq!(
        sym.fields.iter().find(|f| f.field == "sum").unwrap().count,
        1,
        "scalar count = 1"
    );
}

/// On real hardware: the game writes `cells[i] = i*2` then sums them — proving both
/// `self.cells[i] = v` and `self.cells[i]` round-trip through the inline array field.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p rustz80 --test arrays -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn array_field_reads_and_writes_on_hardware() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, sym) =
        speccy_sdk::compile::compile_game_with_symbols(ARR_GAME, "ARR").expect("compile");
    let cells_addr = sym.addr_of("cells").unwrap();
    let sum_addr = sym.addr_of("sum").unwrap();

    let mut spec = spectrum::Spectrum::new_48k(&rom);
    for _ in 0..250 {
        spec.run_frame();
    }
    spec.load_tap(&tap).unwrap();
    spec.autoload_tape();
    for _ in 0..450 {
        spec.run_frame();
    }

    let rd = |a: u16| {
        let b = spec.read_memory(a, 2);
        b[0] as u16 | (b[1] as u16) << 8
    };
    for i in 0..8u16 {
        assert_eq!(
            rd(cells_addr + i * 2),
            i * 2,
            "cells[{i}] written via the array field"
        );
    }
    assert_eq!(
        rd(sum_addr),
        56,
        "sum of cells = 2*(0+..+7) read back via the array field"
    );
}
