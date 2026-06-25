//! The `rustz80-cell` micro-VM: compile a dialect program and run one entry on a
//! **flat-RAM Z80** — no ROM, no ULA, no I/O, no syscalls — under a cycle budget,
//! returning a structured [`Report`] (result, cost, symbol layout, touched memory,
//! halt status). Deterministic, side-effect-free, inspectable. Behind `--features cell`
//! (it pulls in the `z80` CPU); the compiler library proper stays dependency-free.
//!
//! [`Runner`] is the compile-once/run-many shape: it owns one 64 KiB bus and, between
//! runs, resets only the bytes the previous run wrote (not the whole 64 KiB) — so the
//! per-run floor is the work, not a fresh allocation. [`run`] is the one-shot convenience.

use crate::{compile_program, Program, ORG};
use std::collections::HashMap;

/// CLI usage line, shared by the `rustz80-cell` binary.
pub const USAGE: &str =
    "usage: rustz80-cell run <file.rs> [--entry NAME] [--cycles N] [--args a,b,c] [--json]";

const TRAMPOLINE: u16 = 0x7000;
const SP_TOP: u16 = 0xFFF0;
/// A generous default T-state budget (well past any bounded computation).
pub const DEFAULT_CYCLES: u64 = 2_000_000;

/// The bus the CPU steps against — borrows the [`Runner`]'s reusable buffers, counts
/// T-states, and records each *distinct* written address (for an O(touched) reset and
/// the report).
struct CellBus<'a> {
    mem: &'a mut [u8],
    seen: &'a mut [bool],
    touched: &'a mut Vec<u16>,
    cycles: u64,
}

impl z80::Bus for CellBus<'_> {
    fn read(&mut self, a: u16) -> u8 {
        self.mem[a as usize]
    }
    fn write(&mut self, a: u16, v: u8) {
        self.mem[a as usize] = v;
        if !self.seen[a as usize] {
            self.seen[a as usize] = true;
            self.touched.push(a);
        }
    }
    fn input(&mut self, _: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _: u16, _: u8) {}
    fn contend(&mut self, _: u16, _: u32) {}
    fn tick(&mut self, c: u32) {
        self.cycles += c as u64; // the single source of truth for elapsed time
    }
}

/// A compiled cell, runnable many times. One 64 KiB bus is allocated up front and the
/// code loaded once; each [`run`](Runner::run) resets only the previous run's writes,
/// re-lays the argument trampoline, and steps — so reuse pays for the computation, not a
/// fresh 128 KiB alloc/zero.
pub struct Runner {
    prog: Program,
    mem: Vec<u8>,
    seen: Vec<bool>,   // was this address written this run? (dedup for `touched`)
    touched: Vec<u16>, // distinct addresses written by the last run
}

impl Runner {
    /// Compile `src` and allocate the (reusable) machine, loading the code once.
    pub fn compile(src: &str) -> Result<Self, String> {
        let prog = compile_program(src)?;
        let mut mem = vec![0u8; 0x1_0000];
        let org = ORG as usize;
        mem[org..org + prog.code.len()].copy_from_slice(&prog.code);
        Ok(Runner {
            prog,
            mem,
            seen: vec![false; 0x1_0000],
            touched: Vec::new(),
        })
    }

    /// The compiled program (symbol map, code).
    pub fn program(&self) -> &Program {
        &self.prog
    }

