//! The `rustz80-cell` micro-VM: compile a dialect program and run one entry on a
//! **flat-RAM Z80** — no ROM, no ULA, no I/O, no syscalls — under a cycle budget,
//! returning a structured [`Report`] (result, cost, symbol layout, touched memory,
//! halt status). Deterministic, side-effect-free, inspectable. Behind `--features cell`
//! (it pulls in the `z80` CPU); the compiler library proper stays dependency-free.
//!
//! [`Runner`] is the compile-once/run-many shape: it owns one 64 KiB bus and, between
//! runs, resets only the bytes the previous run wrote (not the whole 64 KiB) — so the
//! per-run floor is the work, not a fresh allocation. [`run`] is the one-shot convenience.

use crate::{Program, ORG};
use std::collections::HashMap;

/// CLI usage line, shared by the `rustz80-cell` binary.
pub const USAGE: &str = "usage: rustz80-cell run <file.rs> [--entry NAME] [--cycles N] \
     [--args a,b,c] [--set addr:ty=val,...] [--read name@addr:ty,...] [--json]\n  \
     safety (sandboxed by default): [--allow-raw-memory] [--allow-ports] \
     [--max-code-bytes N] [--max-touched N]";

const TRAMPOLINE: u16 = 0x7000;
const SP_TOP: u16 = 0xFFF0;
/// A generous default T-state budget (well past any bounded computation).
pub const DEFAULT_CYCLES: u64 = 2_000_000;

/// Safety policy for a cell. Games need raw memory; general agent cells usually do not —
/// so the intrinsics are **capability-gated, off by default** ([`CellConfig::sandboxed`]),
/// and resource ceilings are explicit. The cycle budget (passed to [`Runner::run`]) is the
/// deterministic liveness guard; these are the rest.
#[derive(Debug, Clone)]
pub struct CellConfig {
    /// Allow `poke`/`peek` (raw memory access).
    pub allow_raw_memory: bool,
    /// Allow `inport` (I/O ports).
    pub allow_ports: bool,
    /// Reject if the compiled code exceeds this many bytes.
    pub max_code_bytes: Option<usize>,
    /// Abort the run if it writes more than this many distinct addresses.
    pub max_touched: Option<usize>,
}

impl CellConfig {
    /// Deny raw memory + ports, with tight ceilings — the default for untrusted cells.
    pub fn sandboxed() -> Self {
        CellConfig {
            allow_raw_memory: false,
            allow_ports: false,
            max_code_bytes: Some(4096),
            max_touched: Some(4096),
        }
    }
    /// Allow everything, no ceilings — for trusted/game code (matches the pre-policy
    /// behaviour).
    pub fn permissive() -> Self {
        CellConfig {
            allow_raw_memory: true,
            allow_ports: true,
            max_code_bytes: None,
            max_touched: None,
        }
    }
}

impl Default for CellConfig {
    /// Safe by default.
    fn default() -> Self {
        Self::sandboxed()
    }
}

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

/// Which capability-gated intrinsics a source uses (`poke`/`peek`/`inport`).
#[derive(Default)]
struct Caps {
    raw_memory: bool, // poke / peek
    ports: bool,      // inport
}

impl<'ast> syn::visit::Visit<'ast> for Caps {
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*c.func {
            match p.path.get_ident().map(|i| i.to_string()).as_deref() {
                Some("poke") | Some("peek") => self.raw_memory = true,
                Some("inport") => self.ports = true,
                _ => {}
            }
        }
        syn::visit::visit_expr_call(self, c); // recurse into nested calls
    }
}

/// Check a parsed file against a config's capability gates (walks for the gated
/// intrinsics).
fn check_caps(file: &syn::File, cfg: &CellConfig) -> Result<(), String> {
    let mut caps = Caps::default();
    syn::visit::visit_file(&mut caps, file);
    if caps.raw_memory && !cfg.allow_raw_memory {
        return Err("raw memory (`poke`/`peek`) is not allowed (enable allow_raw_memory)".into());
    }
    if caps.ports && !cfg.allow_ports {
        return Err("I/O ports (`inport`) are not allowed (enable allow_ports)".into());
    }
    Ok(())
}

/// The bus the CPU steps against — borrows the [`Runner`]'s reusable buffers, counts
/// T-states, and records each *distinct* written address (for an O(touched) reset and
/// the report).
struct CellBus<'a> {
    mem: &'a mut [u8],
    seen: &'a mut [bool],
    touched: &'a mut Vec<u16>,
    cycles: u64,
    halt: Option<u16>, // set by the HALT trap (`halt(code)`)
}

