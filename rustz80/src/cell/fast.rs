//! Decode-once fast executor for straight-line batch cells — the [`super::Runner::run_many_fast`]
//! hot path. See the module header for why it's safe (differential-checked, fallback on anything
//! outside the arithmetic subset it accepts).

use super::SP_TOP;
use crate::ORG;

/// A decoded instruction — only the arithmetic opcode subset straight-line scoring
/// cells emit. `rr` in `LdMem16`: 1=DE, 2=HL. `r` (8-bit): 0=B,1=C,2=D,3=E,4=H,5=L,7=A
/// (`(HL)` and the rest fall back). `alu`: 0=AND, 1=XOR, 2=OR.
pub(super) enum Op {
    LdHlImm(u16),
    Ld16MemHl(u16),
    LdMem16(u16, u8),
    LdReg(u8, u8),
    LdAImm(u8),
    AddHl(u8),
    ExDeHl,
    PushHl,
    PopDe,
    AluReg(u8, u8),
    Mul,
    Div,
    Ret,
}

fn rd16(code: &[u8], i: usize) -> Option<u16> {
    Some((*code.get(i)? as u16) | ((*code.get(i + 1)? as u16) << 8))
}

/// Decode the straight-line body at `entry`, or `None` if it leaves the supported
/// subset (any branch/call/`halt`/shift/`(HL)`/unknown op) — the caller then falls back.
pub(super) fn decode(code: &[u8], entry: u16) -> Option<Vec<Op>> {
    let mut ops = Vec::new();
    let mut pc = entry.checked_sub(ORG)? as usize;
    let mut last_a: Option<u8> = None; // the operand of the latest `LD A,n` (trap id)
    loop {
        let op = *code.get(pc)?;
        // LD r,r' — 8-bit register moves, excluding any `(HL)` form (index 6, which
        // includes HALT at 0x76) so unsupported ones fall through to the fallback `_`.
        if (0x40..=0x7F).contains(&op) && (op >> 3) & 7 != 6 && op & 7 != 6 {
            ops.push(Op::LdReg((op >> 3) & 7, op & 7));
            pc += 1;
            last_a = None;
            continue;
        }
        // AND/XOR/OR r (no `(HL)` form) — 8-bit ALU into A.
        if (0xA0..=0xB7).contains(&op) && (op & 7) != 6 {
            let alu = match op >> 3 {
                0b10100 => 0,
                0b10101 => 1,
                _ => 2,
            };
            ops.push(Op::AluReg(alu, op & 7));
            pc += 1;
            last_a = None;
            continue;
        }
        let (mut adv, mut clear_a) = (1usize, true);
        match op {
            0x21 => {
                ops.push(Op::LdHlImm(rd16(code, pc + 1)?));
                adv = 3;
            }
            0x2A => {
                ops.push(Op::Ld16MemHl(rd16(code, pc + 1)?));
                adv = 3;
            }
            0x22 => {
                ops.push(Op::LdMem16(rd16(code, pc + 1)?, 2));
                adv = 3;
            }
            0x3E => {
                let n = *code.get(pc + 1)?;
                ops.push(Op::LdAImm(n));
                last_a = Some(n);
                clear_a = false;
                adv = 2;
            }
            0x19 => ops.push(Op::AddHl(1)),
            0x29 => ops.push(Op::AddHl(2)),
            0xEB => ops.push(Op::ExDeHl),
            0xE5 => ops.push(Op::PushHl),
            0xD1 => ops.push(Op::PopDe),
            0xED => {
                match *code.get(pc + 1)? {
                    0x53 => {
                        ops.push(Op::LdMem16(rd16(code, pc + 2)?, 1)); // LD (nn),DE
                        adv = 4;
                    }
                    0xFE => {
                        match last_a? {
                            0x10 => ops.push(Op::Mul),
                            0x11 => ops.push(Op::Div),
                            _ => return None, // FILL / HALT / unknown → fall back
                        }
                        adv = 2;
                    }
                    _ => return None,
                }
            }
            0xC9 => {
                ops.push(Op::Ret);
                return Some(ops);
            }
            _ => return None, // branch / call / shift / `(HL)` / anything else → fall back
        }
        if clear_a {
            last_a = None;
        }
        pc += adv;
    }
}

