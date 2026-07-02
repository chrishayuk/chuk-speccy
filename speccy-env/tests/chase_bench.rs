//! A fourth agentable game (spec 08 §9 — the multi-game agentability table). Drive the
//! pure `chase` tape through `SpectrumEnv`: a host twin scores **survival time** off
//! RAM (plus a large bonus per coin), and a **coin-foraging** agent reads the
//! player/coin/enemy arrays off the symbol map, fleeing the nearest enemy once it's
//! close and otherwise heading for the nearest safe coin — outliving both random and
//! no-op.
//!
//! Where the snake agent is a greedy homing heuristic and the maze agent a BFS planner,
//! this one is *reactive avoidance* under active pursuit: three enemies (`ex[]`/`ey[]`)
//! at the player's own speed chase every tick, which makes outright coin-collection a
//! genuinely hard multi-pursuer evasion problem (no speed edge, only positioning) — so
//! survival is the tractable, honest measure of how much better than random the agent
//! plays, and a landed coin (worth 100× a tick survived) is a bonus on top. It reads
//! only typed probes (no pixels).

use speccy_env::agents::{run_episode, Agent, NoOpAgent, RandomAgent};
use speccy_env::{FromState, SpectrumEnv, StateView, SymbolMap};

/// Host twin of `chase`: reconstructs the typed fields and scores via `Game::reward`.
/// Three enemies at equal speed to the player make coin-collection a genuinely hard
/// pursuit-evasion problem (there's no player speed edge, only positioning), so the
/// reward here is **survival time** (+1 per still-alive step) with a large bonus per
/// coin — a scripted evader that reads the enemy/coin arrays should out-survive a
/// blind random walk even before it ever lands a coin. `update` is a no-op — the tape
/// runs the game on the Z80; the host side only evaluates the env surface.
#[derive(Default)]
struct ChaseBot {
    score: u16,
    dead: u16,
    won: bool,
}

impl speccy_sdk::Game for ChaseBot {
    fn update(&mut self, _i: &speccy_sdk::Input, _f: &mut speccy_sdk::Frame) {}
    fn reward(&self, prev: &Self) -> i16 {
        let alive = (self.dead == 0) as i16;
        let scored = (self.score as i16 - prev.score as i16) * 100;
        alive + scored
    }
    fn done(&self) -> bool {
        self.dead != 0 || self.won // caught by an enemy, or all 4 coins collected
    }
}

impl FromState for ChaseBot {
    fn from_state(s: &StateView) -> Self {
        ChaseBot {
            score: s.u16("score"),
            dead: s.u16("dead"),
            won: s.bool("won"),
        }
    }
}

/// The wall map — a host mirror of `samples/chase.rs`'s `solid(cx, cy)` (the agent
/// can't read a *function* off RAM, so it carries the same rules, exactly like
/// `maze_bench.rs`'s `wall()` mirror). One bordered room, row 0/1 walls, and three
/// pillars.
fn solid(cx: u16, cy: u16) -> bool {
    if cx == 0 || cx >= 31 || cy <= 1 || cy >= 23 {
        return true;
    }
    (cy == 8 && (8..=12).contains(&cx))
        || (cy == 15 && (18..=24).contains(&cx))
        || (cx == 16 && (10..=14).contains(&cy))
}

/// Heads for the nearest uncollected coin (Manhattan-nearest, larger-gap axis first —
/// the same heuristic as the snake homing agent), skipping any direction that's a wall
/// (the `solid` mirror) or a cell an enemy currently occupies — falling back through
/// every direction so it never deadlocks against a pillar. It also avoids **reversing**
/// its last move unless that's the only option: with three enemies converging on the
/// same cell above the player, greedy-only chasing settles into a stable left-right
/// ping-pong (the axis toward the coin stays blocked from both neighbouring cells);
/// refusing to immediately undo the last step forces a detour that breaks the cycle.
/// Reads only typed probes off the symbol map: `x`/`y`, the coin arrays
/// `cgx`/`cgy`/`got`, and the enemy arrays `ex`/`ey`.
#[derive(Default)]
struct ChaseForagerAgent {
    last: Option<u16>,
}

impl ChaseForagerAgent {
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
    fn reverse_of(d: u16) -> u16 {
        (d + 2) % 4
    }
}

