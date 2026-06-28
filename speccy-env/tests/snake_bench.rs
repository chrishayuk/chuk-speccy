//! A second agentable game (spec 08 §9 — toward a multi-game agentability table).
//! Drive the pure `snake_game` tape through `SpectrumEnv`: a host twin reconstructs
//! the typed state off RAM and scores `len` growth, and a reverse-aware homing agent
//! (head → food, read purely from the symbol map) beats random beats no-op.

use speccy_env::agents::{
    run_episode, Agent, NoOpAgent, RandomAgent, RecordingAgent, ReplayAgent,
};
use speccy_env::{FromState, SpectrumEnv, StateView, SymbolMap};

/// Host twin of `snake_game`: reconstructs the typed fields and scores via
/// `Game::reward` (body-length delta). `update` is a no-op — the tape runs the game on
/// the Z80; the host side only evaluates the env surface.
#[derive(Default)]
struct SnakeBot {
    len: u16,
    dead: u16,
}

impl speccy_sdk::Game for SnakeBot {
    fn update(&mut self, _i: &speccy_sdk::Input, _f: &mut speccy_sdk::Frame) {}
    fn reward(&self, prev: &Self) -> i16 {
        self.len as i16 - prev.len as i16 // grew = ate
    }
    fn done(&self) -> bool {
        self.dead != 0 // crashed into itself → episode over
    }
}

impl FromState for SnakeBot {
    fn from_state(s: &StateView) -> Self {
        SnakeBot {
            len: s.u16("len"),
            dead: s.u16("dead"),
        }
    }
}

/// Steers the head toward the food while staying alive: it prefers the larger-gap axis
/// toward the food, but skips any move that would **reverse** onto the neck (ignored by
/// the game) or run **into a wall** — falling back to going straight, then any safe
/// direction. Reads only typed probes (`bx[0]`/`by[0]`/`food_x`/`dir`), no pixels.
#[derive(Default)]
struct SnakeHomingAgent;

impl SnakeHomingAgent {
    // dir codes: 0 right, 1 down, 2 left, 3 up.
    fn is_reverse(d: u16, dir: u16) -> bool {
        (d + 2) % 4 == dir
    }
    fn into_wall(d: u16, hx: u16, hy: u16) -> bool {
        (d == 0 && hx >= 31) || (d == 2 && hx == 0) || (d == 1 && hy >= 23) || (d == 3 && hy == 0)
    }
    fn key(d: u16) -> char {
        ['p', 'a', 'o', 'q'][d as usize] // right, down, left, up
    }
}

impl Agent for SnakeHomingAgent {
    fn act(&mut self, v: &StateView) -> Vec<char> {
        let hx = v.array("bx").first().copied().unwrap_or(0);
        let hy = v.array("by").first().copied().unwrap_or(0);
        let (fx, fy) = (v.u16("food_x"), v.u16("food_y"));
        let dir = v.u16("dir");
        let (dx, dy) = (fx as i32 - hx as i32, fy as i32 - hy as i32);

        let want_h = match dx.cmp(&0) {
            std::cmp::Ordering::Greater => Some(0u16), // right
            std::cmp::Ordering::Less => Some(2u16),    // left
            std::cmp::Ordering::Equal => None,
        };
        let want_v = match dy.cmp(&0) {
            std::cmp::Ordering::Greater => Some(1u16), // down
            std::cmp::Ordering::Less => Some(3u16),    // up
            std::cmp::Ordering::Equal => None,
        };
        // Toward-food axes (larger gap first), then survival fallbacks: keep going
        // straight, then any direction. First that isn't a reverse or into a wall wins.
        let (a, b) = if dx.abs() >= dy.abs() {
            (want_h, want_v)
        } else {
            (want_v, want_h)
        };
        let order = [a, b, Some(dir), Some(0), Some(1), Some(2), Some(3)];
        for d in order.into_iter().flatten() {
            if !Self::is_reverse(d, dir) && !Self::into_wall(d, hx, hy) {
                return vec![Self::key(d)];
            }
        }
        Vec::new()
    }
}

