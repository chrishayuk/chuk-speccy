//! `speccy-compile` — compile a rustz80-dialect `.rs` file to a bootable `.tap`.
//!
//! ```text
//! speccy-compile game.rs [-o game.tap] [--entry main] [--name GAME]
//! ```
//!
//! The dialect file must contain the entry function (default `main`, no args).
//! Load the resulting tape in `speccy-gui` (or any Spectrum) to run it.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut input = None;
    let mut output = None;
    let mut entry = "main".to_string();
    let mut name = None;

    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--output" => output = args.next(),
            "--entry" => {
                if let Some(e) = args.next() {
                    entry = e;
                }
            }
            "--name" => name = args.next(),
            "-h" | "--help" => {
                eprintln!(
                    "usage: speccy-compile <input.rs> [-o out.tap] [--entry main] [--name GAME]"
                );
                return ExitCode::SUCCESS;
            }
            other => input = Some(other.to_string()),
        }
    }

    let Some(input) = input else {
        eprintln!("error: no input file (try --help)");
        return ExitCode::FAILURE;
    };
    let out_path = output.unwrap_or_else(|| {
        let stem = input.strip_suffix(".rs").unwrap_or(&input);
        format!("{stem}.tap")
    });
    // Tape name: up to 10 uppercase chars from the output stem.
    let tape_name = name.unwrap_or_else(|| {
        let stem = std::path::Path::new(&out_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("GAME");
        stem.to_uppercase().chars().take(10).collect()
    });

    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // An `impl Game` compiles via the SDK-prelude path (frame loop) and also emits
    // a `.sym.toml` symbol map (the env bridge); otherwise the file needs a no-arg
    // `fn main` entry.
    let is_game = rustz80::has_game(&src);
    let (tap, symbols) = if is_game {
        match rustz80::compile_game_with_symbols(&src, &tape_name) {
            Ok((t, s)) => (t, Some(s)),
            Err(e) => {
                eprintln!("error: {input}: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        match rustz80::compile_to_tap(&src, &entry, &tape_name) {
            Ok(t) => (t, None),
            Err(e) => {
                eprintln!("error: {input}: {e}");
                return ExitCode::FAILURE;
            }
        }
    };
    if let Err(e) = std::fs::write(&out_path, &tap) {
        eprintln!("error: cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    // Sidecar the symbol map next to the tape (`game.tap` → `game.sym.toml`).
    if let Some(symbols) = &symbols {
        let sym_path = out_path
            .strip_suffix(".tap")
            .unwrap_or(&out_path)
            .to_string()
            + ".sym.toml";
        if let Err(e) = std::fs::write(&sym_path, symbols.to_toml()) {
            eprintln!("error: cannot write {sym_path}: {e}");
            return ExitCode::FAILURE;
        }
        eprintln!("wrote {sym_path} ({} fields)", symbols.fields.len());
    }
    let how = if is_game {
        "impl Game".to_string()
    } else {
        format!("entry `{entry}`")
    };
    eprintln!("wrote {out_path} ({} bytes, {how})", tap.len());
    ExitCode::SUCCESS
}
