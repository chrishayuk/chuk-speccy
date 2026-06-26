//! Every authored game is a benchmark (spec 08 §9). Drive the pure `reach` tape
//! through `SpectrumEnv` with baseline agents and confirm the score ordering
//! `no-op < random < scripted` — concrete numbers, read off the tape via the
//! symbol map, scored by the host `Game::reward`.

use speccy_env::agents::{run_episode, NoOpAgent, RandomAgent, ScriptedAgent};
use speccy_env::{FromState, SpectrumEnv, StateView, SymbolMap};

/// Host twin of the `reach` game: reconstructs the typed fields from RAM and scores
/// via `Game::reward` (score delta). It never runs `update` here — the tape does
/// that on the Z80; the host side only evaluates the env surface.
#[derive(Default)]
struct Reach {
    px: u16,
    py: u16,
    tx: u16,
    ty: u16,
    score: u16,
}

impl speccy_sdk::Game for Reach {
    fn update(&mut self, _i: &speccy_sdk::Input, _f: &mut speccy_sdk::Frame) {}
    fn reward(&self, prev: &Self) -> i16 {
        self.score as i16 - prev.score as i16
    }
}

impl FromState for Reach {
    fn from_state(s: &StateView) -> Self {
        Reach {
            px: s.u16("px"),
            py: s.u16("py"),
            tx: s.u16("tx"),
            ty: s.u16("ty"),
            score: s.u16("score"),
        }
    }
}

fn reach_source() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../speccy-sdk/samples/reach.rs"
    ))
    .expect("read reach.rs")
}

/// Offline: the benchmark wiring (compile → symbol map → reconstruct) holds without
/// a ROM.
#[test]
fn reach_compiles_and_reconstructs() {
    let (_tap, rz_map) =
        speccy_sdk::compile::compile_game_with_symbols(&reach_source(), "REACH").expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    assert!(map.addr_of("score").is_some() && map.addr_of("px").is_some());

    let r = Reach::from_state(&StateView::from_pairs(&[
        ("px", 3),
        ("py", 7),
        ("tx", 4),
        ("ty", 8),
        ("score", 9),
    ]));
    assert_eq!(
        (r.px, r.py, r.tx, r.ty, r.score),
        (3, 7, 4, 8, 9),
        "full reconstruction"
    );
}

/// The benchmark proper: scripted (steers toward the target via typed probes) beats
/// random beats no-op — the agentability ordering, on real hardware.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-env --test benchmark -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn scripted_beats_random_on_reach() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, rz_map) =
        speccy_sdk::compile::compile_game_with_symbols(&reach_source(), "REACH").expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    let mut env = SpectrumEnv::new(&rom, &tap, map, 500);

    // ~13 emulated frames per (heavy) dialect update; 18 frames/step gives the agent
    // roughly one move per step. 200 steps is plenty for the scripted homing agent.
    let steps = 200;
    let fps = 18;
    let noop = run_episode::<Reach, _>(&mut env, &mut NoOpAgent, steps, fps);
    let random = run_episode::<Reach, _>(&mut env, &mut RandomAgent::new(1), steps, fps);
    let scripted = run_episode::<Reach, _>(&mut env, &mut ScriptedAgent, steps, fps);

    eprintln!("\nreach — agentability ({steps} steps, {fps} frame/step):");
    eprintln!("  no-op     {noop}");
    eprintln!("  random    {random}");
    eprintln!("  scripted  {scripted}");

    assert_eq!(noop, 0, "a no-op agent never reaches the target");
    assert!(
        scripted > random,
        "scripted should beat random ({scripted} vs {random})"
    );
    assert!(
        scripted >= 5,
        "scripted reaches several targets ({scripted})"
    );
}
