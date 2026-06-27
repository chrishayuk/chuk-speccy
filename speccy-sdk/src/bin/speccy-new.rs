//! `speccy new` — scaffold a new game from a dual-compile template (L0 ergonomics).
//!
//! ```text
//! speccy-new <name> [--template blank] [-o file.rs]
//! speccy-new --list
//! ```
//!
//! Writes a `speccy-sdk` `Game` that compiles **both** ways out of the box: host
//! (`rustc`) and pure (`speccy-compile <file>` → a bootable `.tap`). Pick a template
//! with `--list`; the default is `blank`.

use speccy_sdk::templates;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut name = None;
    let mut template = "blank".to_string();
    let mut output = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--list" => {
                list();
                return ExitCode::SUCCESS;
            }
            "-t" | "--template" => {
                if let Some(t) = args.next() {
                    template = t;
                }
            }
            "-o" | "--output" => output = args.next(),
            "-h" | "--help" => {
                eprintln!("usage: speccy-new <name> [--template blank] [-o file.rs] | --list");
                return ExitCode::SUCCESS;
            }
            other => name = Some(other.to_string()),
        }
    }

    let Some(name) = name else {
        eprintln!("error: no game name (try --help, or --list for templates)");
        return ExitCode::FAILURE;
    };
    let Some(src) = templates::scaffold(&template, &name) else {
        eprintln!("error: unknown template `{template}`");
        list();
        return ExitCode::FAILURE;
    };

    let out_path = output.unwrap_or_else(|| format!("{}.rs", slug(&name)));
    if let Err(e) = std::fs::write(&out_path, src) {
        eprintln!("error: cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }

    println!("created {out_path} (template: {template})");
    println!("compile it both ways:");
    println!("  pure:  speccy-compile {out_path}            # -> a bootable .tap + .sym.toml");
    println!("  host:  add it to a speccy-sdk Game (see speccy-sdk/tests/dial.rs)");
    ExitCode::SUCCESS
}

fn list() {
    eprintln!("templates:");
    for t in templates::TEMPLATES {
        eprintln!("  {:<8} {}", t.name, t.about);
    }
}

/// A filesystem-friendly lowercase stem (`"My Game"` → `mygame`); falls back to `game`.
fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if s.is_empty() {
        "game".to_string()
    } else {
        s
    }
}