fn snake_source() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../speccy-sdk/samples/snake_game.rs"
    ))
    .expect("read snake_game.rs")
}

/// Offline: the wiring (compile → symbol map → reconstruct the head/food/len) holds
/// without a ROM.
#[test]
fn snake_compiles_and_reconstructs() {
    let (_tap, rz_map) =
        speccy_sdk::compile::compile_game_with_symbols(&snake_source(), "SNAKE").expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    assert!(map.addr_of("len").is_some() && map.addr_of("food_x").is_some());
    assert!(map.addr_of("bx").is_some(), "the body array is mapped");

    // The twin reconstructs its reward + done fields off the typed state.
    let bot = SnakeBot::from_state(&StateView::from_pairs(&[("len", 5), ("dead", 0)]));
    assert_eq!((bot.len, bot.dead), (5, 0));
}

/// The homing agent never reverses and aims the larger gap first.
#[test]
fn homing_agent_steers_toward_food_without_reversing() {
    // Food down-right, moving right (dir 0): bigger gap is horizontal → keep right.
    let v = StateView::from_pairs(&[("bx", 5), ("by", 5), ("food_x", 12), ("food_y", 8), ("dir", 0)]);
    assert_eq!(SnakeHomingAgent.act(&v), vec!['p']);
    // Food is left while moving right (dir 0) — a reverse — so take the vertical axis.
    let v = StateView::from_pairs(&[("bx", 20), ("by", 5), ("food_x", 3), ("food_y", 9), ("dir", 0)]);
    assert_eq!(SnakeHomingAgent.act(&v), vec!['a']);
}

/// On real hardware: the homing agent grows the snake more than random, and a no-op
/// (cursor never pressed → the snake crawls straight and never finds food) scores 0.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-env --test snake_bench -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn homing_beats_random_on_snake() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, rz_map) =
        speccy_sdk::compile::compile_game_with_symbols(&snake_source(), "SNAKE").expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    let mut env = SpectrumEnv::new(&rom, &tap, map, 500);

    let steps = 200;
    let fps = 4; // one snake move per step (it steps every 4 frames) — fine control
    let noop = run_episode::<SnakeBot, _>(&mut env, &mut NoOpAgent, steps, fps);
    let random = run_episode::<SnakeBot, _>(&mut env, &mut RandomAgent::new(1), steps, fps);
    let homing = run_episode::<SnakeBot, _>(&mut env, &mut SnakeHomingAgent, steps, fps);

    eprintln!("\nsnake — agentability ({steps} steps, {fps} frame/step):");
    eprintln!("  no-op    {noop}");
    eprintln!("  random   {random}");
    eprintln!("  homing   {homing}");

    assert_eq!(noop, 0, "a no-op snake crawls straight and never eats");
    assert!(homing > 0, "the homing agent eats at least once ({homing})");
    assert!(homing > random, "homing should out-grow random ({homing} vs {random})");
}

/// The agent-lab cornerstone: record the homing agent's actions, then **replay** them
/// — bit-exact `reset` + the same key sequence must reproduce the episode's reward
/// exactly (deterministic rollouts / replayable repros).
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-env --test snake_bench -- --ignored replay
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn replay_reproduces_the_homing_episode() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, rz_map) =
        speccy_sdk::compile::compile_game_with_symbols(&snake_source(), "SNAKE").expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    let mut env = SpectrumEnv::new(&rom, &tap, map, 500);

    let (steps, fps) = (120, 4);
    let mut rec = RecordingAgent::new(SnakeHomingAgent);
    let recorded = run_episode::<SnakeBot, _>(&mut env, &mut rec, steps, fps);
    let tape_log = rec.log.clone();

    let replayed = run_episode::<SnakeBot, _>(&mut env, &mut ReplayAgent::new(tape_log), steps, fps);

    assert!(recorded > 0, "the recorded episode actually scored ({recorded})");
    assert_eq!(
        recorded, replayed,
        "replaying the same actions reproduces the episode ({recorded} vs {replayed})"
    );
}
