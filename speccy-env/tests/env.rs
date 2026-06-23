//! The env side of the bridge, end to end (spec 08 §2–§3): compile a dialect game
//! together with its symbol map, boot the tape, read the typed `score` off RAM,
//! rebuild a host-side game from it, and run the host `reward` — the same code the
//! host build would run. Then prove bit-exact reset.

use speccy_env::{FromState, SpectrumEnv, StateView, SymbolMap};

/// The same minimal game as `rustz80/tests/seam.rs`: `score` ticks up every frame.
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

/// The host twin: a plain struct that reconstructs from RAM and scores via the
/// typed `Game::reward` (score delta). It never runs `update` here — the *tape*
/// does that on the Z80; the host side only evaluates the env surface.
#[derive(Default)]
struct ScoreGame {
    score: u16,
    #[allow(dead_code)]
    started: u16,
}

impl speccy_sdk::Game for ScoreGame {
    fn update(&mut self, _input: &speccy_sdk::Input, _frame: &mut speccy_sdk::Frame) {}
    fn reward(&self, prev: &Self) -> i16 {
        self.score as i16 - prev.score as i16
    }
}

impl FromState for ScoreGame {
    fn from_state(s: &StateView) -> Self {
        ScoreGame {
            score: s.u16("score"),
            started: s.u16("started"),
        }
    }
}

/// The env reads `score` off the running tape via the emitted symbol map, rebuilds
/// `ScoreGame`, and the host `reward` (score delta) tracks the game — and `reset`
/// is bit-exact.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-env --test env -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn env_reads_reward_off_the_tape_and_resets_bit_exact() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, rz_map) = rustz80::compile_game_with_symbols(SEAM_GAME, "SEAM").expect("compile");
    // Go through the sidecar format — exactly what a consumer with a `.tap` +
    // `.sym.toml` on disk would do.
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse sym.toml");
    assert!(map.addr_of("score").is_some(), "score is in the map");

    let mut env = SpectrumEnv::new(&rom, &tap, map, 450);

    // The score is already advancing once warmed up.
    let s0 = env.view().u16("score");
    assert!(s0 > 0, "game running after warmup (score {s0})");

    // A step: reconstruct the host game, run frames, reward = score delta.
    let t1 = env.step_game::<ScoreGame>(&[], 30);
    assert!(
        t1.reward > 0,
        "reward read off the tape via the host Game (got {})",
        t1.reward
    );
    assert!(!t1.done, "the seam game never ends");
    let t2 = env.step_game::<ScoreGame>(&[], 30);
    assert!(
        t2.reward > 0,
        "reward keeps coming as the env reads live state"
    );

    // Bit-exact reset: back to exactly the warmup snapshot.
    let advanced = env.view().u16("score");
    assert!(
        advanced > s0,
        "score advanced before reset ({s0} -> {advanced})"
    );
    env.reset();
    assert_eq!(
        env.view().u16("score"),
        s0,
        "reset restores the snapshot bit-exactly"
    );
}
