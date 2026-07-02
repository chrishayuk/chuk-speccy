//! A fifth agentable game (spec 08 §9 — the multi-game agentability table). Drive the
//! pure `platform` tape through `SpectrumEnv`: a host twin scores **rightward
//! progress** off RAM (plus a large bonus per coin), and a **climber** agent walks
//! right, jumping at a ledge or proactively under an unmounted platform (a host mirror
//! of the level's `solid(cx, cy)`, exactly like `maze_bench`'s `wall()` mirror) — no
//! pixels.
//!
//! Landing squarely on a platform is a genuine precision-jump problem: rise and drift
//! both advance exactly one cell per tick, so a jump launched from underneath a low
//! platform either bonks its ceiling before clearing it, or (launched later) clears it
//! only by drifting out of that platform's own column span first — a plain heuristic
//! can make real, repeatable progress up the level without landing every jump exactly,
//! so progress (not "did it collect this specific coin") is the honest, tractable
//! measure of how much better than random it plays.

use speccy_env::agents::{run_episode, Agent, NoOpAgent, RandomAgent};
use speccy_env::{FromState, SpectrumEnv, StateView, SymbolMap};

/// Host twin of `platform`: reconstructs the typed fields and scores via
/// `Game::reward`. `update` is a no-op — the tape runs the game on the Z80.
#[derive(Default)]
struct PlatformBot {
    x: u16,
    score: u16,
    dead: u16,
    won: bool,
}

impl speccy_sdk::Game for PlatformBot {
    fn update(&mut self, _i: &speccy_sdk::Input, _f: &mut speccy_sdk::Frame) {}
    fn reward(&self, prev: &Self) -> i16 {
        // Unclamped: summed over an episode this telescopes to net (final − initial) x,
        // so backtracking cancels out instead of a random jitter racking up "progress"
        // from side-to-side steps that never actually advance.
        let progressed = self.x as i16 - prev.x as i16;
        let scored = (self.score as i16 - prev.score as i16) * 100;
        progressed + scored
    }
    fn done(&self) -> bool {
        self.dead != 0 || self.won // fell in the pit, or reached the exit
    }
}

impl FromState for PlatformBot {
    fn from_state(s: &StateView) -> Self {
        PlatformBot {
            x: s.u16("x"),
            score: s.u16("score"),
            dead: s.u16("dead"),
            won: s.bool("won"),
        }
    }
}

/// The level map — a host mirror of `samples/platform.rs`'s `solid(cx, cy)` (the agent
/// can't read a *function* off RAM, so it carries the same rules, exactly like
/// `maze_bench.rs`'s `wall()` mirror). Walls, a floor with a pit at cols 14–15, and
/// three platforms.
fn solid(cx: u16, cy: u16) -> bool {
    if cx == 0 || cx >= 31 {
        return true;
    }
    if cy >= 22 {
        return !(14..=15).contains(&cx); // the pit is the one gap in the floor
    }
    (cy == 18 && (6..=11).contains(&cx))
        || (cy == 14 && (16..=23).contains(&cx))
        || (cy == 10 && (24..=29).contains(&cx))
}

/// The level's platforms as `(solid row, first col, last col)` — the low, mid, and
/// high platform in `solid`'s own layout. A single 5-cell rise from the floor only
/// reaches the low platform; the mid and high platforms are each one further jump up
/// *from the platform below*, so a climber has to actually mount each one in turn, not
/// just leap the pit and hope.
const PLATFORMS: [(u16, u16, u16); 3] = [(18, 6, 11), (14, 16, 23), (10, 24, 29)];

/// Walks right, jumping either at a ledge (the pit, or a platform's own edge) or
/// proactively while standing under a platform it hasn't yet mounted — a purely
/// reactive "walk until the ground runs out" policy never climbs, since the mid/high
/// platforms sit off to the side of the floor, not blocking it. `update` processes
/// horizontal movement *before* checking `onground`, so holding Right and Up together
/// right at a ledge steps onto the gap first and only then finds no ground — too late
/// to launch. So the agent holds only Up while still standing on the last solid cell
/// (jump starts *and* rises in that same tick, since `update` rises immediately once
/// `jump` is set), and only holds Right once airborne, drifting across during the
/// rise/fall. Reads the typed `x`/`y` probes and the level's own geometry (the `solid`
/// mirror and `PLATFORMS` above) — no pixels, no `jump` state needed.
#[derive(Default)]
struct PlatformClimberAgent;

