//! The Dinu X/Y/Z/P/Q decode decomposition and the documented opcode bodies.
//!
//! Split the opcode byte:
//! ```text
//!    7 6   5 4 3   2 1 0
//!   [ x ] [  y  ] [  z  ]      p = y >> 1,  q = y & 1
//! ```
//! `exec` handles the base table; the `CB`/`ED`/`DD`/`FD` prefixes each get their
//! own decoder reusing the same field split. DD/FD thread an [`Index`] through so
//! `(HL)` becomes `(IX+d)`/`(IY+d)` and HL-as-16-bit becomes IX/IY.
//! See `docs/01-core-emulator-spec.md` §4.2.
//!
//! Scope: documented instruction set (M1). The undocumented IXH/IXL half-register
//! ops and the DDCB register-copy variants are M2 — DDCB here operates on memory
//! only, which is the documented behaviour.

use crate::bus::Bus;
use crate::cpu::{Cpu, Index};
use crate::flags::*;

/// MEM index in the `r[]` table = `(HL)` / `(IX+d)` / `(IY+d)`.
const MEM: u8 = 6;

impl Cpu {
    /// Execute a (possibly index-substituted) base-table opcode.
    pub(crate) fn exec<B: Bus>(&mut self, bus: &mut B, op: u8, im: Index) {
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;
        match x {
            0 => self.exec_x0(bus, y, z, p, q, im),
            1 => {
                if y == MEM && z == MEM {
                    self.halted = true; // HALT
                } else {
                    self.ld_r_r(bus, y, z, im);
                }
            }
            2 => {
                let v = self.src8(bus, z, im);
                self.alu(y, v);
            }
            3 => self.exec_x3(bus, y, z, p, q, im),
            _ => unreachable!(),
        }
    }

    /// Read an 8-bit operand by `r[]` index; index 6 reads `(HL)`/`(IX+d)`.
    fn src8<B: Bus>(&mut self, bus: &mut B, z: u8, im: Index) -> u8 {
        if z == MEM {
            let a = self.ptr_addr(bus, im);
            self.mem_read(bus, a)
        } else {
            // No memory operand: index 4/5 may be IXH/IXL (undocumented).
            self.r8_get_idx(z, im)
        }
    }

    /// LD r,r' block (x==1, HALT already carved out). When one operand is memory
    /// (`(IX+d)`), the *other* register is a real H/L; when both are registers,
    /// H/L substitute to IXH/IXL under a DD/FD prefix.
    fn ld_r_r<B: Bus>(&mut self, bus: &mut B, y: u8, z: u8, im: Index) {
        if z == MEM {
            let a = self.ptr_addr(bus, im);
            let v = self.mem_read(bus, a);
            self.r8_set(y, v); // y != 6 (else HALT); real register
        } else if y == MEM {
            let a = self.ptr_addr(bus, im);
            let v = self.r8_get(z); // real register
            self.mem_write(bus, a, v);
        } else {
            let v = self.r8_get_idx(z, im);
            self.r8_set_idx(y, im, v);
        }
    }

    // --- x == 0 --------------------------------------------------------------