impl CellBus<'_> {
    /// Write a byte and record it as touched (so the next run resets it) — shared by the
    /// CPU's `write` and the fill traps.
    fn touch_write(&mut self, a: u16, v: u8) {
        self.mem[a as usize] = v;
        if !self.seen[a as usize] {
            self.seen[a as usize] = true;
            self.touched.push(a);
        }
    }
}

impl z80::Bus for CellBus<'_> {
    fn read(&mut self, a: u16) -> u8 {
        self.mem[a as usize]
    }
    fn write(&mut self, a: u16, v: u8) {
        self.touch_write(a, v);
    }
    fn input(&mut self, _: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _: u16, _: u8) {}
    fn contend(&mut self, _: u16, _: u32) {}
    fn tick(&mut self, c: u32) {
        self.cycles += c as u64; // the single source of truth for elapsed time
    }
    /// Cell80 host intrinsics (`ED FE`, id in `A`). Matches `spectrum::host::math_traps`:
    /// `0x10` MUL16 (`HL = BC*DE`), `0x11` DIVMOD16 (`HL = BC/DE`, `DE = BC%DE`). Done
    /// host-native, so a `var*var` multiply/divide costs a few T-states instead of a
    /// software loop.
    fn host_trap(&mut self, regs: &mut z80::Regs) -> u32 {
        match regs.a {
            0x10 => {
                let p = regs.bc().wrapping_mul(regs.de());
                regs.set_hl(p);
            }
            0x11 => {
                let (bc, de) = (regs.bc(), regs.de());
                match bc.checked_div(de) {
                    Some(q) => {
                        regs.set_hl(q);
                        regs.set_de(bc % de);
                    }
                    None => regs.set_hl(0xFFFF), // divide-by-zero (a bug) — bounded, not a panic
                }
            }
            0x20 => {
                // FILL16: BC slots (2-byte words) of DE at HL — array `[v; N]` init.
                let (mut addr, count, val) = (regs.hl(), regs.bc(), regs.de());
                for _ in 0..count {
                    self.touch_write(addr, val as u8);
                    self.touch_write(addr.wrapping_add(1), (val >> 8) as u8);
                    addr = addr.wrapping_add(2);
                }
            }
            0x30 => self.halt = Some(regs.hl()), // HALT: stop with status code HL
            _ => {}
        }
        4 // a fast hardware op (cell cycle accounting)
    }
}

/// A **compiled** cell: the result of parse + lower + codegen under a policy. Cheap to
/// clone and cache (e.g. by source hash) — re-running a known snippet then skips the
/// (syn-parse-dominated, ~16 µs) compile. Turn one into a runnable machine with
/// [`Runner::new`].
#[derive(Clone)]
pub struct CellProgram {
    prog: Program,
    cfg: CellConfig,
}

impl CellProgram {
    /// Compile `src` with the **permissive** policy (raw memory + ports allowed, no
    /// ceilings) — for trusted/game code.
    pub fn compile(src: &str) -> Result<Self, String> {
        Self::compile_with_config(src, CellConfig::permissive())
    }

    /// Compile `src` under `cfg`: enforce its capability gates (`poke`/`peek`/`inport`)
    /// and `max_code_bytes`. Parses once (shared by the cap scan and the compile).
    pub fn compile_with_config(src: &str, cfg: CellConfig) -> Result<Self, String> {
        let file: syn::File = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
        check_caps(&file, &cfg)?;
        // The cell runs in Cell80 mode: `*`/`/`/`%` lower to `ED FE` host traps that the
        // bus services natively (no software mul/div runtime appended).
        let prog = crate::compile_file(&file, crate::Target::Cell)?;
        if let Some(max) = cfg.max_code_bytes {
            if prog.code.len() > max {
                return Err(format!(
                    "code is {} bytes, over the {max}-byte limit",
                    prog.code.len()
                ));
            }
        }
        Ok(CellProgram { prog, cfg })
    }

    /// The underlying program (symbol map, code).
    pub fn program(&self) -> &Program {
        &self.prog
    }

