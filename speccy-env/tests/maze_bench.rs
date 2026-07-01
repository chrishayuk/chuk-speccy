//! A third agentable game (spec 08 §9 — the multi-game agentability table). Drive the
//! pure `maze` tape through `SpectrumEnv`: a host twin scores room-progress off RAM, and a
//! **BFS pathfinding** agent reads `x`/`y`/`room` off the symbol map, shortest-paths to the
//! exit over a host copy of the wall map, and solves both rooms — beating random and no-op.
//!
//! Where the snake agent is a greedy homing heuristic, this one is a *planner*: it needs the
//! maze's structure (dead ends break greedy homing), so it runs a breadth-first search each
//! step. It reads only typed probes (no pixels), the way any state-driven agent would.

use speccy_env::agents::{run_episode, Agent, NoOpAgent, RandomAgent};
use speccy_env::{FromState, SpectrumEnv, StateView, SymbolMap};
use std::collections::VecDeque;

/// Host twin of `maze`: reconstructs the typed fields and scores progress via
/// `Game::reward` — +1 for each room cleared and +1 for winning the last one (so solving
/// the two-room maze totals 2). `update` is a no-op; the tape runs the game on the Z80.
#[derive(Default)]
struct MazeBot {
    room: u16,
    won: bool,
}

impl speccy_sdk::Game for MazeBot {
    fn update(&mut self, _i: &speccy_sdk::Input, _f: &mut speccy_sdk::Frame) {}
    fn reward(&self, prev: &Self) -> i16 {
        (self.room as i16 - prev.room as i16) + (self.won as i16 - prev.won as i16)
    }
    fn done(&self) -> bool {
        self.won // cleared the last room
    }
}

impl FromState for MazeBot {
    fn from_state(s: &StateView) -> Self {
        MazeBot {
            room: s.u16("room"),
            won: s.bool("won"),
        }
    }
}

/// The wall map — a host mirror of `samples/maze.rs`'s `wall(cx, cy, room)` (the agent
/// can't read a *function* off RAM, so it carries the same rules, like a game AI would).
fn wall(cx: u16, cy: u16, room: u16) -> bool {
    if cx == 0 || cx >= 31 || cy == 0 || cy >= 23 {
        return true;
    }
    if room == 0 {
        (cy == 6 && cx <= 27) || (cy == 12 && cx >= 4) || (cy == 18 && cx <= 27)
    } else {
        (cx == 8 && cy <= 18) || (cx == 16 && cy >= 5) || (cx == 24 && cy <= 18)
    }
}

const EXIT: (u16, u16) = (29, 21);
const W: u16 = 32;
const H: u16 = 24;

/// Breadth-first search from `(sx, sy)` to the exit over free cells; returns the key for
/// the first step of a shortest path (`None` if already there or boxed in).
fn first_move(sx: u16, sy: u16, room: u16) -> Option<char> {
    if (sx, sy) == EXIT {
        return None;
    }
    let idx = |x: u16, y: u16| (y * W + x) as usize;
    let mut prev = vec![usize::MAX; (W * H) as usize];
    let mut seen = vec![false; (W * H) as usize];
    seen[idx(sx, sy)] = true;
    let mut q = VecDeque::new();
    q.push_back((sx, sy));
    let mut reached = false;
    while let Some((cx, cy)) = q.pop_front() {
        if (cx, cy) == EXIT {
            reached = true;
            break;
        }
        let nbrs = [
            (cx, cy.wrapping_sub(1), cy > 0), // up
            (cx, cy + 1, cy + 1 < H),         // down
            (cx.wrapping_sub(1), cy, cx > 0), // left
            (cx + 1, cy, cx + 1 < W),         // right
        ];
        for (nx, ny, in_bounds) in nbrs {
            if !in_bounds || wall(nx, ny, room) || seen[idx(nx, ny)] {
                continue;
            }
            seen[idx(nx, ny)] = true;
            prev[idx(nx, ny)] = idx(cx, cy);
            q.push_back((nx, ny));
        }
    }
    if !reached {
        return None;
    }
    // Walk parents back from the exit until the one whose parent is the start — that's the
    // first step off the start cell.
    let start = idx(sx, sy);
    let mut step = idx(EXIT.0, EXIT.1);
    while prev[step] != start {
        step = prev[step];
    }
    let (fx, fy) = ((step as u16) % W, (step as u16) / W);
    Some(if fx > sx {
        'p' // right
    } else if fx < sx {
        'o' // left
    } else if fy > sy {
        'a' // down
    } else {
        'q' // up
    })
}

