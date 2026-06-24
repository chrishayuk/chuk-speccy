//! The `rustz80-cell` micro-VM: compile a dialect program and run one entry on a
//! **flat-RAM Z80** — no ROM, no ULA, no I/O, no syscalls — under a cycle budget,
//! returning a structured [`Report`] (result, cost, symbol layout, touched memory,
//! halt status). Deterministic, side-effect-free, inspectable. Behind `--features cell`
//! (it pulls in the `z80` CPU); the compiler library proper stays dependency-free.

use crate::ORG;
use std::collections::HashMap;

/// CLI usage line, shared by the `rustz80-cell` binary.
pub const USAGE: &str =
    "usage: rustz80-cell run <file.rs> [--entry NAME] [--cycles N] [--args a,b,c] [--json]";

const TRAMPOLINE: u16 = 0x7000;
const SP_TOP: u16 = 0xFFF0;
/// A generous default T-state budget (well past any bounded computation).
pub const DEFAULT_CYCLES: u64 = 2_000_000;

/// A flat 64 KiB RAM bus with a T-state counter and per-address write tracking — the
/// whole machine the cell runs on (ports float high, no contention).
struct Cell {
    mem: Vec<u8>,
    cycles: u64,
    touched: Vec<bool>,
}

impl z80::Bus for Cell {
    fn read(&mut self, a: u16) -> u8 {
        self.mem[a as usize]
    }
    fn write(&mut self, a: u16, v: u8) {
        self.mem[a as usize] = v;
        self.touched[a as usize] = true;
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

/// Compile `src` and run `entry` (or `run`/`main` if `None`) with `args` in the
/// calling-convention registers (`HL`/`DE`/`BC`), bounded by `budget` T-states.
pub fn run(src: &str, entry: Option<&str>, args: &[u16], budget: u64) -> Result<Report, String> {
    let prog = crate::compile_program(src)?;

    let entry = match entry {
        Some(e) => e.to_string(),
        None if prog.symbols.contains_key("run") => "run".to_string(),
        None if prog.symbols.contains_key("main") => "main".to_string(),
        None => return Err("no `run` or `main` entry — pass an explicit entry".into()),
    };
    let entry_addr = *prog.symbols.get(&entry).ok_or_else(|| {
        let mut names: Vec<String> = prog.symbols.keys().cloned().collect();
        names.sort();
        format!("no entry `{entry}`; available: {}", names.join(", "))
    })?;

    let mut bus = Cell {
        mem: vec![0u8; 0x1_0000],
        cycles: 0,
        touched: vec![false; 0x1_0000],
    };

    // Trampoline at 0x7000: load args into HL/DE/BC, CALL the entry, HALT on return.
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
    bus.mem[tr..tr + t.len()].copy_from_slice(&t);

    let org = ORG as usize;
    bus.mem[org..org + prog.code.len()].copy_from_slice(&prog.code);

    let mut cpu = z80::Cpu::new();
    cpu.reset();
    cpu.regs.pc = TRAMPOLINE;
    cpu.regs.sp = SP_TOP;
    while !cpu.halted && bus.cycles < budget {
        cpu.step(&mut bus);
    }

    Ok(Report {
        entry,
        entry_addr,
        result: cpu.regs.hl(),
        cycles: bus.cycles,
        budget,
        returned: cpu.halted,
        code_bytes: prog.code.len(),
        fn_count: prog.size_report().len(),
        symbols: sorted_symbols(&prog.symbols),
        touched: coalesce(&bus.touched),
    })
}

fn sorted_symbols(symbols: &HashMap<String, u16>) -> Vec<(String, u16)> {
    let mut v: Vec<(String, u16)> = symbols.iter().map(|(k, a)| (k.clone(), *a)).collect();
    v.sort_by_key(|(_, a)| *a);
    v
}

/// Coalesce a per-address touched bitmap into contiguous `(start, end_inclusive)` ranges.
fn coalesce(touched: &[bool]) -> Vec<(u16, u16)> {
    let mut ranges = Vec::new();
    let mut i = 0usize;
    while i < touched.len() {
        if touched[i] {
            let start = i;
            while i < touched.len() && touched[i] {
                i += 1;
            }
            ranges.push((start as u16, (i - 1) as u16));
        } else {
            i += 1;
        }
    }
    ranges
}
