//! Test harness crate for the `z80` core.
//!
//! The headline layer (per-opcode SingleStepTests JSON, then ZEXDOC/ZEXALL) is
//! TODO(M0/M2). For now this provides [`FlatBus`] — the ~20-line RAM-only test
//! double the JSON tests will run against — and a couple of smoke tests.

use z80::Bus;

/// A flat 64K RAM bus with no contention. `tick` accumulates a T-state count so
/// tests can assert cycle totals; `contend` is a no-op.
pub struct FlatBus {
    pub mem: [u8; 0x1_0000],
    pub tstates: u64,
}

impl FlatBus {
    pub fn new() -> Self {
        Self {
            mem: [0; 0x1_0000],
            tstates: 0,
        }
    }

    /// Load `bytes` at `addr`.
    pub fn load(&mut self, addr: u16, bytes: &[u8]) {
        let start = addr as usize;
        self.mem[start..start + bytes.len()].copy_from_slice(bytes);
    }
}

impl Default for FlatBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for FlatBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[addr as usize] = val;
    }
    fn input(&mut self, _port: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _port: u16, _val: u8) {}
    fn contend(&mut self, _addr: u16, _cycles: u32) {}
    fn tick(&mut self, cycles: u32) {
        self.tstates += cycles as u64;
    }
}

/// Run a CP/M-style ZEX test ROM (ZEXDOC / ZEXALL) and return everything it
/// printed. The ROM is loaded at 0x0100; BDOS calls to 0x0005 are trapped to
/// capture console output (function 2 = char in E, function 9 = `$`-terminated
/// string at DE), and a jump to 0x0000 (CP/M warm boot) ends the run.
///
/// This is the M2 acceptance harness: ZEXDOC passes without the XF/YF/Q minutiae,
/// ZEXALL requires all of it. It's slow (ZEXALL is billions of T-states) — run in
/// `--release`. Returns when the program exits or `max_instructions` is hit.
pub fn run_zex(rom: &[u8], max_instructions: u64) -> String {
    let mut out = String::new();
    run_zex_with(rom, max_instructions, |c| out.push(c));
    out
}