/// The planner: read the player cell + current room off the symbol map, BFS to the exit,
/// press the first step. Re-plans every step, so it handles the room reset on scene-flow.
#[derive(Default)]
struct MazePathAgent;

impl Agent for MazePathAgent {
    fn act(&mut self, v: &StateView) -> Vec<char> {
        match first_move(v.u16("x"), v.u16("y"), v.u16("room")) {
            Some(k) => vec![k],
            None => Vec::new(),
        }
    }
}

fn maze_source() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../speccy-sdk/samples/maze.rs"
    ))
    .expect("read maze.rs")
}

/// Offline: the wiring (compile → symbol map → reconstruct room/won) holds without a ROM,
/// and the planner steps toward the exit.
#[test]
fn maze_compiles_and_plans() {
    let (_tap, rz_map) =
        speccy_sdk::compile::compile_game_with_symbols(&maze_source(), "MAZE").expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    assert!(
        map.addr_of("x").is_some() && map.addr_of("room").is_some() && map.addr_of("won").is_some()
    );

    let bot = MazeBot::from_state(&StateView::from_pairs(&[("room", 1), ("won", 0)]));
    assert_eq!((bot.room, bot.won), (1, false));

    // One cell left of the exit (room 1) → the only shortest step is right.
    let v = StateView::from_pairs(&[("x", 28), ("y", 21), ("room", 1)]);
    assert_eq!(MazePathAgent.act(&v), vec!['p']);
    // Directly below the exit → step up.
    let v = StateView::from_pairs(&[("x", 29), ("y", 22), ("room", 1)]);
    assert_eq!(MazePathAgent.act(&v), vec!['q']);
    // From the start of room 0, the planner has a move and it isn't into a wall.
    let mv = MazePathAgent.act(&StateView::from_pairs(&[("x", 2), ("y", 2), ("room", 0)]));
    assert_eq!(
        mv.len(),
        1,
        "the planner always has a next step from the start"
    );
}

/// On real hardware: the BFS planner solves the maze (clears both rooms → reward 2), a
/// no-op never moves (0), and random flails (< the planner).
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-env --test maze_bench -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn pathfinder_solves_the_maze() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, rz_map) =
        speccy_sdk::compile::compile_game_with_symbols(&maze_source(), "MAZE").expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    let mut env = SpectrumEnv::new(&rom, &tap, map, 500);

    let steps = 400;
    let fps = 3; // the player steps every 3 frames — one move per env step
    let noop = run_episode::<MazeBot, _>(&mut env, &mut NoOpAgent, steps, fps);
    let random = run_episode::<MazeBot, _>(&mut env, &mut RandomAgent::new(1), steps, fps);
    let planner = run_episode::<MazeBot, _>(&mut env, &mut MazePathAgent, steps, fps);

    eprintln!("\nmaze — agentability ({steps} steps, {fps} frame/step):");
    eprintln!("  no-op     {noop}");
    eprintln!("  random    {random}");
    eprintln!("  planner   {planner}");

    assert_eq!(noop, 0, "a no-op never moves, so it never reaches an exit");
    assert!(
        planner >= 1,
        "the planner clears at least the first room ({planner})"
    );
    assert!(
        planner > random,
        "the planner should out-solve random ({planner} vs {random})"
    );
}
