//! A pure Z80 disassembler — the read-only mirror of [`crate::decode`].
//!
//! It walks the same X/Y/Z/P/Q decomposition the executor does (so prefixes,
//! `(IX+d)` substitution, DDCB layout, ED block ops and the undocumented slots
//! all line up), but emits text instead of running anything. One instruction per
//! call: give it a `pc` and a byte-reader, get back the mnemonic and how many
//! bytes it spans — so `pc + len` is the next instruction.
//!
//! Numbers are hex (`$1234` / `$3F`); JR/DJNZ targets are resolved to absolute
//! addresses; undefined `ED` opcodes disassemble to `DEFB $ED,$xx`.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};

use crate::cpu::Index;

/// One disassembled instruction.
#[derive(Debug, Clone)]
pub struct Disasm {
    /// Bytes the instruction spans (1..=4).
    pub len: u8,
    /// The mnemonic, e.g. `"LD A,(IX+5)"`.
    pub text: String,
}

/// Disassemble one instruction at `pc`. `read(addr)` returns the byte at `addr`
/// (the 64K space wraps); only `len` bytes from `pc` are read.
pub fn disassemble<F: Fn(u16) -> u8>(pc: u16, read: F) -> Disasm {
    let mut d = Dis { pc, read, c: 0 };
    let op = d.next();
    let text = d.base(op, Index::Hl);
    Disasm { len: d.c, text }
}

// 8-bit registers by `r[]` index; index 6 is the `(HL)`/`(IX+d)` memory slot.
const R: [&str; 8] = ["B", "C", "D", "E", "H", "L", "(HL)", "A"];
// 16-bit pairs: `rp[]` (index 3 = SP) and `rp2[]` (index 3 = AF).
const RP: [&str; 4] = ["BC", "DE", "HL", "SP"];
// Condition codes `cc[]`.
const CC: [&str; 8] = ["NZ", "Z", "NC", "C", "PO", "PE", "P", "M"];
// CB rotate/shift ops (index 6 = the undocumented SLL).
const ROT: [&str; 8] = ["RLC", "RRC", "RL", "RR", "SLA", "SRA", "SLL", "SRL"];
// ALU ops, including the conventional `A,` for the ones that take it.
const ALU: [&str; 8] = ["ADD A,", "ADC A,", "SUB ", "SBC A,", "AND ", "XOR ", "OR ", "CP "];
// ED block ops, indexed `[y-4][z]` (rows: I/D/IR/DR; cols: LD/CP/IN/OUT).
const BLOCK: [[&str; 4]; 4] = [
    ["LDI", "CPI", "INI", "OUTI"],
    ["LDD", "CPD", "IND", "OUTD"],
    ["LDIR", "CPIR", "INIR", "OTIR"],
    ["LDDR", "CPDR", "INDR", "OTDR"],
];

struct Dis<F> {
    pc: u16,
    read: F,
    /// Bytes consumed so far (the running instruction length).
    c: u8,
}

impl<F: Fn(u16) -> u8> Dis<F> {
    fn next(&mut self) -> u8 {
        let b = (self.read)(self.pc.wrapping_add(self.c as u16));
        self.c += 1;
        b
    }

    fn imm16(&mut self) -> u16 {
        let lo = self.next() as u16;
        let hi = self.next() as u16;
        lo | (hi << 8)
    }

    /// JR/DJNZ target: the signed displacement is relative to the *next*
    /// instruction, i.e. `pc + len + d`. At this point `c` is the full length.
    fn rel(&mut self) -> String {
        let d = self.next() as i8 as i16;
        word(self.pc.wrapping_add(self.c as u16).wrapping_add(d as u16))
    }

    /// A `(HL)` / `(IX+d)` / `(IY+d)` memory operand (reads `d` for the indexed forms).
    fn mem(&mut self, im: Index) -> String {
        match im {
            Index::Hl => "(HL)".to_string(),
            _ => {
                let d = self.next() as i8;
                format!("({}{})", idx_name(im), disp(d))
            }
        }
    }