/// Replay `ops` with native registers over the cell's memory (resetting the previous
/// run's writes first, like the authentic path). Returns `[HL, DE, BC]`.
pub(super) fn run(
    ops: &[Op],
    mem: &mut [u8],
    seen: &mut [bool],
    touched: &mut Vec<u16>,
    args: &[u16],
) -> [u16; 3] {
    for &t in touched.iter() {
        mem[t as usize] = 0;
        seen[t as usize] = false;
    }
    touched.clear();

    let (mut bc, mut de, mut hl) = (
        args.get(2).copied().unwrap_or(0),
        args.get(1).copied().unwrap_or(0),
        args.first().copied().unwrap_or(0),
    );
    let mut a: u8 = 0;
    let mut sp = SP_TOP;

    let rd16 = |m: &[u8], at: u16| {
        (m[at as usize] as u16) | ((m[at.wrapping_add(1) as usize] as u16) << 8)
    };
    macro_rules! wr8 {
        ($at:expr, $v:expr) => {{
            let at = $at as usize;
            mem[at] = $v;
            if !seen[at] {
                seen[at] = true;
                touched.push(at as u16);
            }
        }};
    }
    macro_rules! wr16 {
        ($at:expr, $v:expr) => {{
            let (at, v): (u16, u16) = ($at, $v);
            wr8!(at, v as u8);
            wr8!(at.wrapping_add(1), (v >> 8) as u8);
        }};
    }
    macro_rules! get8 {
        ($r:expr) => {
            match $r {
                0 => (bc >> 8) as u8,
                1 => bc as u8,
                2 => (de >> 8) as u8,
                3 => de as u8,
                4 => (hl >> 8) as u8,
                5 => hl as u8,
                _ => a,
            }
        };
    }
    macro_rules! set8 {
        ($r:expr, $v:expr) => {{
            let v: u8 = $v;
            match $r {
                0 => bc = (bc & 0x00FF) | ((v as u16) << 8),
                1 => bc = (bc & 0xFF00) | v as u16,
                2 => de = (de & 0x00FF) | ((v as u16) << 8),
                3 => de = (de & 0xFF00) | v as u16,
                4 => hl = (hl & 0x00FF) | ((v as u16) << 8),
                5 => hl = (hl & 0xFF00) | v as u16,
                _ => a = v,
            }
        }};
    }

    for op in ops {
        match *op {
            Op::LdHlImm(n) => hl = n,
            Op::Ld16MemHl(at) => hl = rd16(mem, at),
            Op::LdMem16(at, rr) => wr16!(at, if rr == 1 { de } else { hl }),
            Op::LdReg(dst, src) => {
                let v = get8!(src);
                set8!(dst, v);
            }
            Op::LdAImm(n) => a = n,
            Op::AddHl(rr) => hl = hl.wrapping_add(if rr == 1 { de } else { hl }),
            Op::ExDeHl => core::mem::swap(&mut de, &mut hl),
            Op::PushHl => {
                sp = sp.wrapping_sub(2);
                wr16!(sp, hl);
            }
            Op::PopDe => {
                de = rd16(mem, sp);
                sp = sp.wrapping_add(2);
            }
            Op::AluReg(alu, r) => {
                let b = get8!(r);
                a = match alu {
                    0 => a & b,
                    1 => a ^ b,
                    _ => a | b,
                };
            }
            Op::Mul => hl = bc.wrapping_mul(de),
            Op::Div => match bc.checked_div(de) {
                Some(q) => {
                    hl = q;
                    de = bc % de;
                }
                None => hl = 0xFFFF,
            },
            Op::Ret => break,
        }
    }
    [hl, de, bc]
}
