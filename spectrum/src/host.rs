//! Host-trap dispatch — the `ED FE` (`HOSTCALL`) ABI from
//! `docs/03-sdk-spec.md`. A Z80 program executes `ED FE` with a syscall id in
//! `A`; the CPU forwards it to [`z80::Bus::host_trap`], the [`Board`](crate::Board)
//! routes it to the installed [`HostCalls`] dispatcher, and the handler reads
//! args from the registers / memory and writes results back (carry = error).
//!
//! Two flavours of handler: [`FnTable`] (id → Rust closure, for math / asset DMA
//! / tests — never crosses a language boundary) and any custom [`HostCalls`] impl
//! (e.g. the PyO3 bridge that forwards to a Python callable).

use crate::memory::Memory;
use std::collections::HashMap;
use z80::Regs;

/// What a handler sees during a trap: the live register file plus scoped memory
/// access. Valid only for the duration of the synchronous dispatch call.
pub struct HostCtx<'a> {
    pub regs: &'a mut Regs,
    mem: &'a mut Memory,
}

impl<'a> HostCtx<'a> {
    pub(crate) fn new(regs: &'a mut Regs, mem: &'a mut Memory) -> Self {
        Self { regs, mem }
    }

    /// The syscall id (register `A`).
    #[inline]
    pub fn id(&self) -> u8 {
        self.regs.a
    }

    /// Read `len` bytes of the 64K space from `addr` (wrapping).
    pub fn read(&self, addr: u16, len: u16) -> Vec<u8> {
        (0..len).map(|i| self.mem.read(addr.wrapping_add(i))).collect()
    }

    /// Write `data` into memory at `addr` (ROM writes ignored, as on hardware).
    pub fn write(&mut self, addr: u16, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            self.mem.write(addr.wrapping_add(i as u16), b);
        }
    }

    /// Signal failure / success to the caller (carry flag).
    pub fn fail(&mut self) {
        self.regs.set_carry(true);
    }
    pub fn ok(&mut self) {
        self.regs.set_carry(false);
    }

    /// Raw pointers to the live register file and memory, for an FFI bridge that
    /// hands them to another language *during* the synchronous trap. The pointers
    /// are valid only for the duration of the dispatch call — the caller must not
    /// let them escape it (the PyO3 bridge enforces this with a liveness guard).
    #[doc(hidden)]
    pub fn raw_parts(&mut self) -> (*mut Regs, *mut Memory) {
        (&mut *self.regs as *mut Regs, &mut *self.mem as *mut Memory)
    }
}

/// Anything that answers host traps. `Send` so it can live behind threaded heads.
/// Return any extra T-states to charge for modelled latency.
pub trait HostCalls: Send {
    fn dispatch(&mut self, ctx: &mut HostCtx) -> u32;
}

/// A single native trap handler.
type Handler = Box<dyn FnMut(&mut HostCtx) -> u32 + Send>;

/// A registry of id → Rust closure — the native handler path (math, asset DMA,
/// tests). An unknown id fails cleanly (carry set, no-op).
#[derive(Default)]
pub struct FnTable {
    map: HashMap<u8, Handler>,
}

impl FnTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler for syscall `id`.
    pub fn on(&mut self, id: u8, f: impl FnMut(&mut HostCtx) -> u32 + Send + 'static) {
        self.map.insert(id, Box::new(f));
    }
}

impl HostCalls for FnTable {
    fn dispatch(&mut self, ctx: &mut HostCtx) -> u32 {
        match self.map.get_mut(&ctx.id()) {
            Some(h) => h(ctx),
            None => {
                ctx.fail(); // unknown id → CF=1, no-op
                0
            }
        }
    }
}