    /// Serialize to a compact, self-contained **image** — code + symbols + policy, no syn,
    /// no source. Cache it (by hash), ship it, retrieve it; [`from_bytes`](Self::from_bytes)
    /// reloads it in ~µs, skipping the parse-dominated (~16 µs) compile. The cell
    /// "cartridge": a few dozen bytes you can hash, index, and hand around cheaply.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(IMAGE_MAGIC);
        b.push(IMAGE_VER);
        b.extend_from_slice(&(self.prog.code.len() as u16).to_le_bytes());
        b.extend_from_slice(&self.prog.code);
        let syms = sorted_symbols(&self.prog.symbols); // deterministic → stable hash
        b.extend_from_slice(&(syms.len() as u16).to_le_bytes());
        for (name, addr) in &syms {
            b.push(name.len() as u8);
            b.extend_from_slice(name.as_bytes());
            b.extend_from_slice(&addr.to_le_bytes());
        }
        let c = &self.cfg;
        let flags = (c.allow_raw_memory as u8)
            | (c.allow_ports as u8) << 1
            | (c.max_code_bytes.is_some() as u8) << 2
            | (c.max_touched.is_some() as u8) << 3;
        b.push(flags);
        b.extend_from_slice(&(c.max_code_bytes.unwrap_or(0) as u32).to_le_bytes());
        b.extend_from_slice(&(c.max_touched.unwrap_or(0) as u32).to_le_bytes());
        b
    }

    /// Reload an image written by [`to_bytes`](Self::to_bytes) — no parse, no compile.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut r = ImageReader { b: bytes, i: 0 };
        if r.take(4)? != IMAGE_MAGIC {
            return Err("not a CZ80 cell image".into());
        }
        let ver = r.u8()?;
        if ver != IMAGE_VER {
            return Err(format!("unsupported cell-image version {ver}"));
        }
        let code_len = r.u16()? as usize;
        let code = r.take(code_len)?.to_vec();
        let nsym = r.u16()?;
        let mut symbols = HashMap::with_capacity(nsym as usize);
        for _ in 0..nsym {
            let nlen = r.u8()? as usize;
            let name = std::str::from_utf8(r.take(nlen)?)
                .map_err(|_| "bad symbol name in image")?
                .to_string();
            symbols.insert(name, r.u16()?);
        }
        let flags = r.u8()?;
        let max_code = r.u32()? as usize;
        let max_touched = r.u32()? as usize;
        Ok(CellProgram {
            prog: Program { code, symbols },
            cfg: CellConfig {
                allow_raw_memory: flags & 1 != 0,
                allow_ports: flags & 2 != 0,
                max_code_bytes: (flags & 4 != 0).then_some(max_code),
                max_touched: (flags & 8 != 0).then_some(max_touched),
            },
        })
    }
}

const IMAGE_MAGIC: &[u8; 4] = b"CZ80";
const IMAGE_VER: u8 = 1;

/// A tiny bounds-checked cursor for [`CellProgram::from_bytes`].
struct ImageReader<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> ImageReader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).ok_or("cell image truncated")?;
        let s = self.b.get(self.i..end).ok_or("cell image truncated")?;
        self.i = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
}

/// A compiled cell, runnable many times. One 64 KiB bus is allocated up front and the
/// code loaded once; each [`run`](Runner::run) resets only the previous run's writes,
/// re-lays the argument trampoline, and steps — so reuse pays for the computation, not a
/// fresh 128 KiB alloc/zero.
pub struct Runner {
    prog: Program,
    cfg: CellConfig,
    mem: Vec<u8>,
    seen: Vec<bool>,   // was this address written this run? (dedup for `touched`)
    touched: Vec<u16>, // distinct addresses written by the last run
}

impl Runner {
    /// Instantiate a runnable machine from an already-[`compile`](CellProgram::compile)d
    /// program — **cheap**: allocate the bus and load the code, *no parse/compile*. The
    /// way to skip cold setup for a cached snippet (compile once → `Runner::new` many).
    pub fn new(program: &CellProgram) -> Self {
        let mut mem = vec![0u8; 0x1_0000];
        let org = ORG as usize;
        mem[org..org + program.prog.code.len()].copy_from_slice(&program.prog.code);
        Runner {
            prog: program.prog.clone(),
            cfg: program.cfg.clone(),
            mem,
            seen: vec![false; 0x1_0000],
            touched: Vec::new(),
        }
    }

    /// Compile `src` (permissive) and instantiate — back-compat for trusted/game code.
    /// Untrusted cells should use [`compile_with_config`](Runner::compile_with_config).
    pub fn compile(src: &str) -> Result<Self, String> {
        Ok(Self::new(&CellProgram::compile(src)?))
    }

