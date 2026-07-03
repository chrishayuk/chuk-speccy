//! A sixth agentable game (spec 08 §9 — the multi-game agentability table, and doc
//! 09's capability C8: item pickup / use-on-obstacle). Drive the pure `island` tape
//! through `SpectrumEnv`: a host twin scores each subgoal milestone (axe/vine/key/
//! door/win), and a **quest** agent reads the player/inventory/obstacle-state probes
//! off the symbol map to sequence: fetch the item, cross the obstacle (which the game
//! auto-clears on a bump while carrying the right item), reach the exit — the same
//! three-waypoint shape repeated in room 1.
//!
//! Where `maze`'s planner solves a fixed room graph and `chase`/`platform` solve
//! movement/physics problems, this is the first agent that must *sequence subgoals*:
//! there is no path to the exit without first fetching the right item and using it on
//! the obstacle it clears — a different cognitive axis (puzzle/adventure), not
//! exploration or evasion.

use speccy_env::agents::{run_episode, Agent, NoOpAgent, RandomAgent};
use speccy_env::{FromState, SpectrumEnv, StateView, SymbolMap};

/// Host twin of `island`: reconstructs the typed fields and scores each subgoal
/// milestone via `Game::reward` (+1 each for axe/vine/key/door, +1 for the win — 5
/// total). `update` is a no-op — the tape runs the game on the Z80.
#[derive(Default)]
struct IslandBot {
    axe_taken: bool,
    vine_cleared: bool,
    key_taken: bool,
    door_open: bool,
    won: bool,
}

impl speccy_sdk::Game for IslandBot {
    fn update(&mut self, _i: &speccy_sdk::Input, _f: &mut speccy_sdk::Frame) {}
    fn reward(&self, prev: &Self) -> i16 {
        (self.axe_taken as i16 - prev.axe_taken as i16)
            + (self.vine_cleared as i16 - prev.vine_cleared as i16)
            + (self.key_taken as i16 - prev.key_taken as i16)
            + (self.door_open as i16 - prev.door_open as i16)
            + (self.won as i16 - prev.won as i16)
    }
    fn done(&self) -> bool {
        self.won
    }
}

impl FromState for IslandBot {
    fn from_state(s: &StateView) -> Self {
        IslandBot {
            axe_taken: s.bool("axe_taken"),
            vine_cleared: s.bool("vine_cleared"),
            key_taken: s.bool("key_taken"),
            door_open: s.bool("door_open"),
            won: s.bool("won"),
        }
    }
}

/// Just the border — a host mirror of `samples/island.rs`'s `wall(cx, cy)`, minus the
/// `self`-dependent obstacle column (the agent reads `vine_cleared`/`door_open`
/// directly off the symbol map instead of recomputing whether column 16 still blocks).
fn border(cx: u16, cy: u16) -> bool {
    cx == 0 || cx >= 31 || cy <= 1 || cy >= 23
}

/// Fetches the room's item, then heads just past the obstacle column — the bump while
/// carrying the item clears it and the agent walks straight through — then heads for
/// the exit. Reads only typed probes off the symbol map: `x`/`y`/`room`/`held` and the
/// room's own clear-flag (`vine_cleared` in room 0, `door_open` in room 1).
#[derive(Default)]
struct QuestAgent;

impl QuestAgent {
    // dir codes: 0 right, 1 down, 2 left, 3 up (matches `SnakeHomingAgent::key`).
    fn key(d: u16) -> char {
        ['p', 'a', 'o', 'q'][d as usize]
    }
    fn step(x: u16, y: u16, d: u16) -> (u16, u16) {
        match d {
            0 => (x + 1, y),
            1 => (x, y + 1),
            2 => (x.wrapping_sub(1), y),
            _ => (x, y.wrapping_sub(1)),
        }
    }
}

