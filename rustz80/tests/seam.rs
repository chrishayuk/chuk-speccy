//! "Prove the seam" (spec 08 §10): one typed `impl Game` source → a bootable
//! `.tap` *and* a symbol map, and an env reading a typed field off the running
//! tape's RAM via that map. This is the join the whole authoring plane stands on.

/// A minimal subset-clean game: `score` ticks up every frame. Two `u16` fields, so
/// the layout is unambiguous (`score` at the state base, `started` just after).
const SEAM_GAME: &str = r#"
struct Seam {
    score: u16,
    started: u16,
}
impl Game for Seam {
    fn update(&mut self, _input: &Input, frame: &mut Frame) {
        if self.started == 0u16 {
            frame.clear(Colour::Black);
            self.started = 1u16;
        }
        self.score = self.score + 1u16;
    }
}
"#;

/// Offline: the compiler emits a full-layout symbol map whose addresses match the
/// codegen layout (every field a `u16` slot at `GAME_STATE + i*2`). No ROM needed.
#[test]
fn emits_symbol_map_matching_layout() {
    let (tap, sym) = rustz80::compile_game_with_symbols(SEAM_GAME, "SEAM").expect("compile");
    assert!(!tap.is_empty(), "produced a tape");

    assert_eq!(sym.base, 0xB000, "the compiler's documented state-base ABI");
    assert_eq!(sym.size, 4, "two u16 fields = 4 bytes");
    assert_eq!(sym.addr_of("score"), Some(sym.base), "score is field 0");
    assert_eq!(
        sym.addr_of("started"),
        Some(sym.base + 2),
        "started is field 1"
    );
    assert_eq!(sym.addr_of("nope"), None);

    // The full layout is emitted (never a curated subset).
    assert_eq!(sym.fields.len(), 2, "every field present");

    let toml = sym.to_toml();
    assert!(toml.contains("[state]") && toml.contains("[fields]"));
    assert!(toml.contains(&format!("base = 0x{:04X}", sym.base)));
    assert!(toml.contains(&format!(
        "\"score\" = {{ addr = 0x{:04X}, width = 2, count = 1, ty = \"u16\" }}",
        sym.base
    )));
}

/// The seam itself, end to end on the real ROM: compile the game, boot its tape,
/// and read `score` off Z80 RAM *via the emitted symbol map* — no hand-written
/// address anywhere. If the field round-trips (typed decl → emitted addr → value
/// read off the running tape) and tracks the game logic, the architecture is real.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p rustz80 --test seam -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn score_round_trips_off_the_running_tape() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, sym) = rustz80::compile_game_with_symbols(SEAM_GAME, "SEAM").expect("compile");
    let score_addr = sym.addr_of("score").expect("score in the symbol map");

    let read_u16 = |spec: &spectrum::Spectrum| -> u16 {
        let b = spec.read_memory(score_addr, 2);
        b[0] as u16 | (b[1] as u16) << 8
    };

    let mut spec = spectrum::Spectrum::new_48k(&rom);
    for _ in 0..250 {
        spec.run_frame(); // boot to the K cursor
    }
    spec.load_tap(&tap).unwrap();
    spec.autoload_tape();
    for _ in 0..400 {
        spec.run_frame(); // load + auto-run into the frame loop
    }

    let s1 = read_u16(&spec);
    for _ in 0..600 {
        spec.run_frame();
    }
    let s2 = read_u16(&spec);

    assert!(
        s1 > 0,
        "score should be advancing once the game is running (got {s1})"
    );
    assert!(
        s2 > s1,
        "score must keep climbing as the env reads it live ({s1} -> {s2})"
    );
}
