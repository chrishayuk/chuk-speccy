//! Shared example harness: run `rustz80`-compiled code on the real `z80` CPU.
//!
//! This file lives in `examples/common/` (no `main`), so cargo does **not** treat it
//! as an example target; each example pulls it in with
//! `#[path = "common/cpu.rs"] mod cpu;`. It is the same flat-RAM bus + run loop the
//! differential tests use — factored out so every example reads top to bottom.

#![allow(dead_code)] // not every example uses every helper

/// A flat 64 KiB RAM bus — enough to run a compiled function with no ROM.
pub struct Ram {
    pub mem: Vec<u8>,
}

impl z80::Bus for Ram {
    fn read(&mut self, a: u16) -> u8 {
        self.mem[a as usize]
    }
    fn write(&mut self, a: u16, v: u8) {
        self.mem[a as usize] = v;
    }
    fn input(&mut self, _: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _: u16, _: u8) {}
    fn contend(&mut self, _: u16, _: u32) {}
    fn tick(&mut self, _: u32) {}
}

/// Compile `src`, call `entry` with `args` in the calling-convention registers
/// (`HL`/`DE`/`BC`, up to three), run to the trampoline's `HALT`, and hand back the
/// halted CPU + final memory. The typed wrappers below read what they need off it.
fn exec(src: &str, entry: &str, args: &[u16]) -> (z80::Cpu, Vec<u8>) {
    let prog = rustz80::compile_program(src).unwrap_or_else(|e| panic!("compile failed: {e}"));
    let target = *prog
        .symbols
        .get(entry)
        .unwrap_or_else(|| panic!("no `{entry}` symbol"));

    let mut bus = Ram {
        mem: vec![0u8; 0x1_0000],
    };

    // Trampoline at 0x7000: load the args, CALL the entry, HALT on return.
    let mut tramp = Vec::new();
    const LD: [u8; 3] = [0x21, 0x11, 0x01]; // LD HL,nn / LD DE,nn / LD BC,nn
    for (i, &v) in args.iter().enumerate().take(3) {
        tramp.push(LD[i]);
        tramp.push(v as u8);
        tramp.push((v >> 8) as u8);
    }
    tramp.push(0xCD); // CALL target
    tramp.push(target as u8);
    tramp.push((target >> 8) as u8);
    tramp.push(0x76); // HALT
    bus.mem[0x7000..0x7000 + tramp.len()].copy_from_slice(&tramp);

    let org = rustz80::ORG as usize;
    bus.mem[org..org + prog.code.len()].copy_from_slice(&prog.code);

    let mut cpu = z80::Cpu::new();
    cpu.reset();
    cpu.regs.pc = 0x7000;
    cpu.regs.sp = 0xFFF0;
    for _ in 0..100_000_000 {
        if cpu.halted {
            break;
        }
        cpu.step(&mut bus);
    }
    assert!(
        cpu.halted,
        "`{entry}` did not return within the step budget"
    );
    (cpu, bus.mem)
}

/// Compile `src`, call `entry` with `args` (in `HL`/`DE`/`BC`), run to completion, and
/// return the result in `HL` plus the final 64 KiB memory (for `poke`-based programs).
pub fn run(src: &str, entry: &str, args: &[u16]) -> (u16, Vec<u8>) {
    let (cpu, mem) = exec(src, entry, args);
    (cpu.regs.hl(), mem)
}

/// `run` for a value-returning entry — just the `HL` result.
pub fn run_value(src: &str, entry: &str, args: &[u16]) -> u16 {
    exec(src, entry, args).0.regs.hl()
}

/// Run a tuple-returning entry and read back the first three result registers
/// `[HL, DE, BC]` — i.e. several values returned at once.
pub fn run_regs(src: &str, entry: &str, args: &[u16]) -> [u16; 3] {
    let (cpu, _) = exec(src, entry, args);
    [cpu.regs.hl(), cpu.regs.de(), cpu.regs.bc()]
}

/// Render the Spectrum bitmap (`0x4000..0x5800`) as ASCII art over a `cols×rows`
/// character grid — each cell sampled at its top-left pixel. Handy for *seeing* what
/// a `poke`-based example drew when you `cargo run` it.
pub fn screen_art(mem: &[u8], cols: u16, rows: u16) -> String {
    let mut out = String::new();
    for cy in 0..rows {
        for cx in 0..cols {
            let (x, y) = (cx * 8, cy * 8);
            let addr =
                0x4000 + ((y & 0xC0) << 5) + ((y & 0x07) << 8) + ((y & 0x38) << 2) + (x >> 3);
            let lit = mem[addr as usize] & (0x80 >> (x & 7)) != 0;
            out.push(if lit { '#' } else { '.' });
        }
        out.push('\n');
    }
    out
}
