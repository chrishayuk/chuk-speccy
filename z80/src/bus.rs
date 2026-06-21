//! The CPU/memory boundary. The CPU owns no memory and no clock — it borrows a
//! `&mut impl Bus` for the duration of `step()`, and *all* timing lives in the
//! bus. See `docs/01-core-emulator-spec.md` §3 for the rationale.

pub trait Bus {
    // --- Data path. Each call is one Z80 memory/IO machine cycle. ---

    /// Read a byte from `addr`.
    fn read(&mut self, addr: u16) -> u8;
    /// Write `val` to `addr`.
    fn write(&mut self, addr: u16, val: u8);
    /// Read a byte from I/O `port` (full 16-bit port address).
    fn input(&mut self, port: u16) -> u8;
    /// Write `val` to I/O `port`.
    fn output(&mut self, port: u16, val: u8);

    // --- Timing path. The CPU reports *when* it touches the bus. ---

    /// Pre-access stall: the CPU is about to touch `addr`; the bus (ULA) injects
    /// any contention delay based on the frame position it already tracks.
    /// A non-contended bus implements this as a no-op.
    fn contend(&mut self, addr: u16, cycles: u32);

    /// Advance the master clock by `cycles` pure internal T-states (no bus
    /// access). This is the single source of truth for elapsed time.
    fn tick(&mut self, cycles: u32);

    /// The CPU decoded the reserved host-trap opcode (`ED FE`, [`crate::TRAP_OP`]).
    /// A handler reads the syscall id from `regs.a`, takes args from the registers
    /// / memory, and writes results back into `regs` (carry = error). Returns any
    /// extra T-states to charge for modelled latency. Default: do nothing (a NOP),
    /// so a bare bus treats `ED FE` as the no-op it is on real hardware.
    fn host_trap(&mut self, regs: &mut crate::Regs) -> u32 {
        let _ = regs;
        0
    }
}
