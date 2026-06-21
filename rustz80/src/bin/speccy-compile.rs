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
                eprintln!("usage: speccy-compile <input.rs> [-o out.tap] [--entry main] [--name GAME]");
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
        let stem = std::path::Path::new(&out_path).file_stem().and_then(|s| s.to_str()).unwrap_or("GAME");
        stem.to_uppercase().chars().take(10).collect()
    });

    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let tap = match rustz80::compile_to_tap(&src, &entry, &tape_name) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&out_path, &tap) {
        eprintln!("error: cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {out_path} ({} bytes, entry `{entry}`)", tap.len());
    ExitCode::SUCCESS
}