    /// Compile `src` under `cfg` and instantiate.
    pub fn compile_with_config(src: &str, cfg: CellConfig) -> Result<Self, String> {
        Ok(Self::new(&CellProgram::compile_with_config(src, cfg)?))
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
        self.run_with_inputs(entry, args, &[], budget)
    }

    /// Like [`run`](Runner::run), but first writes typed `inputs` `(addr, ty, value)` into
    /// memory after the reset — so a cell whose state lives at a known base reads
    /// caller-supplied values (resolve field addresses with [`crate::struct_layout`]).
    pub fn run_with_inputs(
        &mut self,
        entry: Option<&str>,
        args: &[u16],
        inputs: &[(u16, Ty, u64)],
        budget: u64,
    ) -> Result<Report, String> {
        let (entry, entry_addr) = self.resolve_entry(entry)?;
        let (regs, cycles, halt) = self.exec(entry_addr, args, inputs, budget);
        // Observability: clone the symbol map + size report + coalesce the memory diff.
        // The hot path skips all of this — see `run_fast`.
        self.touched.sort_unstable();
        Ok(Report {
            entry,
            entry_addr,
            result: regs[0],
            regs,
            cycles,
            budget,
            returned: halt == Halt::Returned,
            halt,
            code_bytes: self.prog.code.len(),
            fn_count: self.prog.size_report().len(),
            symbols: sorted_symbols(&self.prog.symbols),
            touched: coalesce(&self.touched),
            reads: Vec::new(),
        })
    }

    /// The **hot path**: run `entry` and return just the result registers, cycles, and
    /// halt — *no* symbol-map clone, size report, or memory-diff (no per-call
    /// allocations). For tight agent loops over many candidates (see `run` for the rich
    /// [`Report`]).
    pub fn run_fast(
        &mut self,
        entry: Option<&str>,
        args: &[u16],
        budget: u64,
    ) -> Result<Fast, String> {
        let entry_addr = self.resolve_addr(entry)?;
        Ok(self.exec_fast(entry_addr, args, budget))
    }

    /// Run the same entry over many argument sets, reusing **all** setup — the "score N
    /// candidates" path. The entry is resolved once (no per-call name allocation/lookup);
    /// each element is reset + re-run. One [`Fast`] per input set, in order.
    pub fn run_many_fast(
        &mut self,
        entry: Option<&str>,
        arg_sets: &[&[u16]],
        budget: u64,
    ) -> Result<Vec<Fast>, String> {
        let entry_addr = self.resolve_addr(entry)?;
        Ok(arg_sets
            .iter()
            .map(|args| self.exec_fast(entry_addr, args, budget))
            .collect())
    }

    /// `exec` + pack a [`Fast`] — the shared body of `run_fast`/`run_many_fast`.
    fn exec_fast(&mut self, entry_addr: u16, args: &[u16], budget: u64) -> Fast {
        let (regs, cycles, halt) = self.exec(entry_addr, args, &[], budget);
        Fast {
            result: regs[0],
            regs,
            cycles,
            halt,
        }
    }

    /// Resolve just the entry **address** (default `run`, then `main`) — no name
    /// allocation, for the hot path. The named [`resolve_entry`](Self::resolve_entry) is
    /// for the `Report` path, which needs the name.
    fn resolve_addr(&self, entry: Option<&str>) -> Result<u16, String> {
        let name = match entry {
            Some(e) => e,
            None if self.prog.symbols.contains_key("run") => "run",
            None if self.prog.symbols.contains_key("main") => "main",
            None => return Err("no `run` or `main` entry — pass an explicit entry".into()),
        };
        self.prog.symbols.get(name).copied().ok_or_else(|| {
            let mut names: Vec<String> = self.prog.symbols.keys().cloned().collect();
            names.sort();
            format!("no entry `{name}`; available: {}", names.join(", "))
        })
    }

    /// Resolve the entry name + address (defaulting to `run`, then `main`).
    fn resolve_entry(&self, entry: Option<&str>) -> Result<(String, u16), String> {
        let entry = match entry {
            Some(e) => e.to_string(),
            None if self.prog.symbols.contains_key("run") => "run".to_string(),
            None if self.prog.symbols.contains_key("main") => "main".to_string(),
            None => return Err("no `run` or `main` entry — pass an explicit entry".into()),
        };
        let addr = *self.prog.symbols.get(&entry).ok_or_else(|| {
            let mut names: Vec<String> = self.prog.symbols.keys().cloned().collect();
            names.sort();
            format!("no entry `{entry}`; available: {}", names.join(", "))
        })?;
        Ok((entry, addr))
    }

