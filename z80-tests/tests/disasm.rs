//! Disassembler tests (`z80::disassemble`). Golden checks per opcode family +
//! a fuzz pass that every first byte and every prefixed opcode decodes without
//! panicking into a sane 1..=4 byte instruction.

use z80::disassemble;

/// Disassemble the instruction encoded by `bytes` (placed at 0x8000).
fn dis(bytes: &[u8]) -> (u8, String) {
    let d = disassemble(0x8000, |a| bytes[a.wrapping_sub(0x8000) as usize]);
    (d.len, d.text)
}

fn text(bytes: &[u8]) -> String {
    dis(bytes).1
}

#[test]
fn base_loads_and_alu() {
    assert_eq!(dis(&[0x00]), (1, "NOP".into()));
    assert_eq!(dis(&[0x76]), (1, "HALT".into()));
    assert_eq!(dis(&[0x01, 0x34, 0x12]), (3, "LD BC,$1234".into()));
    assert_eq!(text(&[0x3E, 0xFE]), "LD A,$FE");
    assert_eq!(text(&[0x78]), "LD A,B"); // LD A,B
    assert_eq!(text(&[0x70]), "LD (HL),B");
    assert_eq!(text(&[0x46]), "LD B,(HL)");
    assert_eq!(text(&[0x04]), "INC B");
    assert_eq!(text(&[0x35]), "DEC (HL)");
    assert_eq!(text(&[0x86]), "ADD A,(HL)");
    assert_eq!(text(&[0x90]), "SUB B");
    assert_eq!(text(&[0xBE]), "CP (HL)");
    assert_eq!(text(&[0xC6, 0x10]), "ADD A,$10");
    assert_eq!(text(&[0x09]), "ADD HL,BC");
    assert_eq!(text(&[0x32, 0x00, 0x5C]), "LD ($5C00),A");
    assert_eq!(text(&[0x2A, 0x00, 0x40]), "LD HL,($4000)");
}

#[test]
fn control_flow_targets_are_absolute() {
    // JR/DJNZ are relative to the next instruction (pc + 2 + d).
    assert_eq!(dis(&[0x20, 0x05]), (2, "JR NZ,$8007".into())); // 0x8000 + 2 + 5
    assert_eq!(dis(&[0x18, 0xFE]), (2, "JR $8000".into())); // back to self
    assert_eq!(dis(&[0x10, 0xFE]), (2, "DJNZ $8000".into()));
    assert_eq!(text(&[0xC3, 0x00, 0x80]), "JP $8000");
    assert_eq!(text(&[0xCA, 0x34, 0x12]), "JP Z,$1234");
    assert_eq!(text(&[0xCD, 0x34, 0x12]), "CALL $1234");
    assert_eq!(text(&[0xC4, 0x34, 0x12]), "CALL NZ,$1234");
    assert_eq!(text(&[0xC9]), "RET");
    assert_eq!(text(&[0xC0]), "RET NZ");
    assert_eq!(text(&[0xFF]), "RST $38");
    assert_eq!(text(&[0xE9]), "JP (HL)");
    assert_eq!(text(&[0xD3, 0xFE]), "OUT ($FE),A");
    assert_eq!(text(&[0xDB, 0xFE]), "IN A,($FE)");
}

#[test]
fn cb_prefix() {
    assert_eq!(dis(&[0xCB, 0x00]), (2, "RLC B".into()));
    assert_eq!(text(&[0xCB, 0x06]), "RLC (HL)");
    assert_eq!(text(&[0xCB, 0x7E]), "BIT 7,(HL)");
    assert_eq!(text(&[0xCB, 0x47]), "BIT 0,A");
    assert_eq!(text(&[0xCB, 0xC7]), "SET 0,A");
    assert_eq!(text(&[0xCB, 0x86]), "RES 0,(HL)");
    assert_eq!(text(&[0xCB, 0x30]), "SLL B"); // undocumented
}

#[test]
fn ed_prefix() {
    assert_eq!(dis(&[0xED, 0x52]), (2, "SBC HL,DE".into()));
    assert_eq!(text(&[0xED, 0x4A]), "ADC HL,BC");
    assert_eq!(text(&[0xED, 0x43, 0x00, 0x5C]), "LD ($5C00),BC");
    assert_eq!(text(&[0xED, 0x44]), "NEG");
    assert_eq!(text(&[0xED, 0x56]), "IM 1");
    assert_eq!(text(&[0xED, 0x4D]), "RETI");
    assert_eq!(text(&[0xED, 0x45]), "RETN");
    assert_eq!(text(&[0xED, 0x57]), "LD A,I");
    assert_eq!(text(&[0xED, 0xB0]), "LDIR");
    assert_eq!(text(&[0xED, 0xB8]), "LDDR");
    assert_eq!(text(&[0xED, 0xBB]), "OTDR");
    assert_eq!(text(&[0xED, 0xA1]), "CPI");
    // Undefined ED slot → DEFB.
    assert_eq!(dis(&[0xED, 0x00]), (2, "DEFB $ED,$00".into()));
}