    /// Run `entry` (or `run`/`main` if `None`) with `args` in the calling-convention
    /// registers (`HL`/`DE`/`BC`), bounded by `budget` T-states. Memory the previous
    /// run touched is zeroed first, so repeated runs start from the same clean state.
    pub fn run(
        &mut self,
        entry: Option<&str>,
        args: &[u16],
        budget: u64,
    ) -> Result<Report, String> {
        let entry = match entry {
            Some(e) => e.to_string(),
            None if self.prog.symbols.contains_key("run") => "run".to_string(),
            None if self.prog.symbols.contains_key("main") => "main".to_string(),
            None => return Err("no `run` or `main` entry — pass an explicit entry".into()),
        };
        let entry_addr = *self.prog.symbols.get(&entry).ok_or_else(|| {
            let mut names: Vec<String> = self.prog.symbols.keys().cloned().collect();
            names.sort();
            format!("no entry `{entry}`; available: {}", names.join(", "))
        })?;

        // Reset: zero last run's writes, restore the code (in case it was poked), then
        // lay the trampoline — load args into HL/DE/BC, CALL the entry, HALT on return.
        for &a in &self.touched {
            self.mem[a as usize] = 0;
            self.seen[a as usize] = false;
        }
        self.touched.clear();
        let org = ORG as usize;
        self.mem[org..org + self.prog.code.len()].copy_from_slice(&self.prog.code);

        let mut t = Vec::new();
        const LD: [u8; 3] = [0x21, 0x11, 0x01]; // LD HL,nn / LD DE,nn / LD BC,nn
        for (i, &v) in args.iter().enumerate().take(3) {
            t.push(LD[i]);
            t.push(v as u8);
            t.push((v >> 8) as u8);
        }
        t.push(0xCD); // CALL entry
        t.push(entry_addr as u8);
        t.push((entry_addr >> 8) as u8);
        t.push(0x76); // HALT
        let tr = TRAMPOLINE as usize;
        self.mem[tr..tr + t.len()].copy_from_slice(&t);

        let mut bus = CellBus {
            mem: &mut self.mem,
            seen: &mut self.seen,
            touched: &mut self.touched,
            cycles: 0,
        };
        let mut cpu = z80::Cpu::new();
        cpu.reset();
        cpu.regs.pc = TRAMPOLINE;
        cpu.regs.sp = SP_TOP;
        while !cpu.halted && bus.cycles < budget {
            cpu.step(&mut bus);
        }
        let (cycles, returned, result) = (bus.cycles, cpu.halted, cpu.regs.hl());

        self.touched.sort_unstable();
        Ok(Report {
            entry,
            entry_addr,
            result,
            cycles,
            budget,
            returned,
            code_bytes: self.prog.code.len(),
            fn_count: self.prog.size_report().len(),
            symbols: sorted_symbols(&self.prog.symbols),
            touched: coalesce(&self.touched),
        })
    }
}

/// One-shot convenience: compile `src` and run `entry` once (see [`Runner`] for
/// compile-once/run-many).
pub fn run(src: &str, entry: Option<&str>, args: &[u16], budget: u64) -> Result<Report, String> {
    Runner::compile(src)?.run(entry, args, budget)
}

/// The structured outcome of a [`run`].
#[derive(Debug, Clone)]
pub struct Report {
    /// The entry function that was run, and its address.
    pub entry: String,
    pub entry_addr: u16,
    /// The result in `HL` (a u16; tuple returns also leave `DE`/`BC`, not captured here).
    pub result: u16,
    /// T-states elapsed, and the budget it ran under.
    pub cycles: u64,
    pub budget: u64,
    /// Did the entry return (`true`) or hit the cycle budget first (`false`)?
    pub returned: bool,
    /// Total compiled code size, and the number of functions (incl. monomorphic
    /// instances + the appended runtime).
    pub code_bytes: usize,
    pub fn_count: usize,
    /// The symbol map (name → address), sorted by address.
    pub symbols: Vec<(String, u16)>,
    /// Contiguous RAM ranges written during the run, as `(start, end_inclusive)`.
    pub touched: Vec<(u16, u16)>,
}

impl Report {
    /// A human-readable, aligned summary.
    pub fn to_human(&self) -> String {
        let halt = if self.returned {
            "returned".to_string()
        } else {
            format!("BUDGET EXCEEDED (≥ {} T-states)", self.budget)
        };
        let syms: Vec<String> = self
            .symbols
            .iter()
            .map(|(n, a)| format!("{n}@{a:#06x}"))
            .collect();
        let mem: Vec<String> = self
            .touched
            .iter()
            .map(|(s, e)| format!("{s:#06x}-{e:#06x} ({}B)", e - s + 1))
            .collect();
        let mem = if mem.is_empty() {
            "(none written)".to_string()
        } else {
            mem.join(", ")
        };
        format!(
            "entry      {} @ {:#06x}\n\
             result     {} ({:#06x})\n\
             cycles     {} / {} T-states\n\
             halt       {halt}\n\
             code       {} bytes, {} functions\n\
             symbols    {}\n\
             memory     {mem}",
            self.entry,
            self.entry_addr,
            self.result,
            self.result,
            self.cycles,
            self.budget,
            self.code_bytes,
            self.fn_count,
            syms.join(", "),
        )
    }