    /// 8-bit register name with IX/IY half-register substitution (indices 4/5).
    /// Not used for index 6 (the memory slot is handled by `mem`).
    fn r8(&self, idx: u8, im: Index) -> &'static str {
        match (im, idx) {
            (Index::Ix, 4) => "IXH",
            (Index::Ix, 5) => "IXL",
            (Index::Iy, 4) => "IYH",
            (Index::Iy, 5) => "IYL",
            _ => R[idx as usize],
        }
    }

    /// An 8-bit operand: memory for index 6, else a register.
    fn operand8(&mut self, z: u8, im: Index) -> String {
        if z == 6 {
            self.mem(im)
        } else {
            self.r8(z, im).to_string()
        }
    }

    // --- the base table (x/y/z/p/q), with HL → IX/IY substitution ------------

    fn base(&mut self, op: u8, im: Index) -> String {
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;
        match x {
            0 => self.x0(y, z, p, q, im),
            1 => {
                if y == 6 && z == 6 {
                    "HALT".to_string()
                } else {
                    self.ld_r_r(y, z, im)
                }
            }
            2 => {
                let operand = self.operand8(z, im);
                format!("{}{}", ALU[y as usize], operand)
            }
            3 => self.x3(y, z, p, q, im),
            _ => unreachable!(),
        }
    }

    /// LD r,r' (x==1, HALT removed). With a memory operand the *other* register
    /// is a real H/L; with two registers, H/L substitute to IXH/IXL under DD/FD.
    fn ld_r_r(&mut self, y: u8, z: u8, im: Index) -> String {
        if z == 6 {
            let src = self.mem(im);
            format!("LD {},{}", R[y as usize], src)
        } else if y == 6 {
            let dst = self.mem(im);
            format!("LD {},{}", dst, R[z as usize])
        } else {
            format!("LD {},{}", self.r8(y, im, ), self.r8(z, im))
        }
    }

    fn x0(&mut self, y: u8, z: u8, p: u8, q: u8, im: Index) -> String {
        match z {
            0 => match y {
                0 => "NOP".to_string(),
                1 => "EX AF,AF'".to_string(),
                2 => format!("DJNZ {}", self.rel()),
                3 => format!("JR {}", self.rel()),
                _ => format!("JR {},{}", CC[(y - 4) as usize], self.rel()),
            },
            1 => {
                if q == 0 {
                    let nn = self.imm16();
                    format!("LD {},{}", rp_name(p, im), word(nn))
                } else {
                    format!("ADD {},{}", idx_name(im), rp_name(p, im))
                }
            }
            2 => self.x0_z2(p, q, im),
            3 => {
                let opn = if q == 0 { "INC" } else { "DEC" };
                format!("{} {}", opn, rp_name(p, im))
            }
            4 => {
                let t = self.inc_dec_target(y, im);
                format!("INC {t}")
            }
            5 => {
                let t = self.inc_dec_target(y, im);
                format!("DEC {t}")
            }
            6 => {
                if y == 6 {
                    let m = self.mem(im);
                    let n = self.next();
                    format!("LD {},{}", m, byte(n))
                } else {
                    let n = self.next();
                    format!("LD {},{}", self.r8(y, im), byte(n))
                }
            }
            7 => ["RLCA", "RRCA", "RLA", "RRA", "DAA", "CPL", "SCF", "CCF"][y as usize].to_string(),
            _ => unreachable!(),
        }
    }

    fn inc_dec_target(&mut self, y: u8, im: Index) -> String {
        if y == 6 {
            self.mem(im)
        } else {
            self.r8(y, im).to_string()
        }
    }

    fn x0_z2(&mut self, p: u8, q: u8, im: Index) -> String {
        match (q, p) {
            (0, 0) => "LD (BC),A".to_string(),
            (0, 1) => "LD (DE),A".to_string(),
            (0, 2) => {
                let nn = self.imm16();
                format!("LD ({}),{}", word(nn), idx_name(im))
            }
            (0, 3) => {
                let nn = self.imm16();
                format!("LD ({}),A", word(nn))
            }
            (1, 0) => "LD A,(BC)".to_string(),
            (1, 1) => "LD A,(DE)".to_string(),
            (1, 2) => {
                let nn = self.imm16();
                format!("LD {},({})", idx_name(im), word(nn))
            }
            (1, 3) => {
                let nn = self.imm16();
                format!("LD A,({})", word(nn))
            }
            _ => unreachable!(),
        }
    }

    fn x3(&mut self, y: u8, z: u8, p: u8, q: u8, im: Index) -> String {
        match z {
            0 => format!("RET {}", CC[y as usize]),
            1 => {
                if q == 0 {
                    format!("POP {}", rp2_name(p, im))
                } else {
                    match p {
                        0 => "RET".to_string(),
                        1 => "EXX".to_string(),
                        2 => format!("JP ({})", idx_name(im)),
                        3 => format!("LD SP,{}", idx_name(im)),
                        _ => unreachable!(),
                    }
                }
            }
            2 => {
                let nn = self.imm16();
                format!("JP {},{}", CC[y as usize], word(nn))
            }
            3 => match y {
                0 => {
                    let nn = self.imm16();
                    format!("JP {}", word(nn))
                }
                1 => self.cb(im),
                2 => {
                    let n = self.next();
                    format!("OUT ({}),A", byte(n))
                }
                3 => {
                    let n = self.next();
                    format!("IN A,({})", byte(n))
                }
                4 => format!("EX (SP),{}", idx_name(im)),
                5 => "EX DE,HL".to_string(),
                6 => "DI".to_string(),
                7 => "EI".to_string(),
                _ => unreachable!(),
            },
            4 => {
                let nn = self.imm16();
                format!("CALL {},{}", CC[y as usize], word(nn))
            }
            5 => {
                if q == 0 {
                    format!("PUSH {}", rp2_name(p, im))
                } else {
                    match p {
                        0 => {
                            let nn = self.imm16();
                            format!("CALL {}", word(nn))
                        }
                        1 => {
                            let op = self.next();
                            self.base(op, Index::Ix) // DD prefix
                        }
                        2 => self.ed(), // ED prefix
                        3 => {
                            let op = self.next();
                            self.base(op, Index::Iy) // FD prefix
                        }
                        _ => unreachable!(),
                    }
                }
            }
            6 => {
                let n = self.next();
                format!("{}{}", ALU[y as usize], byte(n))
            }
            7 => format!("RST {}", byte(y * 8)),
            _ => unreachable!(),
        }
    }

    fn cb(&mut self, im: Index) -> String {
        if let Index::Hl = im {
            let op = self.next();
            let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
            match x {
                0 => format!("{} {}", ROT[y as usize], R[z as usize]),
                1 => format!("BIT {},{}", y, R[z as usize]),
                2 => format!("RES {},{}", y, R[z as usize]),
                3 => format!("SET {},{}", y, R[z as usize]),
                _ => unreachable!(),
            }
        } else {
            // DDCB/FDCB: the displacement precedes the opcode; the op targets
            // (IX+d). For z != 6 the result is also copied into r[z] (undocumented).
            let d = self.next() as i8;
            let op = self.next();
            let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
            let m = format!("({}{})", idx_name(im), disp(d));
            match (x, z) {
                (1, _) => format!("BIT {},{}", y, m), // BIT ignores z
                (0, 6) => format!("{} {}", ROT[y as usize], m),
                (0, _) => format!("{} {},{}", ROT[y as usize], m, R[z as usize]),
                (2, 6) => format!("RES {},{}", y, m),
                (2, _) => format!("RES {},{},{}", y, m, R[z as usize]),
                (3, 6) => format!("SET {},{}", y, m),
                (3, _) => format!("SET {},{},{}", y, m, R[z as usize]),
                _ => unreachable!(),
            }
        }
    }

    fn ed(&mut self) -> String {
        let op = self.next();
        if op == crate::TRAP_OP {
            return "HOSTCALL".to_string();
        }
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;
        match x {
            1 => match z {
                0 => {
                    if y == 6 {
                        "IN (C)".to_string()
                    } else {
                        format!("IN {},(C)", R[y as usize])
                    }
                }
                1 => {
                    if y == 6 {
                        "OUT (C),0".to_string()
                    } else {
                        format!("OUT (C),{}", R[y as usize])
                    }
                }
                2 => {
                    let opn = if q == 0 { "SBC" } else { "ADC" };
                    format!("{} HL,{}", opn, RP[p as usize])
                }
                3 => {
                    let nn = self.imm16();
                    if q == 0 {
                        format!("LD ({}),{}", word(nn), RP[p as usize])
                    } else {
                        format!("LD {},({})", RP[p as usize], word(nn))
                    }
                }
                4 => "NEG".to_string(),
                5 => {
                    if y == 1 {
                        "RETI".to_string()
                    } else {
                        "RETN".to_string()
                    }
                }
                6 => format!("IM {}", [0, 0, 1, 2, 0, 0, 1, 2][y as usize]),
                7 => match y {
                    0 => "LD I,A".to_string(),
                    1 => "LD R,A".to_string(),
                    2 => "LD A,I".to_string(),
                    3 => "LD A,R".to_string(),
                    4 => "RRD".to_string(),
                    5 => "RLD".to_string(),
                    _ => defb_ed(op), // ED 77 / ED 7F are NOPs; show the bytes
                },
                _ => unreachable!(),
            },
            2 if z <= 3 && y >= 4 => BLOCK[(y - 4) as usize][z as usize].to_string(),
            // Everything else in the ED page is an undefined NOP on a 48K.
            _ => defb_ed(op),
        }
    }
}

fn idx_name(im: Index) -> &'static str {
    match im {
        Index::Hl => "HL",
        Index::Ix => "IX",
        Index::Iy => "IY",
    }
}

/// `rp[]` name with HL → IX/IY substitution (index 3 = SP).
fn rp_name(p: u8, im: Index) -> String {
    match p {
        2 => idx_name(im).to_string(),
        _ => RP[p as usize].to_string(),
    }
}

/// `rp2[]` name with HL → IX/IY substitution (index 3 = AF).
fn rp2_name(p: u8, im: Index) -> String {
    match p {
        2 => idx_name(im).to_string(),
        3 => "AF".to_string(),
        _ => RP[p as usize].to_string(),
    }
}

fn word(w: u16) -> String {
    format!("${w:04X}")
}

fn byte(b: u8) -> String {
    format!("${b:02X}")
}

/// Signed index displacement, e.g. `+5` / `-3`.
fn disp(d: i8) -> String {
    if d < 0 {
        format!("-{}", -(d as i16))
    } else {
        format!("+{}", d)
    }
}

fn defb_ed(op: u8) -> String {
    format!("DEFB $ED,{}", byte(op))
}