/// As [`run_zex`], but streams each console byte to `on_char` as it is produced,
/// so a caller can show progress during a long ZEXDOC/ZEXALL run.
pub fn run_zex_with(rom: &[u8], max_instructions: u64, mut on_char: impl FnMut(char)) {
    let mut bus = FlatBus::new();
    bus.load(0x0100, rom);
    let mut cpu = z80::Cpu::new();
    cpu.regs.pc = 0x0100;
    cpu.regs.sp = 0xF000;

    for _ in 0..max_instructions {
        match cpu.regs.pc {
            0x0000 => break, // warm boot: program finished
            0x0005 => {
                // BDOS call. Service it, then emulate RET.
                match cpu.regs.c {
                    2 => on_char(cpu.regs.e as char),
                    9 => {
                        let mut addr = cpu.regs.de();
                        while bus.mem[addr as usize] != b'$' {
                            on_char(bus.mem[addr as usize] as char);
                            addr = addr.wrapping_add(1);
                        }
                    }
                    _ => {}
                }
                let lo = bus.mem[cpu.regs.sp as usize] as u16;
                let hi = bus.mem[cpu.regs.sp.wrapping_add(1) as usize] as u16;
                cpu.regs.sp = cpu.regs.sp.wrapping_add(2);
                cpu.regs.pc = lo | (hi << 8);
            }
            _ => cpu.step(&mut bus),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use z80::Cpu;

    /// Validate the core against a ZEX ROM. Off by default (needs the binary):
    ///   ZEX_ROM=/path/to/zexdoc.com cargo test -p z80-tests --release -- \
    ///       --ignored --nocapture zex
    #[test]
    #[ignore = "set ZEX_ROM=/path/to/zexdoc.com (or zexall.com)"]
    fn zex() {
        let path = std::env::var("ZEX_ROM").expect("set ZEX_ROM to a .com test ROM");
        let rom = std::fs::read(&path).expect("read ZEX ROM");
        let out = run_zex(&rom, 50_000_000_000);
        print!("{out}");
        assert!(
            !out.to_ascii_uppercase().contains("ERROR"),
            "ZEX reported at least one CRC mismatch"
        );
    }

    /// Run a program (loaded at 0x0000) for `n` instructions from a clean CPU.
    fn run(prog: &[u8], n: usize) -> (Cpu, FlatBus) {
        let mut bus = FlatBus::new();
        bus.load(0x0000, prog);
        let mut cpu = Cpu::new();
        for _ in 0..n {
            cpu.step(&mut bus);
        }
        (cpu, bus)
    }

    // --- basics --------------------------------------------------------------

    #[test]
    fn nop_advances_pc_and_clock() {
        let (cpu, bus) = run(&[0x00], 1);
        assert_eq!(cpu.regs.pc, 0x0001);
        assert_eq!(bus.tstates, 4, "NOP is a single 4T M1 fetch");
        assert_eq!(cpu.regs.r & 0x7f, 1, "R bumped once");
    }

    #[test]
    fn halt_sets_halted_and_then_idles() {
        let mut bus = FlatBus::new();
        bus.load(0x0000, &[0x76]);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        assert!(cpu.halted);
        let t = bus.tstates;
        cpu.step(&mut bus);
        assert_eq!(bus.tstates, t + 4, "halted step burns 4T");
    }

    #[test]
    fn ld_immediate_and_reg_to_reg() {
        // LD A,0x42 ; LD B,A
        let (cpu, bus) = run(&[0x3E, 0x42, 0x47], 2);
        assert_eq!(cpu.regs.a, 0x42);
        assert_eq!(cpu.regs.b, 0x42);
        assert_eq!(bus.tstates, 7 + 4, "LD A,n=7T then LD B,A=4T");
    }

    #[test]
    fn ld_16bit_immediate() {
        // LD HL,0x1234
        let (cpu, _) = run(&[0x21, 0x34, 0x12], 1);
        assert_eq!(cpu.regs.hl(), 0x1234);
    }

    // --- 8-bit ALU + flags ---------------------------------------------------

    #[test]
    fn add_a_overflow_flags() {
        // LD A,0x7F ; ADD A,1
        let (cpu, _) = run(&[0x3E, 0x7F, 0xC6, 0x01], 2);
        assert_eq!(cpu.regs.a, 0x80);
        // S=1 H=1 PV=1 (overflow), Z=N=C=0, YF/XF=0
        assert_eq!(cpu.regs.f, 0x94, "F after 0x7F+1");
    }

    #[test]
    fn sub_underflow_flags() {
        // LD A,0 ; SUB 1
        let (cpu, _) = run(&[0x3E, 0x00, 0xD6, 0x01], 2);
        assert_eq!(cpu.regs.a, 0xFF);
        // S=1 YF=1 HF=1 XF=1 N=1 C=1 -> 0xBB
        assert_eq!(cpu.regs.f, 0xBB, "F after 0-1");
    }

    #[test]
    fn cp_uses_operand_undocumented_bits() {
        // LD A,0x00 ; CP 0x28  -> result 0xD8, but XF/YF come from operand 0x28
        let (cpu, _) = run(&[0x3E, 0x00, 0xFE, 0x28], 2);
        assert_eq!(cpu.regs.a, 0x00, "CP does not store");
        // operand 0x28 -> YF(0x20)|XF(0x08) set from operand, S from result(0xd8)=1
        assert_eq!(cpu.regs.f & (0x20 | 0x08), 0x28, "XF/YF from operand");
    }

    #[test]
    fn logic_ops_set_parity() {
        // LD A,0x0F ; AND 0x3C -> 0x0C (parity even -> PF set, HF set)
        let (cpu, _) = run(&[0x3E, 0x0F, 0xE6, 0x3C], 2);
        assert_eq!(cpu.regs.a, 0x0C);
        assert_eq!(cpu.regs.f & 0x04, 0x04, "PF set (even parity)");
        assert_eq!(cpu.regs.f & 0x10, 0x10, "AND sets HF");
    }

    #[test]
    fn inc_wraps_and_sets_flags() {
        // LD A,0xFF ; INC A -> 0x00, Z=1 H=1
        let (cpu, _) = run(&[0x3E, 0xFF, 0x3C], 2);
        assert_eq!(cpu.regs.a, 0x00);
        assert_eq!(cpu.regs.f & 0x40, 0x40, "ZF");
        assert_eq!(cpu.regs.f & 0x10, 0x10, "HF");
        assert_eq!(cpu.regs.f & 0x02, 0x00, "NF clear after INC");
    }

    // --- 16-bit --------------------------------------------------------------

    #[test]
    fn add_hl_rp() {
        // LD HL,0x1234 ; LD DE,0x1111 ; ADD HL,DE
        let (cpu, bus) = run(&[0x21, 0x34, 0x12, 0x11, 0x11, 0x11, 0x19], 3);
        assert_eq!(cpu.regs.hl(), 0x2345);
        assert_eq!(bus.tstates, 10 + 10 + 11, "ADD HL,rp is 11T");
    }

    #[test]
    fn inc_dec_16bit() {
        // LD BC,0xFFFF ; INC BC -> 0 ; DEC BC -> 0xFFFF
        let (cpu, _) = run(&[0x01, 0xFF, 0xFF, 0x03], 2);
        assert_eq!(cpu.regs.bc(), 0x0000);
    }

    // --- control flow --------------------------------------------------------

    #[test]
    fn jr_relative() {
        // JR +2 (skips the two bytes after the operand)
        let (cpu, _) = run(&[0x18, 0x02], 1);
        assert_eq!(cpu.regs.pc, 0x0004);
    }

    #[test]
    fn jp_conditional_taken() {
        // LD A,0 ; INC A->Z clear ; JP NZ,0x1000
        let (cpu, _) = run(&[0x3E, 0x00, 0x3C, 0xC2, 0x00, 0x10], 3);
        assert_eq!(cpu.regs.pc, 0x1000);
    }

    #[test]
    fn call_ret_roundtrip() {
        // SP=0xFFF0 ; CALL 0x0006 ; (at 0x0006) RET
        let mut bus = FlatBus::new();
        bus.load(0x0000, &[0x31, 0xF0, 0xFF, 0xCD, 0x06, 0x00]);
        bus.load(0x0006, &[0xC9]); // RET
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD SP
        cpu.step(&mut bus); // CALL
        assert_eq!(cpu.regs.pc, 0x0006);
        assert_eq!(cpu.regs.sp, 0xFFEE, "CALL pushed 2 bytes");
        cpu.step(&mut bus); // RET
        assert_eq!(cpu.regs.pc, 0x0006, "RET to instruction after CALL");
        assert_eq!(cpu.regs.sp, 0xFFF0, "stack restored");
    }

    #[test]
    fn push_pop_roundtrip() {
        // LD SP,0xFFF0 ; LD BC,0x1234 ; PUSH BC ; POP HL
        let (cpu, _) = run(
            &[0x31, 0xF0, 0xFF, 0x01, 0x34, 0x12, 0xC5, 0xE1],
            4,
        );
        assert_eq!(cpu.regs.hl(), 0x1234);
        assert_eq!(cpu.regs.sp, 0xFFF0);
    }

    // --- CB prefix -----------------------------------------------------------

    #[test]
    fn cb_rlc_carry() {
        // LD B,0x80 ; RLC B -> 0x01, CF=1
        let (cpu, _) = run(&[0x06, 0x80, 0xCB, 0x00], 2);
        assert_eq!(cpu.regs.b, 0x01);
        assert_eq!(cpu.regs.f & 0x01, 0x01, "carry out");
    }

    #[test]
    fn cb_bit_set_clear() {
        // LD A,0x80 ; BIT 7,A (set -> Z=0) ; BIT 0,A (clear -> Z=1)
        let (mut cpu, mut bus) = run(&[0x3E, 0x80, 0xCB, 0x7F], 2);
        assert_eq!(cpu.regs.f & 0x40, 0x00, "BIT 7 of 0x80: not zero");
        assert_eq!(cpu.regs.f & 0x80, 0x80, "BIT 7 set -> SF");
        bus.load(cpu.regs.pc, &[0xCB, 0x47]); // BIT 0,A
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.f & 0x40, 0x40, "BIT 0 of 0x80: zero -> ZF");
    }

    #[test]
    fn cb_res_set() {
        // LD A,0xFF ; RES 3,A -> 0xF7 ; SET... via two ops
        let (cpu, _) = run(&[0x3E, 0xFF, 0xCB, 0x9F], 2); // RES 3,A
        assert_eq!(cpu.regs.a, 0xF7);
    }

    // --- ED prefix -----------------------------------------------------------

    #[test]
    fn ed_neg() {
        // LD A,1 ; NEG -> 0xFF
        let (cpu, _) = run(&[0x3E, 0x01, 0xED, 0x44], 2);
        assert_eq!(cpu.regs.a, 0xFF);
        assert_eq!(cpu.regs.f, 0xBB, "NEG of 1 flags == 0-1");
    }

    #[test]
    fn ed_sbc_hl() {
        // LD HL,0x0000 ; (CF=0) SBC HL,HL -> 0
        let (cpu, _) = run(&[0x21, 0x00, 0x00, 0xED, 0x62], 2);
        assert_eq!(cpu.regs.hl(), 0x0000);
        assert_eq!(cpu.regs.f & 0x40, 0x40, "ZF set");
    }

    #[test]
    fn ed_ldir_block_copy() {
        // LD HL,src ; LD DE,dst ; LD BC,3 ; LDIR
        let mut bus = FlatBus::new();
        bus.load(0x8000, &[0xAA, 0xBB, 0xCC]); // source data
        bus.load(
            0x0000,
            &[
                0x21, 0x00, 0x80, // LD HL,0x8000
                0x11, 0x00, 0x90, // LD DE,0x9000
                0x01, 0x03, 0x00, // LD BC,3
                0xED, 0xB0, // LDIR
            ],
        );
        let mut cpu = Cpu::new();
        for _ in 0..4 {
            cpu.step(&mut bus); // LDIR runs to completion in one "instruction"...
        }
        // LDIR loops internally by rewinding PC; step the repeats out.
        for _ in 0..6 {
            cpu.step(&mut bus);
        }
        assert_eq!(bus.mem[0x9000], 0xAA);
        assert_eq!(bus.mem[0x9001], 0xBB);
        assert_eq!(bus.mem[0x9002], 0xCC);
        assert_eq!(cpu.regs.bc(), 0x0000);
        assert_eq!(cpu.regs.hl(), 0x8003);
        assert_eq!(cpu.regs.de(), 0x9003);
    }

    // --- DD/FD index prefix --------------------------------------------------

    #[test]
    fn dd_ld_a_ix_d() {
        // LD IX,0x8000 ; LD A,(IX+2) where mem[0x8002]=0x55
        let mut bus = FlatBus::new();
        bus.mem[0x8002] = 0x55;
        bus.load(0x0000, &[0xDD, 0x21, 0x00, 0x80, 0xDD, 0x7E, 0x02]);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD IX,nn
        assert_eq!(cpu.regs.ix, 0x8000);
        cpu.step(&mut bus); // LD A,(IX+2)
        assert_eq!(cpu.regs.a, 0x55);
    }

    #[test]
    fn dd_inc_ix_d() {
        // LD IX,0x8000 ; INC (IX+1) where mem[0x8001]=0x0F -> 0x10
        let mut bus = FlatBus::new();
        bus.mem[0x8001] = 0x0F;
        bus.load(0x0000, &[0xDD, 0x21, 0x00, 0x80, 0xDD, 0x34, 0x01]);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(bus.mem[0x8001], 0x10);
    }

    #[test]
    fn ddcb_set_ix_d() {
        // LD IX,0x8000 ; SET 0,(IX+0) where mem[0x8000]=0x00 -> 0x01
        let mut bus = FlatBus::new();
        bus.load(0x0000, &[0xDD, 0x21, 0x00, 0x80, 0xDD, 0xCB, 0x00, 0xC6]);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(bus.mem[0x8000], 0x01, "SET 0,(IX+0)");
    }

    // --- exchanges -----------------------------------------------------------

    #[test]
    fn ex_de_hl_and_exx() {
        // LD HL,0x1111 ; LD DE,0x2222 ; EX DE,HL
        let (cpu, _) = run(
            &[0x21, 0x11, 0x11, 0x11, 0x22, 0x22, 0xEB],
            3,
        );
        assert_eq!(cpu.regs.hl(), 0x2222);
        assert_eq!(cpu.regs.de(), 0x1111);
    }

    // --- M2: SCF/CCF Q-quirk -------------------------------------------------
    // These two programs differ only in whether a flag-modifying instruction
    // immediately precedes SCF. The undocumented YF/XF (bits 5,3) of F come out
    // differently — that difference *is* the Q-quirk. CP sets F's bits 5,3 from
    // its operand (0x28) without changing A, giving a strong distinguishing case.

    #[test]
    fn scf_q_quirk_prev_modified_flags() {
        // LD A,0 ; CP 0x28 ; SCF   (CP modifies F right before SCF)
        let (cpu, _) = run(&[0x3E, 0x00, 0xFE, 0x28, 0x37], 3);
        // q_prev == F, so YF/XF come from A only (=0): F = 0x81, bits 5,3 clear.
        assert_eq!(cpu.regs.f, 0x81, "SCF after a flag op: YF/XF from A");
        assert_eq!(cpu.regs.f & 0x28, 0x00, "bits 5,3 clear");
    }

    #[test]
    fn scf_q_quirk_prev_no_flags() {
        // LD A,0 ; CP 0x28 ; LD A,0 ; SCF   (LD does NOT modify F before SCF)
        let (cpu, _) = run(&[0x3E, 0x00, 0xFE, 0x28, 0x3E, 0x00, 0x37], 4);
        // q_prev == 0, so YF/XF come from (A | F): F's 0x28 survives -> 0xA9.
        assert_eq!(cpu.regs.f, 0xA9, "SCF after a non-flag op: YF/XF from A|F");
        assert_eq!(cpu.regs.f & 0x28, 0x28, "bits 5,3 set");
    }

    // --- M2: IXH/IXL undocumented half-registers -----------------------------

    #[test]
    fn ixh_ixl_access() {
        // LD IX,0x12FF ; LD A,IXH ; INC IXL ; LD B,IXL
        let (cpu, _) = run(
            &[
                0xDD, 0x21, 0xFF, 0x12, // LD IX,0x12FF
                0xDD, 0x7C, // LD A,IXH
                0xDD, 0x2C, // INC IXL  (0xFF -> 0x00)
                0xDD, 0x45, // LD B,IXL
            ],
            4,
        );
        assert_eq!(cpu.regs.a, 0x12, "A <- IXH");
        assert_eq!(cpu.regs.ix, 0x1200, "INC IXL wrapped low byte");
        assert_eq!(cpu.regs.b, 0x00, "B <- IXL");
    }

    #[test]
    fn ld_h_from_ix_d_uses_real_h() {
        // With a (IX+d) operand present, the partner register is the *real* H,
        // not IXH. LD IX,0x8000 ; LD H,(IX+0) where mem[0x8000]=0x77.
        let mut bus = FlatBus::new();
        bus.mem[0x8000] = 0x77;
        bus.load(0x0000, &[0xDD, 0x21, 0x00, 0x80, 0xDD, 0x66, 0x00]);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.h, 0x77, "real H loaded from (IX+0)");
        assert_eq!(cpu.regs.ix, 0x8000, "IX untouched (not used as IXH)");
    }

    #[test]
    fn ld_ix_absolute_roundtrip() {
        // Regression (ZEXDOC ld <ix,iy>,(nnnn) / ld (nnnn),<ix,iy>): the absolute
        // load/store forms must use IX/IY under DD, not HL.
        let mut bus = FlatBus::new();
        // LD IX,0x1234 ; LD (0x9000),IX ; LD IX,0 ; LD IX,(0x9000)
        bus.load(
            0x0000,
            &[
                0xDD, 0x21, 0x34, 0x12, // LD IX,0x1234
                0xDD, 0x22, 0x00, 0x90, // LD (0x9000),IX
                0xDD, 0x21, 0x00, 0x00, // LD IX,0
                0xDD, 0x2A, 0x00, 0x90, // LD IX,(0x9000)
            ],
        );
        let mut cpu = Cpu::new();
        for _ in 0..4 {
            cpu.step(&mut bus);
        }
        assert_eq!(bus.mem[0x9000], 0x34, "low byte stored");
        assert_eq!(bus.mem[0x9001], 0x12, "high byte stored");
        assert_eq!(cpu.regs.ix, 0x1234, "IX reloaded from (nnnn)");
    }

    // --- M2: DDCB undocumented register copy ---------------------------------

    #[test]
    fn ddcb_rlc_copies_to_register() {
        // RLC (IX+0) -> B : DD CB 00 00. mem[0x8000]=0x80 -> 0x01, and B=0x01.
        let mut bus = FlatBus::new();
        bus.mem[0x8000] = 0x80;
        bus.load(0x0000, &[0xDD, 0x21, 0x00, 0x80, 0xDD, 0xCB, 0x00, 0x00]);
        let mut cpu = Cpu::new();
        cpu.step(&mut bus); // LD IX
        cpu.step(&mut bus); // RLC (IX+0),B
        assert_eq!(bus.mem[0x8000], 0x01, "memory rotated");
        assert_eq!(cpu.regs.b, 0x01, "result also copied to B (undocumented)");
        assert_eq!(cpu.regs.f & 0x01, 0x01, "carry out");
    }

    // --- integration: a real little program ---------------------------------

    #[test]
    fn sum_1_to_10_with_djnz() {
        // LD B,10 ; LD A,0 ; loop: ADD A,B ; DJNZ loop ; HALT
        let mut bus = FlatBus::new();
        bus.load(0x0000, &[0x06, 0x0A, 0x3E, 0x00, 0x80, 0x10, 0xFD, 0x76]);
        let mut cpu = Cpu::new();
        // Run to HALT (with a sane instruction cap so a bug can't hang the test).
        for _ in 0..1000 {
            cpu.step(&mut bus);
            if cpu.halted {
                break;
            }
        }
        assert!(cpu.halted, "program reached HALT");
        assert_eq!(cpu.regs.a, 55, "10+9+...+1 == 55");
        assert_eq!(cpu.regs.b, 0, "loop counter exhausted");
    }
}