    /// A single-line JSON object (for machine/agent consumption).
    pub fn to_json(&self) -> String {
        let syms: Vec<String> = self
            .symbols
            .iter()
            .map(|(n, a)| format!("\"{n}\":{a}"))
            .collect();
        let mem: Vec<String> = self
            .touched
            .iter()
            .map(|(s, e)| format!("[{s},{e}]"))
            .collect();
        format!(
            "{{\"entry\":\"{}\",\"entry_addr\":{},\"result\":{},\"cycles\":{},\"budget\":{},\
             \"halt\":\"{}\",\"code_bytes\":{},\"functions\":{},\"symbols\":{{{}}},\"memory_touched\":[{}]}}",
            self.entry,
            self.entry_addr,
            self.result,
            self.cycles,
            self.budget,
            if self.returned { "returned" } else { "budget_exceeded" },
            self.code_bytes,
            self.fn_count,
            syms.join(","),
            mem.join(","),
        )
    }
}

fn sorted_symbols(symbols: &HashMap<String, u16>) -> Vec<(String, u16)> {
    let mut v: Vec<(String, u16)> = symbols.iter().map(|(k, a)| (k.clone(), *a)).collect();
    v.sort_by_key(|(_, a)| *a);
    v
}

/// Coalesce a *sorted* list of distinct addresses into contiguous `(start, end)` ranges.
fn coalesce(sorted: &[u16]) -> Vec<(u16, u16)> {
    let mut ranges: Vec<(u16, u16)> = Vec::new();
    for &a in sorted {
        match ranges.last_mut() {
            Some(last) if last.1.checked_add(1) == Some(a) => last.1 = a,
            _ => ranges.push((a, a)),
        }
    }
    ranges
}

/// Parse a comma-separated arg list — decimal or `0x`-prefixed hex, each a `u16`.
pub fn parse_args(s: &str) -> Result<Vec<u16>, String> {
    s.split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            let t = t.trim();
            let v = match t.strip_prefix("0x") {
                Some(h) => u16::from_str_radix(h, 16),
                None => t.parse::<u16>(),
            };
            v.map_err(|_| format!("bad arg `{t}` (want a u16, decimal or 0x..)"))
        })
        .collect()
}

/// Parse `run <file> [opts]` argv, run the cell, and return the formatted output
/// (JSON if `--json`, else the human summary). The `rustz80-cell` binary is a shim
/// over this.
pub fn run_cli(args: &[String]) -> Result<String, String> {
    let mut it = args.iter();
    match it.next().map(String::as_str) {
        Some("run") => {}
        Some(other) => return Err(format!("unknown command `{other}`\n{USAGE}")),
        None => return Err(USAGE.into()),
    }
    let file = it.next().ok_or(USAGE)?;
    let mut entry: Option<String> = None;
    let mut cycles = DEFAULT_CYCLES;
    let mut call_args: Vec<u16> = Vec::new();
    let mut json = false;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--entry" => entry = Some(it.next().ok_or("--entry needs a name")?.clone()),
            "--cycles" => {
                cycles = it
                    .next()
                    .ok_or("--cycles needs a number")?
                    .parse()
                    .map_err(|_| "bad --cycles (want a positive integer)")?
            }
            "--args" => call_args = parse_args(it.next().ok_or("--args needs values")?)?,
            "--json" => json = true,
            other => return Err(format!("unknown option `{other}`\n{USAGE}")),
        }
    }
    let src = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let report = run(&src, entry.as_deref(), &call_args, cycles)?;
    Ok(if json {
        report.to_json()
    } else {
        report.to_human()
    })
}