    fn exec_x0<B: Bus>(&mut self, bus: &mut B, y: u8, z: u8, p: u8, q: u8, im: Index) {
        match z {
            0 => match y {
                0 => {} // NOP
                1 => self.ex_af(),
                2 => self.djnz(bus),
                3 => self.jr(bus, true),
                _ => {
                    let taken = self.cond(y - 4);
                    self.jr(bus, taken);
                }
            },
            1 => {
                if q == 0 {
                    // LD rp[p], nn
                    let nn = self.imm16(bus);
                    self.rp_set(p, im, nn);
                } else {
                    // ADD HL, rp[p]
                    bus.tick(7);
                    let a = self.hl_index(im);
                    let b = self.rp_get(p, im);
                    let r = self.add16(a, b);
                    self.hl_index_set(im, r);
                }
            }
            2 => self.x0_z2(bus, p, q, im),
            3 => {
                // INC/DEC rp[p] (no flags); 2T internal.
                bus.tick(2);
                let v = self.rp_get(p, im);
                let r = if q == 0 {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.rp_set(p, im, r);
            }
            4 => {
                // INC r[y]
                if y == MEM {
                    let a = self.ptr_addr(bus, im);
                    let v = self.mem_read(bus, a);
                    bus.tick(1);
                    let r = self.inc8(v);
                    self.mem_write(bus, a, r);
                } else {
                    let r = self.inc8(self.r8_get_idx(y, im));
                    self.r8_set_idx(y, im, r);
                }
            }
            5 => {
                // DEC r[y]
                if y == MEM {
                    let a = self.ptr_addr(bus, im);
                    let v = self.mem_read(bus, a);
                    bus.tick(1);
                    let r = self.dec8(v);
                    self.mem_write(bus, a, r);
                } else {
                    let r = self.dec8(self.r8_get_idx(y, im));
                    self.r8_set_idx(y, im, r);
                }
            }
            6 => {
                // LD r[y], n
                if y == MEM {
                    let a = self.ptr_addr(bus, im);
                    let n = self.imm8(bus);
                    self.mem_write(bus, a, n);
                } else {
                    let n = self.imm8(bus);
                    self.r8_set_idx(y, im, n);
                }
            }
            7 => match y {
                0 => self.rlca(),
                1 => self.rrca(),
                2 => self.rla(),
                3 => self.rra(),
                4 => self.daa(),
                5 => self.cpl(),
                6 => self.scf(),
                7 => self.ccf(),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }

    /// The z==2 sub-block: indirect loads of A / HL. The `p==2` (HL) forms honour
    /// the index prefix, becoming `LD (nn),IX/IY` and `LD IX/IY,(nn)`.
    fn x0_z2<B: Bus>(&mut self, bus: &mut B, p: u8, q: u8, im: Index) {
        match (q, p) {
            (0, 0) => {
                let a = self.regs.bc();
                self.regs.wz = (a.wrapping_add(1) & 0xff) | ((self.regs.a as u16) << 8);
                self.mem_write(bus, a, self.regs.a); // LD (BC),A
            }
            (0, 1) => {
                let a = self.regs.de();
                self.regs.wz = (a.wrapping_add(1) & 0xff) | ((self.regs.a as u16) << 8);
                self.mem_write(bus, a, self.regs.a); // LD (DE),A
            }
            (0, 2) => {
                let addr = self.imm16(bus); // LD (nn),HL/IX/IY
                self.regs.wz = addr.wrapping_add(1);
                let hl = self.hl_index(im);
                self.write16(bus, addr, hl);
            }
            (0, 3) => {
                let addr = self.imm16(bus); // LD (nn),A
                self.regs.wz = (addr.wrapping_add(1) & 0xff) | ((self.regs.a as u16) << 8);
                self.mem_write(bus, addr, self.regs.a);
            }
            (1, 0) => {
                let a = self.regs.bc(); // LD A,(BC)
                self.regs.wz = a.wrapping_add(1);
                self.regs.a = self.mem_read(bus, a);
            }
            (1, 1) => {
                let a = self.regs.de(); // LD A,(DE)
                self.regs.wz = a.wrapping_add(1);
                self.regs.a = self.mem_read(bus, a);
            }
            (1, 2) => {
                let addr = self.imm16(bus); // LD HL/IX/IY,(nn)
                self.regs.wz = addr.wrapping_add(1);
                let v = self.read16(bus, addr);
                self.hl_index_set(im, v);
            }
            (1, 3) => {
                let addr = self.imm16(bus); // LD A,(nn)
                self.regs.wz = addr.wrapping_add(1);
                self.regs.a = self.mem_read(bus, addr);
            }
            _ => unreachable!(),
        }
    }

    // --- x == 3 --------------------------------------------------------------

    fn exec_x3<B: Bus>(&mut self, bus: &mut B, y: u8, z: u8, p: u8, q: u8, im: Index) {
        match z {
            0 => {
                // RET cc[y]
                bus.tick(1);
                if self.cond(y) {
                    let pc = self.pop(bus);
                    self.regs.pc = pc;
                    self.regs.wz = pc;
                }
            }
            1 => {
                if q == 0 {
                    let v = self.pop(bus); // POP rp2[p]
                    self.rp2_set(p, im, v);
                } else {
                    match p {
                        0 => {
                            let pc = self.pop(bus); // RET
                            self.regs.pc = pc;
                            self.regs.wz = pc;
                        }
                        1 => self.exx(),
                        2 => self.regs.pc = self.hl_index(im), // JP (HL)/(IX)/(IY)
                        3 => {
                            bus.tick(2); // LD SP,HL/IX/IY
                            self.regs.sp = self.hl_index(im);
                        }
                        _ => unreachable!(),
                    }
                }
            }
            2 => {
                // JP cc[y], nn
                let addr = self.imm16(bus);
                self.regs.wz = addr;
                if self.cond(y) {
                    self.regs.pc = addr;
                }
            }
            3 => match y {
                0 => {
                    let addr = self.imm16(bus); // JP nn
                    self.regs.wz = addr;
                    self.regs.pc = addr;
                }
                1 => self.exec_cb(bus, im), // CB prefix
                2 => {
                    // OUT (n),A
                    let n = self.imm8(bus);
                    let port = (n as u16) | ((self.regs.a as u16) << 8);
                    self.regs.wz = ((self.regs.a as u16) << 8) | (n.wrapping_add(1) as u16);
                    self.io_write(bus, port, self.regs.a);
                }
                3 => {
                    // IN A,(n)
                    let n = self.imm8(bus);
                    let port = (n as u16) | ((self.regs.a as u16) << 8);
                    self.regs.a = self.io_read(bus, port);
                    self.regs.wz = port.wrapping_add(1);
                }
                4 => self.ex_sp_hl(bus, im), // EX (SP),HL/IX/IY
                5 => self.ex_de_hl(),        // EX DE,HL
                6 => {
                    self.iff1 = false; // DI
                    self.iff2 = false;
                }
                7 => self.ei_pending = true, // EI (delayed enable)
                _ => unreachable!(),
            },
            4 => {
                // CALL cc[y], nn
                let addr = self.imm16(bus);
                self.regs.wz = addr;
                if self.cond(y) {
                    let ret = self.regs.pc;
                    self.push(bus, ret);
                    self.regs.pc = addr;
                }
            }
            5 => {
                if q == 0 {
                    let v = self.rp2_get(p, im); // PUSH rp2[p]
                    self.push(bus, v);
                } else {
                    match p {
                        0 => {
                            let addr = self.imm16(bus); // CALL nn
                            self.regs.wz = addr;
                            let ret = self.regs.pc;
                            self.push(bus, ret);
                            self.regs.pc = addr;
                        }
                        1 => {
                            let op = self.fetch_op(bus); // DD prefix
                            self.exec(bus, op, Index::Ix);
                        }
                        2 => self.exec_ed(bus), // ED prefix
                        3 => {
                            let op = self.fetch_op(bus); // FD prefix
                            self.exec(bus, op, Index::Iy);
                        }
                        _ => unreachable!(),
                    }
                }
            }
            6 => {
                let n = self.imm8(bus); // ALU[y] n
                self.alu(y, n);
            }
            7 => {
                // RST y*8
                let ret = self.regs.pc;
                self.push(bus, ret);
                let target = (y as u16) * 8;
                self.regs.pc = target;
                self.regs.wz = target;
            }
            _ => unreachable!(),
        }
    }

    // --- CB prefix -----------------------------------------------------------

    fn exec_cb<B: Bus>(&mut self, bus: &mut B, im: Index) {
        if let Index::Hl = im {
            let op = self.fetch_op(bus);
            let x = op >> 6;
            let y = (op >> 3) & 7;
            let z = op & 7;
            if z == MEM {
                let addr = self.regs.hl();
                self.cb_mem(bus, x, y, z, addr);
            } else {
                let v = self.r8_get(z);
                if x == 1 {
                    // BIT n,r: undocumented bits 5/3 from the register value.
                    self.bit(y, v, v);
                } else {
                    let r = self.cb_rmw(x, y, v);
                    self.r8_set(z, r);
                }
            }
        } else {
            // DDCB / FDCB: displacement precedes the opcode byte; the operation
            // targets (IX+d)/(IY+d). For z != 6 the result is *also* copied into
            // the real register r[z] (undocumented), and BIT ignores z.
            let base = self.hl_index(im);
            let d = self.imm8(bus) as i8 as i16 as u16;
            let addr = base.wrapping_add(d);
            self.regs.wz = addr;
            let op = self.imm8(bus); // not an M1 fetch: no R increment
            bus.tick(2);
            let x = op >> 6;
            let y = (op >> 3) & 7;
            let z = op & 7;
            self.cb_mem(bus, x, y, z, addr);
        }
    }

    /// CB op on a memory cell at `addr` (read, modify, write; BIT is read-only).
    /// `z` is the opcode's low field: for DDCB/FDCB with `z != 6` the result is
    /// also written into register `r[z]` (the undocumented copy).
    fn cb_mem<B: Bus>(&mut self, bus: &mut B, x: u8, y: u8, z: u8, addr: u16) {
        let v = self.mem_read(bus, addr);
        bus.tick(1);
        if x == 1 {
            // BIT n,(addr): undocumented bits 5/3 come from MEMPTR high byte.
            self.bit(y, v, (self.regs.wz >> 8) as u8);
        } else {
            let r = self.cb_rmw(x, y, v);
            self.mem_write(bus, addr, r);
            if z != MEM {
                self.r8_set(z, r); // DDCB/FDCB undocumented register copy
            }
        }
    }

    /// Apply a non-BIT CB op (rotate/shift, RES, SET).
    fn cb_rmw(&mut self, x: u8, y: u8, v: u8) -> u8 {
        match x {
            0 => self.cb_rot(y, v),
            2 => v & !(1 << y), // RES
            3 => v | (1 << y),  // SET
            _ => unreachable!("cb_rmw x={x}"),
        }
    }

    // --- ED prefix -----------------------------------------------------------

    fn exec_ed<B: Bus>(&mut self, bus: &mut B) {
        let op = self.fetch_op(bus);
        // `ED FE` is the reserved host-trap opcode (otherwise an undefined NOP).
        // The two fetches already charged 8T; a handler may add latency.
        if op == crate::TRAP_OP {
            let extra = bus.host_trap(&mut self.regs);
            bus.tick(extra);
            return;
        }
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;
        match x {
            1 => self.exec_ed_x1(bus, y, z, p, q),
            2 => {
                if z <= 3 && y >= 4 {
                    self.block_op(bus, y, z);
                }
                // else: NONI/NOP
            }
            _ => {
                // x==0 / x==3 ED slots are undefined NOPs on a 48K.
            }
        }
    }

    fn exec_ed_x1<B: Bus>(&mut self, bus: &mut B, y: u8, z: u8, p: u8, q: u8) {
        match z {
            0 => {
                // IN r[y],(C)  (y==6 → just sets flags)
                let port = self.regs.bc();
                let v = self.io_read(bus, port);
                self.regs.wz = port.wrapping_add(1);
                self.regs.f = (self.regs.f & CF) | SZ53P[v as usize];
                self.q = self.regs.f;
                if y != MEM {
                    self.r8_set(y, v);
                }
            }
            1 => {
                // OUT (C),r[y]  (y==6 → OUT (C),0)
                let port = self.regs.bc();
                let v = if y == MEM { 0 } else { self.r8_get(y) };
                self.io_write(bus, port, v);
                self.regs.wz = port.wrapping_add(1);
            }
            2 => {
                bus.tick(7);
                if q == 0 {
                    self.sbc16(self.regs_rp(p)); // SBC HL,rp
                } else {
                    self.adc16(self.regs_rp(p)); // ADC HL,rp
                }
            }
            3 => {
                let addr = self.imm16(bus);
                self.regs.wz = addr.wrapping_add(1);
                if q == 0 {
                    let v = self.regs_rp(p); // LD (nn),rp
                    self.write16(bus, addr, v);
                } else {
                    let v = self.read16(bus, addr); // LD rp,(nn)
                    self.set_regs_rp(p, v);
                }
            }
            4 => self.neg(),
            5 => {
                // RETN (y==1 → RETI; both pop PC and copy iff2→iff1 on a 48K).
                let pc = self.pop(bus);
                self.regs.pc = pc;
                self.regs.wz = pc;
                self.iff1 = self.iff2;
            }
            6 => self.im = [0u8, 0, 1, 2, 0, 0, 1, 2][y as usize], // IM n
            7 => match y {
                0 => {
                    bus.tick(1);
                    self.regs.i = self.regs.a; // LD I,A
                }
                1 => {
                    bus.tick(1);
                    self.regs.r = self.regs.a; // LD R,A
                }
                2 => {
                    bus.tick(1);
                    let v = self.regs.i; // LD A,I
                    self.regs.a = v;
                    self.ir_to_a_flags(v);
                }
                3 => {
                    bus.tick(1);
                    let v = self.regs.r; // LD A,R
                    self.regs.a = v;
                    self.ir_to_a_flags(v);
                }
                4 => self.rrd(bus),
                5 => self.rld(bus),
                _ => {} // 6,7: NOP
            },
            _ => unreachable!(),
        }
    }

    /// LD A,I / LD A,R flag effects (PV = IFF2).
    fn ir_to_a_flags(&mut self, v: u8) {
        let mut f = (self.regs.f & CF) | (v & (SF | YF | XF));
        if v == 0 {
            f |= ZF;
        }
        if self.iff2 {
            f |= PF;
        }
        self.regs.f = f;
        self.q = self.regs.f;
    }

    /// rp[] without index substitution (ED ops never use IX/IY).
    fn regs_rp(&self, p: u8) -> u16 {
        match p {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            2 => self.regs.hl(),
            3 => self.regs.sp,
            _ => unreachable!(),
        }
    }

    fn set_regs_rp(&mut self, p: u8, v: u16) {
        match p {
            0 => self.regs.set_bc(v),
            1 => self.regs.set_de(v),
            2 => self.regs.set_hl(v),
            3 => self.regs.sp = v,
            _ => unreachable!(),
        }
    }

    /// RRD: rotate nibbles between A and (HL).
    fn rrd<B: Bus>(&mut self, bus: &mut B) {
        let addr = self.regs.hl();
        let m = self.mem_read(bus, addr);
        let a = self.regs.a;
        let new_m = (a << 4) | (m >> 4);
        let new_a = (a & 0xf0) | (m & 0x0f);
        bus.tick(4);
        self.mem_write(bus, addr, new_m);
        self.regs.a = new_a;
        self.regs.f = (self.regs.f & CF) | SZ53P[new_a as usize];
        self.regs.wz = addr.wrapping_add(1);
        self.q = self.regs.f;
    }

    /// RLD: rotate nibbles the other way.
    fn rld<B: Bus>(&mut self, bus: &mut B) {
        let addr = self.regs.hl();
        let m = self.mem_read(bus, addr);
        let a = self.regs.a;
        let new_m = (m << 4) | (a & 0x0f);
        let new_a = (a & 0xf0) | (m >> 4);
        bus.tick(4);
        self.mem_write(bus, addr, new_m);
        self.regs.a = new_a;
        self.regs.f = (self.regs.f & CF) | SZ53P[new_a as usize];
        self.regs.wz = addr.wrapping_add(1);
        self.q = self.regs.f;
    }

    // --- ED block ops --------------------------------------------------------

    /// `y` in 4..7 selects the increment/decrement+repeat variant; `z` in 0..3
    /// selects LD / CP / IN / OUT.
    fn block_op<B: Bus>(&mut self, bus: &mut B, y: u8, z: u8) {
        let inc = y & 1 == 0; // even y = increment (LDI/CPI/...), odd = decrement
        let repeat = y >= 6; // LDIR/CPIR/INIR/OTIR and the DR forms
        match z {
            0 => self.block_ld(bus, inc, repeat),
            1 => self.block_cp(bus, inc, repeat),
            2 => self.block_in(bus, inc, repeat),
            3 => self.block_out(bus, inc, repeat),
            _ => unreachable!(),
        }
    }

    fn block_ld<B: Bus>(&mut self, bus: &mut B, inc: bool, repeat: bool) {
        let hl = self.regs.hl();
        let de = self.regs.de();
        let v = self.mem_read(bus, hl);
        self.mem_write(bus, de, v);
        bus.tick(2);
        let step = if inc { 1u16 } else { 0xffff };
        self.regs.set_hl(hl.wrapping_add(step));
        self.regs.set_de(de.wrapping_add(step));
        let bc = self.regs.bc().wrapping_sub(1);
        self.regs.set_bc(bc);

        // Flags: H=N=0, PV=(BC!=0); YF/XF from (A+v) undocumented bits.
        let n = self.regs.a.wrapping_add(v);
        let mut f = self.regs.f & (SF | ZF | CF);
        if bc != 0 {
            f |= PF;
        }
        if n & 0x02 != 0 {
            f |= YF;
        }
        if n & 0x08 != 0 {
            f |= XF;
        }
        self.regs.f = f;
        self.q = self.regs.f;

        if repeat && bc != 0 {
            bus.tick(5);
            self.regs.pc = self.regs.pc.wrapping_sub(2);
            self.regs.wz = self.regs.pc.wrapping_add(1);
        }
    }

    fn block_cp<B: Bus>(&mut self, bus: &mut B, inc: bool, repeat: bool) {
        let hl = self.regs.hl();
        let v = self.mem_read(bus, hl);
        bus.tick(5);
        let a = self.regs.a;
        let res = a.wrapping_sub(v);
        let half = (a & 0x0f).wrapping_sub(v & 0x0f) & 0x10 != 0;

        let step = if inc { 1u16 } else { 0xffff };
        self.regs.set_hl(hl.wrapping_add(step));
        let bc = self.regs.bc().wrapping_sub(1);
        self.regs.set_bc(bc);

        let mut f = (self.regs.f & CF) | NF | (res & SF);
        if res == 0 {
            f |= ZF;
        }
        if half {
            f |= HF;
        }
        if bc != 0 {
            f |= PF;
        }
        let n = res.wrapping_sub(half as u8);
        if n & 0x02 != 0 {
            f |= YF;
        }
        if n & 0x08 != 0 {
            f |= XF;
        }
        self.regs.f = f;
        self.q = self.regs.f;
        self.regs.wz = if inc {
            self.regs.wz.wrapping_add(1)
        } else {
            self.regs.wz.wrapping_sub(1)
        };

        if repeat && bc != 0 && res != 0 {
            bus.tick(5);
            self.regs.pc = self.regs.pc.wrapping_sub(2);
            self.regs.wz = self.regs.pc.wrapping_add(1);
        }
    }

    fn block_in<B: Bus>(&mut self, bus: &mut B, inc: bool, repeat: bool) {
        bus.tick(1);
        let port = self.regs.bc();
        let v = self.io_read(bus, port);
        let hl = self.regs.hl();
        self.mem_write(bus, hl, v);
        self.regs.wz = if inc {
            port.wrapping_add(1)
        } else {
            port.wrapping_sub(1)
        };

        let b = self.regs.b.wrapping_sub(1);
        self.regs.b = b;
        let step = if inc { 1u16 } else { 0xffff };
        self.regs.set_hl(hl.wrapping_add(step));

        // Documented-ish flag model for INI/IND.
        let mut f = SZ53[b as usize];
        if v & 0x80 != 0 {
            f |= NF;
        }
        let k = if inc {
            v as u16 + (self.regs.c.wrapping_add(1)) as u16
        } else {
            v as u16 + (self.regs.c.wrapping_sub(1)) as u16
        };
        if k > 0xff {
            f |= HF | CF;
        }
        if SZ53P[((k as u8 & 7) ^ b) as usize] & PF != 0 {
            f |= PF;
        }
        self.regs.f = f;
        self.q = self.regs.f;

        if repeat && b != 0 {
            bus.tick(5);
            self.regs.pc = self.regs.pc.wrapping_sub(2);
        }
    }

    fn block_out<B: Bus>(&mut self, bus: &mut B, inc: bool, repeat: bool) {
        bus.tick(1);
        let hl = self.regs.hl();
        let v = self.mem_read(bus, hl);
        let b = self.regs.b.wrapping_sub(1);
        self.regs.b = b;
        let port = self.regs.bc();
        self.io_write(bus, port, v);
        let step = if inc { 1u16 } else { 0xffff };
        self.regs.set_hl(hl.wrapping_add(step));
        self.regs.wz = if inc {
            port.wrapping_add(1)
        } else {
            port.wrapping_sub(1)
        };

        let mut f = SZ53[b as usize];
        if v & 0x80 != 0 {
            f |= NF;
        }
        let k = v as u16 + self.regs.l as u16;
        if k > 0xff {
            f |= HF | CF;
        }
        if SZ53P[((k as u8 & 7) ^ b) as usize] & PF != 0 {
            f |= PF;
        }
        self.regs.f = f;
        self.q = self.regs.f;

        if repeat && b != 0 {
            bus.tick(5);
            self.regs.pc = self.regs.pc.wrapping_sub(2);
        }
    }

    // --- shared control-flow helpers ----------------------------------------

    /// Evaluate condition `cc[y]`: NZ,Z,NC,C,PO,PE,P,M.
    fn cond(&self, y: u8) -> bool {
        match y {
            0 => !self.flag(ZF),
            1 => self.flag(ZF),
            2 => !self.flag(CF),
            3 => self.flag(CF),
            4 => !self.flag(PF),
            5 => self.flag(PF),
            6 => !self.flag(SF),
            7 => self.flag(SF),
            _ => unreachable!(),
        }
    }

    fn jr<B: Bus>(&mut self, bus: &mut B, taken: bool) {
        let d = self.imm8(bus) as i8 as i16 as u16;
        if taken {
            bus.tick(5);
            self.regs.pc = self.regs.pc.wrapping_add(d);
            self.regs.wz = self.regs.pc;
        }
    }

    fn djnz<B: Bus>(&mut self, bus: &mut B) {
        bus.tick(1);
        let d = self.imm8(bus) as i8 as i16 as u16;
        self.regs.b = self.regs.b.wrapping_sub(1);
        if self.regs.b != 0 {
            bus.tick(5);
            self.regs.pc = self.regs.pc.wrapping_add(d);
            self.regs.wz = self.regs.pc;
        }
    }

    fn ex_af(&mut self) {
        core::mem::swap(&mut self.regs.a, &mut self.regs.a_);
        core::mem::swap(&mut self.regs.f, &mut self.regs.f_);
    }

    fn exx(&mut self) {
        core::mem::swap(&mut self.regs.b, &mut self.regs.b_);
        core::mem::swap(&mut self.regs.c, &mut self.regs.c_);
        core::mem::swap(&mut self.regs.d, &mut self.regs.d_);
        core::mem::swap(&mut self.regs.e, &mut self.regs.e_);
        core::mem::swap(&mut self.regs.h, &mut self.regs.h_);
        core::mem::swap(&mut self.regs.l, &mut self.regs.l_);
    }

    fn ex_de_hl(&mut self) {
        core::mem::swap(&mut self.regs.d, &mut self.regs.h);
        core::mem::swap(&mut self.regs.e, &mut self.regs.l);
    }

    fn ex_sp_hl<B: Bus>(&mut self, bus: &mut B, im: Index) {
        let sp = self.regs.sp;
        let lo = self.mem_read(bus, sp);
        let hi = self.mem_read(bus, sp.wrapping_add(1));
        bus.tick(1);
        let hl = self.hl_index(im);
        self.mem_write(bus, sp.wrapping_add(1), (hl >> 8) as u8);
        self.mem_write(bus, sp, hl as u8);
        bus.tick(2);
        let v = (lo as u16) | ((hi as u16) << 8);
        self.hl_index_set(im, v);
        self.regs.wz = v;
    }
}