    /// Reset (zero last run's writes + restore code), lay the trampoline + inputs, and
    /// step the CPU. Returns `(regs[HL,DE,BC], cycles, halt)`. The allocation-free core
    /// shared by `run`/`run_fast` — the per-call cost is the computation, not bookkeeping.
    fn exec(
        &mut self,
        entry_addr: u16,
        args: &[u16],
        inputs: &[(u16, Ty, u64)],
        budget: u64,
    ) -> ([u16; 3], u64, Halt) {
        // Reset only the bytes the previous run wrote, then restore the code (in case it
        // was poked).
        for &a in &self.touched {
            self.mem[a as usize] = 0;
            self.seen[a as usize] = false;
        }
        self.touched.clear();
        let org = ORG as usize;
        self.mem[org..org + self.prog.code.len()].copy_from_slice(&self.prog.code);

        // Trampoline written straight to memory (no per-call Vec): load args into
        // HL/DE/BC, CALL the entry, HALT on return.
        const LD: [u8; 3] = [0x21, 0x11, 0x01];
        let mut p = TRAMPOLINE as usize;
        for (i, &v) in args.iter().enumerate().take(3) {
            self.mem[p] = LD[i];
            self.mem[p + 1] = v as u8;
            self.mem[p + 2] = (v >> 8) as u8;
            p += 3;
        }
        self.mem[p] = 0xCD; // CALL entry
        self.mem[p + 1] = entry_addr as u8;
        self.mem[p + 2] = (entry_addr >> 8) as u8;
        self.mem[p + 3] = 0x76; // HALT

        // Typed inputs (after the reset, so they survive it; marked touched so the next
        // run cleans them). Little-endian, low byte first.
        for &(addr, ty, val) in inputs {
            let bytes = match ty {
                Ty::U8 => 1,
                Ty::U16 => 2,
                Ty::U32 => 4,
            };
            for i in 0..bytes {
                let a = addr.wrapping_add(i as u16) as usize;
                self.mem[a] = (val >> (8 * i)) as u8;
                if !self.seen[a] {
                    self.seen[a] = true;
                    self.touched.push(a as u16);
                }
            }
        }

        let max_touched = self.cfg.max_touched;
        let mut bus = CellBus {
            mem: &mut self.mem,
            seen: &mut self.seen,
            touched: &mut self.touched,
            cycles: 0,
            halt: None,
        };
        let mut cpu = z80::Cpu::new();
        cpu.reset();
        cpu.regs.pc = TRAMPOLINE;
        cpu.regs.sp = SP_TOP;
        let mut mem_limit = false;
        while !cpu.halted && bus.cycles < budget {
            cpu.step(&mut bus);
            if bus.halt.is_some() {
                break; // `halt(code)` — stop right after the trap
            }
            if matches!(max_touched, Some(m) if bus.touched.len() > m) {
                mem_limit = true;
                break;
            }
        }
        let halt = if let Some(code) = bus.halt {
            Halt::Halted(code)
        } else if cpu.halted {
            Halt::Returned
        } else if mem_limit {
            Halt::MemoryLimit
        } else {
            Halt::CycleBudget
        };
        (
            [cpu.regs.hl(), cpu.regs.de(), cpu.regs.bc()],
            bus.cycles,
            halt,
        )
    }