impl Agent for ChaseForagerAgent {
    fn act(&mut self, v: &StateView) -> Vec<char> {
        let (x, y) = (v.u16("x"), v.u16("y"));
        let (cgx, cgy, got) = (v.array("cgx"), v.array("cgy"), v.array("got"));
        let (ex, ey) = (v.array("ex"), v.array("ey"));

        // Flee the nearest enemy once it's within striking distance — otherwise the
        // three enemies (independently, but converging on the same target) mob the
        // player's neighbourhood and permanently wall off whatever coin it's nearing.
        // Only once nothing is closing in does it resume seeking the nearest coin.
        const THREAT: i32 = 3;
        let nearest_enemy = (0..ex.len())
            .map(|i| (ex[i] as i32 - x as i32).abs() + (ey[i] as i32 - y as i32).abs())
            .min()
            .unwrap_or(i32::MAX);

        let (dx, dy) = if nearest_enemy <= THREAT {
            let i = (0..ex.len())
                .min_by_key(|&i| (ex[i] as i32 - x as i32).abs() + (ey[i] as i32 - y as i32).abs())
                .unwrap();
            (x as i32 - ex[i] as i32, y as i32 - ey[i] as i32) // away from it
        } else {
            // Prefer a coin that's both close to the player *and* far from every enemy
            // — a purely player-nearest target walks straight at whichever enemy spawned
            // closest to it, since each coin sits near exactly one enemy's corner.
            let target = (0..cgx.len())
                .filter(|&k| got.get(k).copied().unwrap_or(1) == 0)
                .min_by_key(|&k| {
                    let to_player =
                        (cgx[k] as i32 - x as i32).abs() + (cgy[k] as i32 - y as i32).abs();
                    let from_nearest_enemy = (0..ex.len())
                        .map(|i| {
                            (ex[i] as i32 - cgx[k] as i32).abs()
                                + (ey[i] as i32 - cgy[k] as i32).abs()
                        })
                        .min()
                        .unwrap_or(0);
                    to_player - 2 * from_nearest_enemy
                });
            match target {
                Some(k) => (cgx[k] as i32 - x as i32, cgy[k] as i32 - y as i32),
                None => return Vec::new(), // every coin gone, nothing chasing — hold
            }
        };

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

        // The larger-gap axis first, then the other axis, then every direction as a
        // last resort (so a pillar in the way never deadlocks the agent) — skip a wall
        // or a cell an enemy currently occupies. A candidate that reverses the last
        // move is remembered but only taken if nothing else is safe.
        let order = [a, b, Some(0), Some(1), Some(2), Some(3)];
        let mut reversal = None;
        for d in order.into_iter().flatten() {
            let (nx, ny) = Self::step(x, y, d);
            if solid(nx, ny) {
                continue;
            }
            if (0..ex.len()).any(|i| ex[i] == nx && ey[i] == ny) {
                continue;
            }
            if self.last == Some(Self::reverse_of(d)) {
                reversal.get_or_insert(d);
                continue;
            }
            self.last = Some(d);
            return vec![Self::key(d)];
        }
        if let Some(d) = reversal {
            self.last = Some(d);
            return vec![Self::key(d)];
        }
        Vec::new()
    }

    fn reset(&mut self) {
        self.last = None;
    }
}

fn chase_source() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../speccy-sdk/samples/chase.rs"
    ))
    .expect("read chase.rs")
}

/// Offline: the wiring (compile → symbol map → reconstruct score/dead/won) holds
/// without a ROM, and the forager targets the nearer coin while dodging an enemy.
#[test]
fn chase_compiles_and_reconstructs() {
    let (_tap, rz_map) =
        speccy_sdk::compile::compile_game_with_symbols(&chase_source(), "CHASE").expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    assert!(map.addr_of("score").is_some() && map.addr_of("won").is_some());
    assert!(
        map.addr_of("cgx").is_some() && map.addr_of("ex").is_some(),
        "the coin and enemy arrays are mapped"
    );

    let bot = ChaseBot::from_state(&StateView::from_pairs(&[
        ("score", 2),
        ("dead", 0),
        ("won", 0),
    ]));
    assert_eq!((bot.score, bot.dead, bot.won), (2, 0, false));
}

/// The forager prefers the nearer coin, and steps around a cell an enemy occupies.
#[test]
fn forager_prefers_nearer_coin_and_dodges_enemies() {
    // Player at (10,10), one uncollected coin at (12,10) — go for it (right).
    let v = StateView::from_arrays(&[
        ("x", &[10]),
        ("y", &[10]),
        ("cgx", &[12]),
        ("cgy", &[10]),
        ("got", &[0]),
        ("ex", &[0]),
        ("ey", &[0]),
    ]);
    let mut agent = ChaseForagerAgent::default();
    assert_eq!(agent.act(&v), vec!['p']);

    // Same setup, but an enemy sits exactly on the cell to the right — the forager must
    // not step there.
    let v = StateView::from_arrays(&[
        ("x", &[10]),
        ("y", &[10]),
        ("cgx", &[12]),
        ("cgy", &[10]),
        ("got", &[0]),
        ("ex", &[11]),
        ("ey", &[10]),
    ]);
    let mut agent = ChaseForagerAgent::default();
    let keys = agent.act(&v);
    assert_ne!(keys, vec!['p'], "must not step onto the enemy's cell");
}

/// On real hardware: the forager outlives both baselines. A no-op dies at a fixed,
/// deterministic tick (the nearest enemy's straight-line approach) — a hard floor.
/// Random's survival is noisy (it can blunder *toward* an oncoming enemy, sometimes
/// dying faster than standing still, sometimes wandering clear for a while), but the
/// forager's flee-then-forage policy reliably outlasts it. Warmup is deliberately
/// short (60 frames, not the usual 500): `chase`'s enemies chase from frame one, so a
/// long idle warmup — fine for every other sample — would snapshot the reset baseline
/// *after* a no-input player has already been caught.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p chuk-speccy-env --test chase_bench -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn forager_beats_random_on_chase() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
    let (tap, rz_map) =
        speccy_sdk::compile::compile_game_with_symbols(&chase_source(), "CHASE").expect("compile");
    let map = SymbolMap::from_toml(&rz_map.to_toml()).expect("parse");
    let mut env = SpectrumEnv::new(&rom, &tap, map, 60);

    let steps = 200;
    let fps = 4; // the player steps every 4 frames — one move per env step
    let noop = run_episode::<ChaseBot, _>(&mut env, &mut NoOpAgent, steps, fps);
    let random = run_episode::<ChaseBot, _>(&mut env, &mut RandomAgent::new(2), steps, fps);
    let forager =
        run_episode::<ChaseBot, _>(&mut env, &mut ChaseForagerAgent::default(), steps, fps);

    eprintln!("\nchase — agentability ({steps} steps, {fps} frame/step):");
    eprintln!("  no-op     {noop}");
    eprintln!("  random    {random}");
    eprintln!("  forager   {forager}");

    assert!(
        forager > noop,
        "the forager should outlive standing still ({forager} vs {noop})"
    );
    assert!(
        forager > random,
        "the forager should out-survive random ({forager} vs {random})"
    );
}