impl Agent for PlatformClimberAgent {
    fn act(&mut self, v: &StateView) -> Vec<char> {
        let (x, y) = (v.u16("x"), v.u16("y"));
        let onground = y >= 23 || solid(x, y + 1);
        if !onground {
            return vec!['p']; // airborne — drift across the gap
        }

        let under_unmounted_platform = PLATFORMS
            .iter()
            .any(|&(row, lo, hi)| (lo..=hi).contains(&x) && y > row - 1);
        let ahead_supported = x + 1 >= 31 || y + 1 >= 23 || solid(x + 1, y + 1);
        if under_unmounted_platform || !ahead_supported {
            vec!['q'] // climb, or a ledge — jump in place; drift starts once airborne
        } else {
            vec!['p'] // safe to walk onto the next cell
        }
    }
}

fn platform_source() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../speccy-sdk/samples/platform.rs"
    ))
    .expect("read platform.rs")
}

/// Offline: the wiring (compile → symbol map → reconstruct x/score/dead/won) holds
/// without a ROM, and the climber walks on open ground but jumps at a ledge.
#[test]
fn platform_compiles_and_reconstructs() {
    let (_tap, rz_map) = speccy_sdk::compile::compile_game_with_symbols(&platform_source(), "PLAT")
        .expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    assert!(map.addr_of("score").is_some() && map.addr_of("won").is_some());
    assert!(map.addr_of("jump").is_some(), "the jump counter is mapped");

    let bot = PlatformBot::from_state(&StateView::from_pairs(&[
        ("x", 10),
        ("score", 2),
        ("dead", 0),
        ("won", 0),
    ]));
    assert_eq!((bot.x, bot.score, bot.dead, bot.won), (10, 2, 0, false));

    // On solid ground with more solid ground ahead: just walk right.
    let mid_floor = StateView::from_pairs(&[("x", 5), ("y", 21)]);
    assert_eq!(PlatformClimberAgent.act(&mid_floor), vec!['p']);
    // At the pit's edge (col 13, floor ends at col 14): jump in place, not walk+jump
    // (which would step onto the gap before `onground` is even checked).
    let ledge = StateView::from_pairs(&[("x", 13), ("y", 21)]);
    assert_eq!(PlatformClimberAgent.act(&ledge), vec!['q']);
    // Airborne (no ground below at the current cell): drift right.
    let airborne = StateView::from_pairs(&[("x", 14), ("y", 20)]);
    assert_eq!(PlatformClimberAgent.act(&airborne), vec!['p']);
}

/// On real hardware: the climber makes real rightward progress up the level, while a
/// no-op platform (cursor never pressed) makes none.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-env --test platform_bench -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn climber_beats_random_on_platform() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, rz_map) = speccy_sdk::compile::compile_game_with_symbols(&platform_source(), "PLAT")
        .expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    let mut env = SpectrumEnv::new(&rom, &tap, map, 500);

    let steps = 200;
    let fps = 3; // the player steps every 3 frames — one move per env step
    let noop = run_episode::<PlatformBot, _>(&mut env, &mut NoOpAgent, steps, fps);
    let random = run_episode::<PlatformBot, _>(&mut env, &mut RandomAgent::new(1), steps, fps);
    let climber = run_episode::<PlatformBot, _>(&mut env, &mut PlatformClimberAgent, steps, fps);

    eprintln!("\nplatform — agentability ({steps} steps, {fps} frame/step):");
    eprintln!("  no-op     {noop}");
    eprintln!("  random    {random}");
    eprintln!("  climber   {climber}");

    assert_eq!(noop, 0, "standing still makes no progress");
    assert!(climber > 0, "the climber makes real progress ({climber})");
    assert!(
        climber > random,
        "the climber should out-progress random ({climber} vs {random})"
    );
}
