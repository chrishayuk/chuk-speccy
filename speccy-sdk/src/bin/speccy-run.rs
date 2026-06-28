//! `speccy run` — compile a dialect game (or take a `.tap`) and render it **running**
//! to an animated GIF, in one command.
//!
//! ```text
//! speccy-run <game.rs|game.tap> [-o out.gif] [--rom 48.rom]
//!            [--frames 120] [--every 2] [--boot 420]
//! ```
//!
//! The ROM comes from `--rom` or `$SPECTRUM_ROM`. The GIF is headless (no window), so
//! it doubles as a README/agent-episode capture.

use speccy_sdk::run;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut input = None;
    let mut output = None;
    let mut rom_path = None;
    let mut frames = run::DEFAULT_FRAMES;
    let mut every = run::DEFAULT_EVERY;
    let mut boot = run::DEFAULT_BOOT;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--output" => output = args.next(),
            "--rom" => rom_path = args.next(),
            "--frames" => frames = parse_or(args.next(), frames),
            "--every" => every = parse_or(args.next(), every),
            "--boot" => boot = parse_or(args.next(), boot),
            "-h" | "--help" => {
                eprintln!(
                    "usage: speccy-run <game.rs|game.tap> [-o out.gif] [--rom 48.rom] \
                     [--frames N] [--every N] [--boot N]"
                );
                return ExitCode::SUCCESS;
            }
            other => input = Some(other.to_string()),
        }
    }

    let Some(input) = input else {
        eprintln!("error: no input (a game .rs or .tap; try --help)");
        return ExitCode::FAILURE;
    };
    let Some(rom_path) = rom_path.or_else(|| std::env::var("SPECTRUM_ROM").ok()) else {
        eprintln!("error: no ROM — pass --rom <48.rom> or set SPECTRUM_ROM");
        return ExitCode::FAILURE;
    };
    let rom = match std::fs::read(&rom_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: cannot read ROM {rom_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // A `.tap` runs as-is; anything else is dialect source to compile first.
    let tap = if input.ends_with(".tap") {
        match std::fs::read(&input) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: cannot read {input}: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        let src = match std::fs::read_to_string(&input) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: cannot read {input}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let name = stem(&input).to_uppercase();
        match run::compile_source(&src, &name) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: {input}: {e}");
                return ExitCode::FAILURE;
            }
        }
    };

    let out_path = output.unwrap_or_else(|| format!("{}.gif", stem(&input)));
    let gif = match run::render_gif(&tap, &rom, frames, every, boot) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: render: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&out_path, &gif) {
        eprintln!("error: cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    println!("wrote {out_path} ({frames} frames, {} bytes)", gif.len());
    ExitCode::SUCCESS
}

fn parse_or(v: Option<String>, default: usize) -> usize {
    v.and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// The file stem (`path/to/snake.rs` → `snake`).
fn stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("game")
        .to_string()
}
