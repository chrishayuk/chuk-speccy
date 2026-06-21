//! Pure Z80 CPU core. `no_std`, no dependencies — it never knows what a Spectrum
//! is. All memory and all timing live behind the [`Bus`] trait (see
//! `docs/01-core-emulator-spec.md` §3).
//!
//! Scaffold status: the register file, flag tables, Bus boundary, and the
//! X/Y/Z/P/Q decode skeleton are in place. The opcode bodies are stubs marked
//! `TODO` — fill them in per milestones M1/M2. `dead_code` is allowed crate-wide
//! while the tables/arrays are still being wired up; remove it once the decoder
//! references them all.
#![no_std]
#![allow(dead_code)]

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