    /// Read a byte from the cell's memory *after a run* (the bus stays live until the
    /// next [`run`](Runner::run) resets it).
    pub fn peek_u8(&self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    /// Read a little-endian `u16` (one slot).
    pub fn peek_u16(&self, addr: u16) -> u16 {
        u16::from_le_bytes([
            self.mem[addr as usize],
            self.mem[addr.wrapping_add(1) as usize],
        ])
    }
    /// Read a `u32` (two slots: low word at `addr`, high word at `addr + 2`).
    pub fn peek_u32(&self, addr: u16) -> u32 {
        self.peek_u16(addr) as u32 | (self.peek_u16(addr.wrapping_add(2)) as u32) << 16
    }
    /// Decode named, typed values from post-run memory — the typed state read-back. The
    /// `(name, addr, ty)` layout is the caller's (e.g. from a state-struct symbol map);
    /// this turns it into `(name, value)` pairs read off the live bus.
    pub fn read_named(&self, fields: &[(String, u16, Ty)]) -> Vec<(String, u64)> {
        fields
            .iter()
            .map(|(name, addr, ty)| {
                let v = match ty {
                    Ty::U8 => self.peek_u8(*addr) as u64,
                    Ty::U16 => self.peek_u16(*addr) as u64,
                    Ty::U32 => self.peek_u32(*addr) as u64,
                };
                (name.clone(), v)
            })
            .collect()
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

/// One-shot convenience: compile `src` and run `entry` once (see [`Runner`] for
/// compile-once/run-many).
pub fn run(src: &str, entry: Option<&str>, args: &[u16], budget: u64) -> Result<Report, String> {
    Runner::compile(src)?.run(entry, args, budget)
}

/// Where a [`StateCell`]'s state struct is laid out — clear of code (`ORG`), the scratch
/// register file, the trampoline, and the stack.
pub const STATE_BASE: u16 = 0xB000;

/// A cell bound to a **state struct** at [`STATE_BASE`] — typed I/O by *field name*. The
/// agent/MCP surface for "named inputs in → run → named outputs out": compile once, then
/// `set` fields, `run`, `get` fields; the layout maps names to addresses. The program is a
/// method on the state (`impl State { fn run(&mut self) … }`), reached through `&mut self`.
///
/// ```
/// use rustz80::cell::{StateCell, DEFAULT_CYCLES};
/// let src = "struct State { x: u16, score: u16 }
///            impl State { fn run(&mut self) -> u16 { self.score = self.x * 2u16; self.score } }";
/// let mut cell = StateCell::bind(src, "State", None)?;
/// cell.set("x", 10)?;
/// cell.run(DEFAULT_CYCLES)?;
/// assert_eq!(cell.get("score"), Some(20));   // typed, by name — no raw addresses
/// # Ok::<(), String>(())
/// ```
pub struct StateCell {
    runner: Runner,
    addrs: HashMap<String, u16>, // scalar field name -> byte address
    entry: String,
    pending: Vec<(u16, Ty, u64)>,
}

impl StateCell {
    /// Compile `src`, bind its `state` struct's scalar fields at [`STATE_BASE`], and target
    /// `entry` (default `"<state>::run"`).
    pub fn bind(src: &str, state: &str, entry: Option<&str>) -> Result<Self, String> {
        let layout = crate::struct_layout(src, state)?;
        let mut addrs = HashMap::new();
        for f in &layout {
            if f.slots == 1 {
                // scalar field, addressable by name (a `u16`/`u8` slot)
                addrs.insert(f.name.clone(), STATE_BASE + f.offset * 2);
            }
        }
        Ok(StateCell {
            runner: Runner::compile(src)?,
            addrs,
            entry: entry.map_or_else(|| format!("{state}::run"), String::from),
            pending: Vec::new(),
        })
    }

    /// Queue a named `u16` input (written into the state before the next [`run`](StateCell::run)).
    pub fn set(&mut self, field: &str, value: u16) -> Result<(), String> {
        let &addr = self
            .addrs
            .get(field)
            .ok_or_else(|| format!("no scalar field `{field}`"))?;
        self.pending.push((addr, Ty::U16, value as u64));
        Ok(())
    }

    /// Run the entry with `&mut self` state at [`STATE_BASE`], applying then clearing the
    /// queued inputs.
    pub fn run(&mut self, budget: u64) -> Result<Report, String> {
        let pending = std::mem::take(&mut self.pending);
        self.runner
            .run_with_inputs(Some(&self.entry), &[STATE_BASE], &pending, budget)
    }

    /// Read a named `u16` field from the last run's state.
    pub fn get(&self, field: &str) -> Option<u16> {
        self.addrs.get(field).map(|&a| self.runner.peek_u16(a))
    }

    /// The bound (scalar) field names.
    pub fn fields(&self) -> impl Iterator<Item = &str> {
        self.addrs.keys().map(String::as_str)
    }
}

/// The lightweight outcome of a [`run_fast`](Runner::run_fast): the result registers,
/// T-states, and halt reason — no allocations (no symbol map, size report, or memory
/// diff). For tight agent loops.
#[derive(Debug, Clone, Copy)]
pub struct Fast {
    /// The primary result in `HL`.
    pub result: u16,
    /// All three result registers `[HL, DE, BC]`.
    pub regs: [u16; 3],
    pub cycles: u64,
    pub halt: Halt,
}

/// The structured outcome of a [`run`].
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
    /// [`Runner::read_named`] / the CLI `--read`).
    pub reads: Vec<(String, u64)>,
    /// T-states elapsed, and the budget it ran under.
    pub cycles: u64,
    pub budget: u64,
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
             cycles     {} / {} T-states\n\
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
            "{{\"entry\":\"{}\",\"entry_addr\":{},\"result\":{},\"regs\":[{},{},{}],\"cycles\":{},\
             \"budget\":{},\"halt\":\"{}\"{},\"code_bytes\":{},\"functions\":{},\"symbols\":{{{}}},\
             \"memory_touched\":[{}],\"reads\":{{{}}}}}",
            self.entry,
            self.entry_addr,
            self.result,
            self.regs[0],
            self.regs[1],
            self.regs[2],
            self.cycles,
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

/// Parse a `--set` spec — comma-separated `addr:ty=value` (addr/value decimal or `0x..`),
/// the typed inputs written into memory before the run.
fn parse_sets(s: &str) -> Result<Vec<(u16, Ty, u64)>, String> {
    let num16 = |t: &str| match t.strip_prefix("0x") {
        Some(h) => u16::from_str_radix(h, 16),
        None => t.parse::<u16>(),
    };
    let num64 = |t: &str| match t.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16),
        None => t.parse::<u64>(),
    };
    s.split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            let t = t.trim();
            let bad = || format!("bad --set `{t}` (want addr:ty=value)");
            let (lhs, val_s) = t.split_once('=').ok_or_else(bad)?;
            let (addr_s, ty_s) = lhs.split_once(':').ok_or_else(bad)?;
            let addr = num16(addr_s).map_err(|_| format!("bad address in `{t}`"))?;
            let val = num64(val_s).map_err(|_| format!("bad value in `{t}`"))?;
            Ok((addr, Ty::parse(ty_s)?, val))
        })
        .collect()
}

