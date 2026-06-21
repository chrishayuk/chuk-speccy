//! Headless "does it all work?" check: fetch a list of games from World of
//! Spectrum, load each into the core, run a few seconds, and report whether a
//! non-blank screen rendered. No window — runnable anywhere (CI, ssh).
//!
//! Usage: `speccy-library <48.rom> ["Game One" "Game Two" …]`
//!   With no titles, runs a curated set of classics.

use spectrum::Spectrum;

/// Curated classics — known to be available as trap-loadable `.tap`/snapshots.
const DEFAULT_GAMES: &[&str] = &[
    "Skool Daze",
    "Renegade",
    "Spy vs Spy",
    "Chaos",
    "Green Beret",
    "Jet Set Willy",
    "Spellbound",
    "Daley Thompson's Decathlon",
    "Manic Miner",
    "Knight Lore",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let rom_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: speccy-library <48.rom> [\"Game\" …]");
            std::process::exit(2);
        }
    };
    let rom = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("could not read ROM {rom_path}: {e}");
        std::process::exit(1);
    });

    let titles: Vec<String> = args.collect();
    let titles: Vec<&str> = if titles.is_empty() {
        DEFAULT_GAMES.to_vec()
    } else {
        titles.iter().map(String::as_str).collect()
    };

    println!("{:<28} {:<28} {:>5}  {:>6}  result", "query", "title", "fmt", "fill%");
    println!("{}", "-".repeat(86));
    let mut ok = 0usize;
    for q in &titles {
        match wos::fetch(q) {
            Ok(game) => {
                let mut spec = Spectrum::new_48k(&rom);
                load(&mut spec, &game.format, &game.data);
                // Let the loader/title settle.
                for _ in 0..700 {
                    spec.run_frame();
                }
                let fill = fill_percent(&spec.screen_indexed());
                let verdict = if fill >= 1 { "OK" } else { "BLANK?" };
                if fill >= 1 {
                    ok += 1;
                }
                let year = game.year.map(|y| y.to_string()).unwrap_or_default();
                println!(
                    "{:<28} {:<28} {:>5}  {:>5}%  {}",
                    truncate(q, 28),
                    truncate(&format!("{} {}", game.title, year), 28),
                    game.format,
                    fill,
                    verdict,
                );
            }
            Err(e) => {
                println!("{:<28} {:<28} {:>5}  {:>6}  FETCH FAIL: {e}", truncate(q, 28), "", "-", "-");
            }
        }
    }
    println!("\n{ok}/{} produced a non-blank screen", titles.len());
}

/// Load a game by format (`.tap` boots + trap-loads; snapshots load directly).
fn load(spec: &mut Spectrum, fmt: &str, data: &[u8]) {
    if fmt == "tap" {
        for _ in 0..250 {
            spec.run_frame();
        }
        if spec.load_tap(data).is_ok() {
            spec.autoload_tape();
        }
    } else {
        let _ = spec.load_snapshot(fmt, data);
    }
}

/// Percentage of screen pixels that are non-zero (a blank screen is ~0%).
fn fill_percent(indexed: &[u8]) -> usize {
    if indexed.is_empty() {
        return 0;
    }
    let set = indexed.iter().filter(|&&b| b != 0).count();
    set * 100 / indexed.len()
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n - 1])
    }
}
