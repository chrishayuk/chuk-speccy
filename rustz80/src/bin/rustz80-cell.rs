//! `rustz80-cell` — a deterministic, headless micro-VM runner for the dialect.
//!
//! Compile a restricted-Rust source and run one entry function on a **flat-RAM Z80** —
//! no ROM, no ULA, no I/O, no syscalls. Reports the result, the cost (T-states, code
//! bytes), the symbol layout, and which memory the run touched. Bounded by a cycle
//! budget, reproducible, side-effect-free: a "safe executable thought bubble" an agent
//! can program against and measure.
//!
//! ```text
//! rustz80-cell run prog.rs [--entry run] [--cycles N] [--args a,b,c] [--json]
//! ```
//!
//! All logic lives in [`rustz80::cell`] (tested); this is a thin CLI shim. Behind
//! `--features cell` (it pulls in the `z80` CPU).

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match rustz80::cell::run_cli(&args) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("rustz80-cell: {e}");
            ExitCode::FAILURE
        }
    }
}
