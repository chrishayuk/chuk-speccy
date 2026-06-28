//! A second agentable game (spec 08 §9 — toward a multi-game agentability table).
//! Drive the pure `snake_game` tape through `SpectrumEnv`: a host twin reconstructs
//! the typed state off RAM and scores `len` growth, and a reverse-aware homing agent
//! (head → food, read purely from the symbol map) beats random beats no-op.

use speccy_env::agents::{run_episode, Agent, NoOpAgent, RandomAgent};
use speccy_env::{FromState, SpectrumEnv, StateView, SymbolMap};

/// Host twin of `snake_game`: reconstructs the typed fields and scores via
/// `Game::reward` (body-length delta). `update` is a no-op — the tape runs the game on
/// the Z80; the host side only evaluates the env surface.
#[derive(Default)]
struct SnakeBot {
    len: u16,
}

impl speccy_sdk::Game for SnakeBot {
    fn update(&mut self, _i: &speccy_sdk::Input, _f: &mut speccy_sdk::Frame) {}
    fn reward(&self, prev: &Self) -> i16 {
        self.len as i16 - prev.len as i16 // grew = ate
    }
}

impl FromState for SnakeBot {
    fn from_state(s: &StateView) -> Self {
        SnakeBot { len: s.u16("len") }
    }
}

/// Steers the head toward the food one axis at a time (the larger gap first), reading
/// `dir` so it never commands a reverse (which `snake_game` ignores). Typed probes
/// only — no pixels.
#[derive(Default)]
struct SnakeHomingAgent;

impl Agent for SnakeHomingAgent {
    fn act(&mut self, v: &StateView) -> Vec<char> {
        let hx = v.array("bx").first().copied().unwrap_or(0) as i32;
        let hy = v.array("by").first().copied().unwrap_or(0) as i32;
        let (fx, fy) = (v.u16("food_x") as i32, v.u16("food_y") as i32);
        let dir = v.u16("dir"); // 0 right, 1 down, 2 left, 3 up
        let (dx, dy) = (fx - hx, fy - hy);

        // (key, the dir it would be reversing — pressing it is ignored if dir == that).
        let horiz = match dx.cmp(&0) {
            std::cmp::Ordering::Greater => Some(('p', 2u16)), // right, blocked if going left
            std::cmp::Ordering::Less => Some(('o', 0u16)),    // left, blocked if going right
            std::cmp::Ordering::Equal => None,
        };
        let vert = match dy.cmp(&0) {
            std::cmp::Ordering::Greater => Some(('a', 3u16)), // down, blocked if going up
            std::cmp::Ordering::Less => Some(('q', 1u16)),    // up, blocked if going down
            std::cmp::Ordering::Equal => None,
        };
        // Try the larger-gap axis first; fall back to the other if it'd be a reverse.
        let (first, second) = if dx.abs() >= dy.abs() {
            (horiz, vert)
        } else {
            (vert, horiz)
        };
        for cand in [first, second].into_iter().flatten() {
            if dir != cand.1 {
                return vec![cand.0];
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

    // The twin reconstructs its reward field off the typed state.
    let bot = SnakeBot::from_state(&StateView::from_pairs(&[("len", 5)]));
    assert_eq!(bot.len, 5);
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
    let fps = 8; // ~2 snake moves per step (it steps every 4 frames)
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
