//! The runnable machine — `Runner`, its `CellBus`, the exec core, and `CellPool`.
use super::report::{coalesce, sorted_symbols};
use super::*;
use crate::{Program, ORG};

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
    /// candidates" path. The entry is resolved once (no per-call name allocation/lookup).
    ///
    /// If the entry is **straight-line** over the opcode subset the compiler emits (no
    /// branches/calls/`halt`), it's decoded once and replayed by a stripped fast executor
    /// (no per-instruction fetch/contention/refresh/flag work) — several × faster. The
    /// cycle count is then input-independent, so it's taken from one authentic calibration
    /// run; results are still the real Z80 semantics (oracle-validated). Anything outside
    /// that subset transparently falls back to the authentic interpreter, per input.
    /// One [`Fast`] per input set, in order.
    pub fn run_many_fast(
        &mut self,
        entry: Option<&str>,
        arg_sets: &[&[u16]],
        budget: u64,
    ) -> Result<Vec<Fast>, String> {
        let entry_addr = self.resolve_addr(entry)?;
        if let Some(ops) = fast::decode(&self.prog.code, entry_addr) {
            // Calibrate the (input-independent) cycle count + confirm a clean return under
            // budget. If the cell doesn't return cleanly (shouldn't for straight-line), or
            // there are no inputs, fall through to the authentic path.
            if let Some(first) = arg_sets.first() {
                let (_, cycles, halt) = self.exec(entry_addr, first, &[], budget);
                if halt == Halt::Returned {
                    return Ok(arg_sets
                        .iter()
                        .map(|args| {
                            let regs = fast::run(
                                &ops,
                                &mut self.mem,
                                &mut self.seen,
                                &mut self.touched,
                                args,
                            );
                            Fast {
                                result: regs[0],
                                regs,
                                cycles,
                                halt: Halt::Returned,
                            }
                        })
                        .collect());
                }
            }
        }
        // Fallback: the authentic interpreter, per input.
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

    /// Re-point this runner at `program`, **reusing the allocated 64 KiB bus** (for
    /// [`CellPool`]). Clears the previous run's writes and the previous program's code so
    /// there's no cross-program leakage, then loads the new code — paying only O(code), not
    /// a fresh 128 KiB alloc/zero.
    fn reset_for(&mut self, program: &CellProgram) {
        for &a in &self.touched {
            self.mem[a as usize] = 0;
            self.seen[a as usize] = false;
        }
        self.touched.clear();
        let org = ORG as usize;
        for b in self.mem[org..org + self.prog.code.len()].iter_mut() {
            *b = 0; // the old program's code (the new one may be shorter)
        }
        self.prog = program.prog.clone();
        self.cfg = program.cfg.clone();
        self.mem[org..org + self.prog.code.len()].copy_from_slice(&self.prog.code);
    }
}

/// A pool of reusable 64 KiB buses. Acquiring a cell for *any* program recycles an idle bus
/// instead of allocating + zeroing a fresh 128 KiB (the ~1 µs `Runner::new` cost the
/// lifecycle bench isolates) — paying only to load the code. For "spawn many short-lived
/// cells" / "instantiate N candidate tools concurrently" patterns: [`acquire`](Self::acquire)
/// a runner, run it, [`release`](Self::release) it back. The pool grows to the high-water
/// mark of live cells.
#[derive(Default)]
pub struct CellPool {
    idle: Vec<Runner>,
}

impl CellPool {
    /// An empty pool (allocates buses lazily, on the first [`acquire`](Self::acquire)).
    pub fn new() -> Self {
        Self::default()
    }

    /// A runner loaded with `program` — recycling an idle bus if one is free (no 128 KiB
    /// alloc), else allocating one. Return it with [`release`](Self::release).
    pub fn acquire(&mut self, program: &CellProgram) -> Runner {
        match self.idle.pop() {
            Some(mut r) => {
                r.reset_for(program);
                r
            }
            None => Runner::new(program),
        }
    }

    /// Return a runner to the pool so its bus can be reused by a later acquire.
    pub fn release(&mut self, runner: Runner) {
        self.idle.push(runner);
    }

    /// How many buses are idle (reusable without allocation).
    pub fn idle_count(&self) -> usize {
        self.idle.len()
    }
}

/// One-shot convenience: compile `src` and run `entry` once (see [`Runner`] for
/// compile-once/run-many).
pub fn run(src: &str, entry: Option<&str>, args: &[u16], budget: u64) -> Result<Report, String> {
    Runner::compile(src)?.run(entry, args, budget)
}
