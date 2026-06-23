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

    println!(
        "{:<28} {:<28} {:>5}  {:>6}  result",
        "query", "title", "fmt", "fill%"
    );
    println!("{}", "-".repeat(86));
    let mut ok = 0usize;
    for q in &titles {
        match wos::fetch(q) {
            Ok(game) => {
                let mut spec = Spectrum::new_48k(&rom);
                load(&mut spec, &game.format, &game.data);
                // Real-time tapes (.tzx) load over many frames; run until the
                // signal is exhausted, then let the title settle.
                let mut f = 0;
                while spec.tape_playing() && f < 80_000 {
                    spec.run_frame();
                    f += 1;
                }
                for _ in 0..700 {
                    spec.run_frame();
                }
                let fill = fill_percent(&spec.screen_indexed());
                let verdict = if fill >= 1 {
                    "OK"
                } else if model_128k(&game.format, &game.data) {
                    "128K title — needs the 128K model (unsupported)"
                } else {
                    "BLANK?"
                };
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
                println!(
                    "{:<28} {:<28} {:>5}  {:>6}  FETCH FAIL: {e}",
                    truncate(q, 28),
                    "",
                    "-",
                    "-"
                );
            }
        }
    }
    println!("\n{ok}/{} produced a non-blank screen", titles.len());
}

/// Heuristic: does this image require the 128K model (not yet emulated, so it
/// boots into 48K and renders blank)? A `.z80` v2/v3 carries a hardware byte; a
/// 128K `.sna` is larger than the fixed 48K size. `.tap`/`.tzx` don't encode it.
fn model_128k(fmt: &str, data: &[u8]) -> bool {
    match fmt {
        "z80" => z80_is_128k(data),
        "sna" => data.len() > 49179, // a 48K .sna is exactly 49179 bytes
        _ => false,
    }
}

fn z80_is_128k(d: &[u8]) -> bool {
    if d.len() < 35 {
        return false;
    }
    let ext_len = u16::from_le_bytes([d[30], d[31]]);
    if ext_len == 0 {
        return false; // v1 header — always 48K
    }
    let hw = d[34];
    match ext_len {
        23 => hw >= 3, // v2: 3 = 128K, 4 = 128K+IF1
        _ => hw >= 4,  // v3: 4 = 128K, 5 = 128K+IF1, 6 = 128K+MGT, …
    }
}

/// Load a game by format (`.tap` boots + trap-loads; `.tzx` loads real-time via
/// the tape signal; snapshots load directly).
fn load(spec: &mut Spectrum, fmt: &str, data: &[u8]) {
    match fmt {
        "tap" => {
            for _ in 0..250 {
                spec.run_frame();
            }
            if spec.load_tap(data).is_ok() {
                spec.autoload_tape();
            }
        }
        "tzx" => {
            for _ in 0..250 {
                spec.run_frame();
            }
            spec.autoload_tape();
            let _ = spec.play_tape("tzx", data);
        }
        _ => {
            let _ = spec.load_snapshot(fmt, data);
        }
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

#[cfg(test)]
mod tests {
    use super::z80_is_128k;

    fn z80(ext_len: u16, hw: u8) -> Vec<u8> {
        let mut d = vec![0u8; 40];
        d[30..32].copy_from_slice(&ext_len.to_le_bytes());
        d[34] = hw;
        d
    }

    #[test]
    fn detects_128k_z80() {
        assert!(!z80_is_128k(&z80(0, 9))); // v1 header → always 48K
        assert!(!z80_is_128k(&z80(23, 0))); // v2, 48K
        assert!(z80_is_128k(&z80(23, 3))); // v2, 128K
        assert!(!z80_is_128k(&z80(54, 0))); // v3, 48K
        assert!(z80_is_128k(&z80(54, 4))); // v3, 128K
        assert!(!z80_is_128k(&[0u8; 10])); // too short
    }
}
