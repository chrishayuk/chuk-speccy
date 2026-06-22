//! Pure Z80 CPU core. `no_std`, no dependencies — it never knows what a Spectrum
//! is. All memory and all timing live behind the [`Bus`] trait (see
//! `docs/01-core-emulator-spec.md` §3).
//!
//! Status: the full documented **and** undocumented instruction set is
//! implemented and **ZEXALL/ZEXDOC-clean** — including MEMPTR/WZ, XF/YF, the
//! SCF/CCF Q-quirk, IXH/IXL, and DDCB. Decode is table-driven via the X/Y/Z/P/Q
//! split (`decode.rs`); flags come from precomputed tables (`flags.rs`); a
//! read-only disassembler mirrors the decoder (`disasm.rs`).
#![no_std]

// The disassembler builds owned `String`s; `alloc` is provided by every consumer
// (all are `std`). The CPU and decoder themselves stay allocation-free.
extern crate alloc;

pub mod alu;
pub mod bus;
pub mod cpu;
pub mod decode;
pub mod disasm;
pub mod flags;

pub use bus::Bus;
pub use cpu::{Cpu, Index, Regs, StopReason};
pub use disasm::{disassemble, Disasm};

/// The reserved host-trap opcode: `ED FE` (`HOSTCALL`). Genuinely undefined on a
/// real Z80 (NONI+NOP) and on the ZX Spectrum Next's extended ED set, so a hybrid
/// binary degrades to "host did nothing" on bare hardware. The CPU forwards it to
/// [`Bus::host_trap`]; the Spectrum/host layer does the rest. (`ED 70`/`ED 71`
/// were avoided — they're the undocumented `IN (C)`/`OUT (C),0` ZEXALL exercises.)
pub const TRAP_OP: u8 = 0xFE;
