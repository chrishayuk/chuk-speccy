//! Run outcome types — `Halt`, `Fast`, `Report` (+ `Ty`) and their formatters.
use std::collections::HashMap;

/// The frozen cell ABI / report-schema version (`"abi"` in [`Report::to_json`]). Bump only
/// on a breaking change to the register/memory/capability contract or the JSON shape. See
/// `docs/09-cell80-abi.md`.
pub const ABI_VERSION: u32 = 1;

/// Why a run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Halt {
    /// The entry returned (clean).
    Returned,
    /// The program called `halt(code)` (Cell80 `ED FE` HALT) — an explicit stop.
    Halted(u16),
    /// The T-state budget was reached first.
    CycleBudget,
    /// The `max_touched` memory ceiling was reached.
    MemoryLimit,
}

impl Halt {
    fn as_str(self) -> &'static str {
        match self {
            Halt::Returned => "returned",
            Halt::Halted(_) => "halted",
            Halt::CycleBudget => "cycle_budget",
            Halt::MemoryLimit => "memory_limit",
        }
    }
}

/// A scalar width for typed memory read-back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    U8,
    U16,
    U32,
}

impl Ty {
    /// Parse `u8`/`u16`/`u32`.
    pub fn parse(s: &str) -> Result<Ty, String> {
        match s {
            "u8" => Ok(Ty::U8),
            "u16" => Ok(Ty::U16),
            "u32" => Ok(Ty::U32),
            other => Err(format!("unknown type `{other}` (want u8/u16/u32)")),
        }
    }
}

/// The lightweight outcome of a [`run_fast`](crate::cell::Runner::run_fast): the result registers,
/// T-states, and halt reason — no allocations (no symbol map, size report, or memory
/// diff). For tight agent loops.
#[derive(Debug, Clone, Copy)]
pub struct Fast {
    /// The primary result in `HL`.
    pub result: u16,
    /// All three result registers `[HL, DE, BC]`.
    pub regs: [u16; 3],
    /// T-states elapsed. **See the caveat on [`Report::cycles`] — a deterministic *relative*
    /// cost, not authentic Z80 time; pair it with `trapped_ops` before using as a signal.**
    pub cycles: u64,
    /// Count of cost-bearing `ED FE` host traps (`mul`/`div`/fill) — see [`Report::trapped_ops`].
    pub trapped_ops: u64,
    pub halt: Halt,
}

/// The structured outcome of a [`run`](crate::cell::run).
#[derive(Debug, Clone)]
pub struct Report {
    /// The entry function that was run, and its address.
    pub entry: String,
    pub entry_addr: u16,
    /// The primary result in `HL`.
    pub result: u16,
    /// All three result registers `[HL, DE, BC]` — a `-> (u16, u16, u16)` tuple return
    /// fills all three (`result` is `regs[0]`).
    pub regs: [u16; 3],
    /// Named typed values decoded from post-run memory (empty unless requested via
    /// [`Runner::read_named`](crate::cell::Runner::read_named) / the CLI `--read`).
    pub reads: Vec<(String, u64)>,
    /// T-states elapsed, and the budget it ran under. **Caveat — not authentic Z80 time:**
    /// in Cell mode `*`/`/`/`%` and `[v; N]` fills are `ED FE` host traps serviced natively
    /// and charged a flat ~4 T-states, *not* the real software-routine cost. So `cycles` is
    /// a **deterministic relative cost metric** — correct for liveness (the budget) and
    /// replay, but it must **not** be read as hardware-fidelity time or used as an RL reward
    /// (that would reward shoving work into traps that read as "free"). Pair it with
    /// `trapped_ops` to make a faithful cost signal. See `docs/09-cell80-abi.md`.
    pub cycles: u64,
    pub budget: u64,
    /// How many cost-bearing `ED FE` host traps (`mul`/`div`/fill) the run executed — the
    /// honest companion to `cycles`. Each trap is charged a flat ~4 T-states, so a program
    /// with high `trapped_ops` did real work that `cycles` undercounts. A reward function
    /// should weight or refuse trap-heavy programs rather than treat low `cycles` as cheap.
    pub trapped_ops: u64,
    /// Did the entry return cleanly (`true`)? (Shorthand for `halt == Halt::Returned`.)
    pub returned: bool,
    /// Why the run stopped (returned / cycle budget / memory limit).
    pub halt: Halt,
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
        let halt = match self.halt {
            Halt::Returned => "returned".to_string(),
            Halt::Halted(c) => format!("halted (code {c})"),
            Halt::CycleBudget => format!("CYCLE BUDGET EXCEEDED (≥ {} T-states)", self.budget),
            Halt::MemoryLimit => "MEMORY LIMIT EXCEEDED".to_string(),
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
        let reads = if self.reads.is_empty() {
            String::new()
        } else {
            let r: Vec<String> = self.reads.iter().map(|(n, v)| format!("{n}={v}")).collect();
            format!("\nreads      {}", r.join(", "))
        };
        format!(
            "entry      {} @ {:#06x}\n\
             result     {} ({:#06x})\n\
             regs       HL={} DE={} BC={}\n\
             cycles     {} / {} T-states ({} trapped op(s) — see ABI note)\n\
             halt       {halt}\n\
             code       {} bytes, {} functions\n\
             symbols    {}\n\
             memory     {mem}{reads}",
            self.entry,
            self.entry_addr,
            self.result,
            self.result,
            self.regs[0],
            self.regs[1],
            self.regs[2],
            self.cycles,
            self.budget,
            self.trapped_ops,
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
        let reads: Vec<String> = self
            .reads
            .iter()
            .map(|(n, v)| format!("\"{n}\":{v}"))
            .collect();
        // `halt_code` only appears for an explicit `halt(code)`.
        let halt_code = match self.halt {
            Halt::Halted(c) => format!(",\"halt_code\":{c}"),
            _ => String::new(),
        };
        format!(
            "{{\"abi\":{},\"entry\":\"{}\",\"entry_addr\":{},\"result\":{},\"regs\":[{},{},{}],\"cycles\":{},\
             \"trapped_ops\":{},\"budget\":{},\"halt\":\"{}\"{},\"code_bytes\":{},\"functions\":{},\
             \"symbols\":{{{}}},\"memory_touched\":[{}],\"reads\":{{{}}}}}",
            ABI_VERSION,
            self.entry,
            self.entry_addr,
            self.result,
            self.regs[0],
            self.regs[1],
            self.regs[2],
            self.cycles,
            self.trapped_ops,
            self.budget,
            self.halt.as_str(),
            halt_code,
            self.code_bytes,
            self.fn_count,
            syms.join(","),
            mem.join(","),
            reads.join(","),
        )
    }
}

pub(super) fn sorted_symbols(symbols: &HashMap<String, u16>) -> Vec<(String, u16)> {
    let mut v: Vec<(String, u16)> = symbols.iter().map(|(k, a)| (k.clone(), *a)).collect();
    v.sort_by_key(|(_, a)| *a);
    v
}

/// Coalesce a *sorted* list of distinct addresses into contiguous `(start, end)` ranges.
pub(super) fn coalesce(sorted: &[u16]) -> Vec<(u16, u16)> {
    let mut ranges: Vec<(u16, u16)> = Vec::new();
    for &a in sorted {
        match ranges.last_mut() {
            Some(last) if last.1.checked_add(1) == Some(a) => last.1 = a,
            _ => ranges.push((a, a)),
        }
    }
    ranges
}