impl Agent for QuestAgent {
    fn act(&mut self, v: &StateView) -> Vec<char> {
        let (x, y) = (v.u16("x"), v.u16("y"));
        let room = v.u16("room");
        let held = v.u16("held");
        let cleared = if room == 0 {
            v.bool("vine_cleared")
        } else {
            v.bool("door_open")
        };

        // Empty-handed and not yet cleared -> the item; carrying it -> just past the
        // obstacle column (crossing it clears it); cleared -> the exit.
        let (tx, ty) = if cleared {
            (29u16, 12u16)
        } else if held != 0 {
            (17u16, 12u16)
        } else {
            (5u16, 5u16)
        };
        if (x, y) == (tx, ty) {
            return Vec::new();
        }

        let (dx, dy) = (tx as i32 - x as i32, ty as i32 - y as i32);
        let want_h = match dx.cmp(&0) {
            std::cmp::Ordering::Greater => Some(0u16),
            std::cmp::Ordering::Less => Some(2u16),
            std::cmp::Ordering::Equal => None,
        };
        let want_v = match dy.cmp(&0) {
            std::cmp::Ordering::Greater => Some(1u16),
            std::cmp::Ordering::Less => Some(3u16),
            std::cmp::Ordering::Equal => None,
        };
        let (a, b) = if dx.abs() >= dy.abs() {
            (want_h, want_v)
        } else {
            (want_v, want_h)
        };

        let order = [a, b, Some(0), Some(1), Some(2), Some(3)];
        for d in order.into_iter().flatten() {
            let (nx, ny) = Self::step(x, y, d);
            if !border(nx, ny) {
                return vec![Self::key(d)];
            }
        }
        Vec::new()
    }
}

fn island_source() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../speccy-sdk/samples/island.rs"
    ))
    .expect("read island.rs")
}

/// Offline: the wiring (compile → symbol map → reconstruct milestones) holds without a
/// ROM, and the quest agent sequences item -> crossing -> exit correctly.
#[test]
fn island_compiles_and_reconstructs() {
    let (_tap, rz_map) = speccy_sdk::compile::compile_game_with_symbols(&island_source(), "ISLAND")
        .expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    assert!(map.addr_of("held").is_some() && map.addr_of("won").is_some());
    assert!(map.addr_of("vine_cleared").is_some() && map.addr_of("door_open").is_some());

    let bot = IslandBot::from_state(&StateView::from_pairs(&[
        ("axe_taken", 1),
        ("vine_cleared", 1),
        ("key_taken", 0),
        ("door_open", 0),
        ("won", 0),
    ]));
    assert_eq!(
        (bot.axe_taken, bot.vine_cleared, bot.won),
        (true, true, false)
    );
}

/// The quest agent's three-phase targeting: item while empty-handed, the crossing
/// point while carrying it, the exit once cleared.
#[test]
fn quest_agent_sequences_item_crossing_and_exit() {
    let v = StateView::from_pairs(&[
        ("x", 2),
        ("y", 12),
        ("room", 0),
        ("held", 0),
        ("vine_cleared", 0),
    ]);
    assert_eq!(
        QuestAgent.act(&v),
        vec!['q'],
        "empty-handed: head for the item (up first, |dy|>|dx|)"
    );

    let v = StateView::from_pairs(&[
        ("x", 5),
        ("y", 5),
        ("room", 0),
        ("held", 1),
        ("vine_cleared", 0),
    ]);
    assert_eq!(
        QuestAgent.act(&v),
        vec!['p'],
        "carrying the axe: head for the crossing (right, |dx|>|dy|)"
    );

    let v = StateView::from_pairs(&[
        ("x", 17),
        ("y", 12),
        ("room", 0),
        ("held", 0),
        ("vine_cleared", 1),
    ]);
    assert_eq!(QuestAgent.act(&v), vec!['p'], "cleared: head for the exit");
}

/// On real hardware: the quest agent solves both rooms (reward 5), while no-op and
/// random never do. Generous budget: every room entry costs `frame.clear()`'s real
/// ~15-20-frame cost (`speccy-sdk/tests/dial.rs`'s
/// `rustz80_frame_clear_costs_about_20_frames_per_call` — a `rustz80` codegen quality
/// issue, not specific to this transition), not a normal single-move's worth.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-env --test island_bench -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn quest_agent_beats_random_on_island() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, rz_map) = speccy_sdk::compile::compile_game_with_symbols(&island_source(), "ISLAND")
        .expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    let mut env = SpectrumEnv::new(&rom, &tap, map, 500);

    let steps = 250;
    let fps = 3; // the player steps every 3 frames — one move per env step
    let noop = run_episode::<IslandBot, _>(&mut env, &mut NoOpAgent, steps, fps);
    let random = run_episode::<IslandBot, _>(&mut env, &mut RandomAgent::new(1), steps, fps);
    let quest = run_episode::<IslandBot, _>(&mut env, &mut QuestAgent, steps, fps);

    eprintln!("\nisland — agentability ({steps} steps, {fps} frame/step):");
    eprintln!("  no-op    {noop}");
    eprintln!("  random   {random}");
    eprintln!("  quest    {quest}");

    assert_eq!(noop, 0, "standing still never fetches an item");
    assert_eq!(
        quest, 5,
        "the quest agent should solve the whole chain (5 milestones)"
    );
    assert!(
        quest > random,
        "the quest agent should out-solve random ({quest} vs {random})"
    );
}
