//! Naive Z80 codegen (Stage 1). `HL` is the working accumulator, `DE` secondary;
//! locals (incl. parameters) live in a fixed RAM scratch region (the "virtual
//! register file") and expressions evaluate via the stack. Functions follow the
//! spec-07 calling convention; `*`/`/`/`%` call an appended micro-runtime.
//! Correct first — peephole/strength-reduce come in Stage 2.

use crate::ir::*;
use std::collections::HashMap;

mod asm;
mod expr;
mod runtime;
mod stmt;

use asm::{slot_addr, Asm};
use stmt::{gen_return, gen_stmt};

/// Code-generation target. `Spectrum48` is authentic Z80 — `*`/`/`/`%` use the appended
/// software micro-runtime, so the output runs anywhere (real ROM, `.tap`). `Cell` is the
/// micro-VM ([`crate::cell`]): those ops lower to the `ED FE` host-trap (serviced natively
/// by the cell bus — see the Cell80 plan), so no software runtime is appended. `ED FE` is
/// a no-op on real hardware, so it never reaches a real game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Spectrum48,
    Cell,
}

/// Compile a whole program (functions laid out in order, micro-runtime appended).
///
/// If `entry` is set, a tiny `DI; CALL entry; EI; RET` trampoline is emitted **at
/// `org`** so callers can `USR org`. The `DI` matters: the compiler keeps live
/// values in `DE`/`BC` across instructions, but the Spectrum's interrupt routine
/// clobbers `BC`/`DE` (its keyboard scan), so an interrupt mid-computation would
/// corrupt arithmetic. Disabling interrupts for the run avoids that; `EI` restores
/// them before returning to BASIC.
pub fn codegen_program(
    funcs: &[(String, Func)],
    org: u16,
    entry: Option<&str>,
    target: Target,
) -> (Vec<u8>, HashMap<String, u16>) {
    let mut a = Asm::new(org, target);
    if let Some(e) = entry {
        a.byte(0xF3); // DI
        a.call(e); // CALL entry
        a.byte(0xFB); // EI
        a.byte(0xC9); // RET
    }
    let mut base = 0u16;
    for (name, func) in funcs {
        a.define(name);
        a.base = base;
        emit_func(&mut a, func);
        base += func.n_locals as u16;
    }
    a.finish()
}

/// A generic **frame-synced entry loop** at `org`: zero a `state_bytes` region at
/// `state_base`, then each interrupt do `EI; HALT; DI; CALL entry(state_base, 0, 0);
/// JP loop` — interrupts on only for the `HALT` frame-sync, off during `entry` (so
/// its arithmetic isn't corrupted by the ROM's keyboard scan). The compiler knows
/// nothing about "games": `entry`, `state_base`, and `state_bytes` are the caller's.
pub fn codegen_loop(
    funcs: &[(String, Func)],
    org: u16,
    entry: &str,
    state_base: u16,
    state_bytes: u16,
) -> Vec<u8> {
    // Games are authentic Z80 (real ROM); always the Spectrum target.
    let mut a = Asm::new(org, Target::Spectrum48);
    a.byte(0xF3); // DI
                  // Zero the state region (memset via LD (HL),0 + LDIR).
    if state_bytes >= 2 {
        a.byte(0x21);
        a.word(state_base); // LD HL, STATE
        a.byte(0x36);
        a.byte(0x00); // LD (HL), 0
        a.byte(0x11);
        a.word(state_base + 1); // LD DE, STATE+1
        a.byte(0x01);
        a.word(state_bytes - 1); // LD BC, n-1
        a.byte(0xED);
        a.byte(0xB0); // LDIR
    } else if state_bytes == 1 {
        a.byte(0x21);
        a.word(state_base);
        a.byte(0x36);
        a.byte(0x00);
    }
    let loop_l = a.label();
    a.place(loop_l);
    a.byte(0xFB); // EI
    a.byte(0x76); // HALT     (wait for the 50 Hz frame interrupt)
    a.byte(0xF3); // DI
    a.byte(0x21);
    a.word(state_base); // LD HL, &state   (first arg)
    a.byte(0x11);
    a.word(0); // LD DE, 0   (second arg, unused)
    a.byte(0x01);
    a.word(0); // LD BC, 0   (third arg, unused)
    a.call(entry); // CALL entry
    a.jump(0xC3, loop_l); // JP loop

    let mut base = 0u16;
    for (name, func) in funcs {
        a.define(name);
        a.base = base;
        emit_func(&mut a, func);
        base += func.n_locals as u16;
    }
    a.finish().0
}

fn emit_func(a: &mut Asm, f: &Func) {
    // Prologue: copy parameters from the convention registers into their slots.
    for i in 0..f.params {
        let addr = slot_addr(a.base, i);
        match i {
            0 => {
                a.byte(0x22); // LD (addr), HL
                a.word(addr);
            }
            1 => {
                a.byte(0xED); // LD (addr), DE
                a.byte(0x53);
                a.word(addr);
            }
            2 => {
                a.byte(0xED); // LD (addr), BC
                a.byte(0x43);
                a.word(addr);
            }
            _ => unreachable!(),
        }
    }
    // The epilogue label — `return` jumps here. The body and tail fall through to
    // it; an early `return` skips the tail (its value is already in `HL`).
    let end = a.label();
    a.func_end = Some(end);
    for s in &f.body {
        gen_stmt(a, s);
    }
    gen_return(a, &f.ret);
    a.place(end);
    a.func_end = None;
    a.byte(0xC9); // RET
}
