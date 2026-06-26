//! The `rustz80-cell` micro-VM: compile a dialect program and run one entry on a
//! **flat-RAM Z80** — no ROM, no ULA, no I/O, no syscalls — under a cycle budget,
//! returning a structured [`Report`] (result, cost, symbol layout, touched memory,
//! halt status). Deterministic, side-effect-free, inspectable. Behind `--features cell`
//! (it pulls in the `z80` CPU); the compiler library proper stays dependency-free.
//!
//! [`Runner`] is the compile-once/run-many shape: it owns one 64 KiB bus and, between
//! runs, resets only the bytes the previous run wrote (not the whole 64 KiB) — so the
//! per-run floor is the work, not a fresh allocation. [`run`] is the one-shot convenience.

/// Where the argument trampoline is laid (below `ORG`); `SP_TOP` is the initial stack.
const TRAMPOLINE: u16 = 0x7000;
const SP_TOP: u16 = 0xFFF0;
/// A generous default T-state budget (well past any bounded computation).
pub const DEFAULT_CYCLES: u64 = 2_000_000;

mod cli;
mod config;
mod fast;
mod program;
mod report;
mod runner;
mod state;

pub use cli::{parse_args, run_cli, USAGE};
pub use config::CellConfig;
pub use program::CellProgram;
pub use report::{Fast, Halt, Report, Ty, ABI_VERSION};
pub use runner::{run, CellPool, Runner};
pub use state::{StateCell, STATE_BASE};