/// Parse a `--read` spec — comma-separated `name@addr:ty` (addr decimal or `0x..`).
fn parse_reads(s: &str) -> Result<Vec<(String, u16, Ty)>, String> {
    s.split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            let t = t.trim();
            let bad = || format!("bad --read `{t}` (want name@addr:ty)");
            let (name, rest) = t.split_once('@').ok_or_else(bad)?;
            let (addr_s, ty_s) = rest.split_once(':').ok_or_else(bad)?;
            let addr = match addr_s.strip_prefix("0x") {
                Some(h) => u16::from_str_radix(h, 16),
                None => addr_s.parse::<u16>(),
            }
            .map_err(|_| format!("bad address in `{t}`"))?;
            Ok((name.to_string(), addr, Ty::parse(ty_s)?))
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
    let mut sets: Vec<(u16, Ty, u64)> = Vec::new();
    let mut reads: Vec<(String, u16, Ty)> = Vec::new();
    let mut json = false;
    let mut cfg = CellConfig::sandboxed(); // safe by default on the CLI
    let num = |o: Option<&String>, what: &str| -> Result<usize, String> {
        o.ok_or_else(|| format!("{what} needs a number"))?
            .parse()
            .map_err(|_| format!("bad {what}"))
    };
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
            "--set" => sets = parse_sets(it.next().ok_or("--set needs a spec")?)?,
            "--read" => reads = parse_reads(it.next().ok_or("--read needs a spec")?)?,
            "--allow-raw-memory" => cfg.allow_raw_memory = true,
            "--allow-ports" => cfg.allow_ports = true,
            "--max-code-bytes" => cfg.max_code_bytes = Some(num(it.next(), "--max-code-bytes")?),
            "--max-touched" => cfg.max_touched = Some(num(it.next(), "--max-touched")?),
            "--json" => json = true,
            other => return Err(format!("unknown option `{other}`\n{USAGE}")),
        }
    }
    let src = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let mut runner = Runner::compile_with_config(&src, cfg)?;
    let mut report = runner.run_with_inputs(entry.as_deref(), &call_args, &sets, cycles)?;
    if !reads.is_empty() {
        report.reads = runner.read_named(&reads); // decode typed fields from post-run memory
    }
    Ok(if json {
        report.to_json()
    } else {
        report.to_human()
    })
}