#[test]
fn index_prefix_dd_fd() {
    assert_eq!(dis(&[0xDD, 0x21, 0x34, 0x12]), (4, "LD IX,$1234".into()));
    assert_eq!(dis(&[0xDD, 0x7E, 0x05]), (3, "LD A,(IX+5)".into()));
    assert_eq!(text(&[0xDD, 0x70, 0xFB]), "LD (IX-5),B");
    assert_eq!(text(&[0xFD, 0x7E, 0x05]), "LD A,(IY+5)");
    assert_eq!(text(&[0xDD, 0x86, 0x02]), "ADD A,(IX+2)");
    assert_eq!(text(&[0xDD, 0x34, 0x00]), "INC (IX+0)");
    assert_eq!(text(&[0xDD, 0xE5]), "PUSH IX");
    assert_eq!(text(&[0xDD, 0x09]), "ADD IX,BC");
    assert_eq!(text(&[0xDD, 0x23]), "INC IX");
    // Undocumented half-registers.
    assert_eq!(text(&[0xDD, 0x65]), "LD IXH,IXL");
    // LD (IX+d),n — displacement then immediate.
    assert_eq!(dis(&[0xDD, 0x36, 0x05, 0xFF]), (4, "LD (IX+5),$FF".into()));
}

#[test]
fn ddcb_fdcb() {
    assert_eq!(dis(&[0xDD, 0xCB, 0x05, 0x06]), (4, "RLC (IX+5)".into()));
    assert_eq!(text(&[0xDD, 0xCB, 0x05, 0x46]), "BIT 0,(IX+5)");
    assert_eq!(text(&[0xFD, 0xCB, 0xFB, 0xC6]), "SET 0,(IY-5)");
    // Undocumented register-copy variant (z != 6).
    assert_eq!(text(&[0xDD, 0xCB, 0x05, 0x00]), "RLC (IX+5),B");
    assert_eq!(text(&[0xDD, 0xCB, 0x05, 0xC1]), "SET 0,(IX+5),C");
}

#[test]
fn sequential_decode_walks_a_program() {
    // DI; LD HL,$4000; LD A,$02; OUT ($FE),A; HALT
    let prog = [0xF3, 0x21, 0x00, 0x40, 0x3E, 0x02, 0xD3, 0xFE, 0x76];
    let mut pc = 0u16;
    let mut out = Vec::new();
    while (pc as usize) < prog.len() {
        let d = disassemble(pc, |a| prog[a as usize]);
        out.push(d.text);
        pc += d.len as u16;
    }
    assert_eq!(
        out,
        ["DI", "LD HL,$4000", "LD A,$02", "OUT ($FE),A", "HALT"]
    );
    assert_eq!(pc as usize, prog.len(), "lengths tile the program exactly");
}

#[test]
fn length_matches_cpu_execution() {
    use z80::Cpu;
    use z80_tests::FlatBus;

    // For every non-branch base opcode, the bytes the CPU consumes (its PC delta)
    // must equal the disassembler's reported length. Operand bytes are 0x00, so
    // relative jumps land on the next instruction (PC delta still == length);
    // absolute jumps/calls/returns/RST move PC elsewhere and are pinned by the
    // golden tests instead.
    let is_branch = |op: u8| {
        matches!(
            op,
            0xC3 | 0xCD | 0xC9 | 0xE9                                  // JP nn, CALL nn, RET, JP (HL)
            | 0xC2 | 0xCA | 0xD2 | 0xDA | 0xE2 | 0xEA | 0xF2 | 0xFA    // JP cc,nn
            | 0xC4 | 0xCC | 0xD4 | 0xDC | 0xE4 | 0xEC | 0xF4 | 0xFC    // CALL cc,nn
            | 0xC0 | 0xC8 | 0xD0 | 0xD8 | 0xE0 | 0xE8 | 0xF0 | 0xF8    // RET cc
            | 0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF
        ) // RST
    };

    for op in 0u16..=255 {
        let op = op as u8;
        if is_branch(op) {
            continue;
        }
        let mut bus = FlatBus::new();
        bus.load(0x8000, &[op, 0x00, 0x00, 0x00]);
        let mut cpu = Cpu::new();
        cpu.regs.pc = 0x8000;
        cpu.step(&mut bus);
        let advanced = cpu.regs.pc.wrapping_sub(0x8000) as u8;
        let d = disassemble(0x8000, |a| bus.mem[a as usize]);
        assert_eq!(
            advanced, d.len,
            "op {op:02X} ({}): cpu +{advanced}, disasm len {}",
            d.text, d.len
        );
    }
}

#[test]
fn fuzz_every_opcode_decodes_sanely() {
    // Every first byte, and every opcode under each prefix, must produce a
    // 1..=4 byte instruction with non-empty text and never panic.
    let check = |bytes: &[u8; 6]| {
        let d = disassemble(0x8000, |a| bytes[a.wrapping_sub(0x8000) as usize]);
        assert!((1..=4).contains(&d.len), "len {} for {:02X?}", d.len, bytes);
        assert!(!d.text.is_empty(), "empty text for {bytes:02X?}");
    };
    for op in 0u16..=255 {
        check(&[op as u8, 0, 0, 0, 0, 0]);
        for &prefix in &[0xCBu8, 0xED, 0xDD, 0xFD] {
            check(&[prefix, op as u8, 0, 0, 0, 0]);
            // DDCB/FDCB: prefix, CB, displacement, opcode.
            check(&[prefix, 0xCB, 0x05, op as u8, 0, 0]);
        }
    }
}
