//! Register file + the `Cpu` struct, its `step()` entry point, and the low-level
//! bus/stack/register helpers the decoder builds on.

use crate::bus::Bus;

/// Why an outer run loop stopped. Mirrors the MCP layer's `StopReason`
/// (`docs/02-mcp-server-layer-spec.md` §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Ran the requested quantum (instruction/frame count) to completion.
    Completed,
    /// Hit a breakpoint at the given address.
    Breakpoint(u16),
    /// Executed HALT and is waiting for an interrupt.
    Halt,
    /// Exhausted a T-state budget.
    Budget,
}

/// Which 16-bit register stands in for HL this instruction: HL itself, or IX/IY
/// under a `DD`/`FD` prefix. `(HL)` becomes `(IX+d)`/`(IY+d)` accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Index {
    Hl,
    Ix,
    Iy,
}

/// The Z80 register file. Explicit `u8`s with pair accessors — codegen is
/// identical to a blob and the field access reads better.
#[derive(Default, Debug, Clone)]
pub struct Regs {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    // shadow set
    pub a_: u8,
    pub f_: u8,
    pub b_: u8,
    pub c_: u8,
    pub d_: u8,
    pub e_: u8,
    pub h_: u8,
    pub l_: u8,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
    pub i: u8,
    pub r: u8,
    /// MEMPTR — needed for `BIT n,(HL)` flag bits & some XF/YF.
    pub wz: u16,
}

macro_rules! pair {
    ($get:ident, $set:ident, $hi:ident, $lo:ident) => {
        #[inline]
        pub fn $get(&self) -> u16 {
            (self.$hi as u16) << 8 | self.$lo as u16
        }
        #[inline]
        pub fn $set(&mut self, v: u16) {
            self.$hi = (v >> 8) as u8;
            self.$lo = v as u8;
        }
    };
}

impl Regs {
    pair!(af, set_af, a, f);
    pair!(bc, set_bc, b, c);
    pair!(de, set_de, d, e);
    pair!(hl, set_hl, h, l);
}

/// The CPU: registers plus the bits of state that aren't registers.
#[derive(Default, Debug, Clone)]
pub struct Cpu {
    pub regs: Regs,
    pub iff1: bool,
    pub iff2: bool,
    /// Interrupt mode: 0, 1, or 2.
    pub im: u8,
    pub halted: bool,
    /// The Q latch (`docs/01-core-emulator-spec.md` §4.3): set to `F` by any
    /// instruction that writes flags, reset to 0 by one that doesn't. SCF/CCF
    /// consult the *previous* instruction's value (`q_prev`).
    pub q: u8,
    /// `q` as it stood after the previous instruction — what SCF/CCF read.
    pub q_prev: u8,
    /// Set true while `EI`'s one-instruction interrupt-enable delay is pending.
    pub ei_pending: bool,
}

impl Cpu {
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute one instruction against `bus`.
    pub fn step<B: Bus>(&mut self, bus: &mut B) {
        if self.halted {
            // HALT executes NOPs (advancing R and the clock) until an interrupt.
            self.inc_r();
            bus.tick(4);
            return;
        }

        // EI enables interrupts only *after* the following instruction.
        let was_ei_pending = self.ei_pending;
        // Q-quirk bookkeeping: remember the previous instruction's latch, then
        // reset. Any flag-writing op this instruction sets `q = F` again.
        self.q_prev = self.q;
        self.q = 0;

        let op = self.fetch_op(bus);
        self.exec(bus, op, Index::Hl);

        if was_ei_pending {
            self.ei_pending = false;
            self.iff1 = true;
            self.iff2 = true;
        }
    }

    /// Try to accept a maskable interrupt (the ULA's `/INT`), called at an
    /// instruction boundary. Returns true if serviced. Masked when `iff1` is
    /// clear (and ignored right after `EI`, whose enable is still pending). On a
    /// 48K the data bus floats to 0xFF, so IM 0/1 both vector to `RST 38h`.
    pub fn interrupt<B: Bus>(&mut self, bus: &mut B) -> bool {
        if !self.iff1 || self.ei_pending {
            return false;
        }
        // An interrupt wakes the CPU from HALT (PC already points past it).
        self.halted = false;
        self.iff1 = false;
        self.iff2 = false;
        self.inc_r(); // the interrupt-acknowledge cycle bumps R
        self.q = 0;

        let pc = self.regs.pc;
        match self.im {
            2 => {
                // IM 2: vector table indexed by I; low byte is the bus value (0xFF).
                bus.tick(6);
                self.push(bus, pc);
                let vector = ((self.regs.i as u16) << 8) | 0x00FF;
                let target = self.read16(bus, vector);
                self.regs.pc = target;
                self.regs.wz = target; // 19T total
            }
            _ => {
                bus.tick(6);
                self.push(bus, pc);
                self.regs.pc = 0x0038;
                self.regs.wz = 0x0038; // 13T total
            }
        }
        true
    }

    // --- M1 opcode fetch -----------------------------------------------------

    /// M1 opcode fetch: read at PC, advance PC, bump R, account 4T. The fetch is
    /// a contended M-cycle, so it contends before the read (a no-op on buses
    /// without contention). Used for the leading byte and each prefix byte.
    pub(crate) fn fetch_op<B: Bus>(&mut self, bus: &mut B) -> u8 {
        bus.contend(self.regs.pc, 4);
        let op = bus.read(self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        self.inc_r();
        bus.tick(4);
        op
    }

    /// R is a 7-bit counter; bit 7 is preserved across increments.
    #[inline]
    pub(crate) fn inc_r(&mut self) {
        let r = self.regs.r;
        self.regs.r = (r & 0x80) | (r.wrapping_add(1) & 0x7f);
    }

    // --- Operand / memory / IO access (3T mem, 4T io) ------------------------

    /// Read an immediate byte at PC (operand fetch, 3T).
    pub(crate) fn imm8<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let v = self.mem_read(bus, self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        v
    }

    /// Read an immediate little-endian word at PC (two operand fetches).
    pub(crate) fn imm16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.imm8(bus) as u16;
        let hi = self.imm8(bus) as u16;
        lo | (hi << 8)
    }

    pub(crate) fn mem_read<B: Bus>(&mut self, bus: &mut B, addr: u16) -> u8 {
        bus.contend(addr, 3);
        let v = bus.read(addr);
        bus.tick(3);
        v
    }

    pub(crate) fn mem_write<B: Bus>(&mut self, bus: &mut B, addr: u16, val: u8) {
        bus.contend(addr, 3);
        bus.write(addr, val);
        bus.tick(3);
    }

    pub(crate) fn io_read<B: Bus>(&mut self, bus: &mut B, port: u16) -> u8 {
        let v = bus.input(port);
        bus.tick(4);
        v
    }

    pub(crate) fn io_write<B: Bus>(&mut self, bus: &mut B, port: u16, val: u8) {
        bus.output(port, val);
        bus.tick(4);
    }

    pub(crate) fn read16<B: Bus>(&mut self, bus: &mut B, addr: u16) -> u16 {
        let lo = self.mem_read(bus, addr) as u16;
        let hi = self.mem_read(bus, addr.wrapping_add(1)) as u16;
        lo | (hi << 8)
    }

    pub(crate) fn write16<B: Bus>(&mut self, bus: &mut B, addr: u16, val: u16) {
        self.mem_write(bus, addr, val as u8);
        self.mem_write(bus, addr.wrapping_add(1), (val >> 8) as u8);
    }

    pub(crate) fn push<B: Bus>(&mut self, bus: &mut B, val: u16) {
        bus.tick(1); // internal cycle before the writes
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.mem_write(bus, self.regs.sp, (val >> 8) as u8);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.mem_write(bus, self.regs.sp, val as u8);
    }

    pub(crate) fn pop<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.mem_read(bus, self.regs.sp) as u16;
        self.regs.sp = self.regs.sp.wrapping_add(1);
        let hi = self.mem_read(bus, self.regs.sp) as u16;
        self.regs.sp = self.regs.sp.wrapping_add(1);
        lo | (hi << 8)
    }

    // --- Register access by table index -------------------------------------

    /// Read 8-bit register by index (0=B..5=L,7=A). Index 6 = `(HL)` is handled
    /// by the decoder via `ptr_addr`, never here.
    #[inline]
    pub(crate) fn r8_get(&self, idx: u8) -> u8 {
        match idx {
            0 => self.regs.b,
            1 => self.regs.c,
            2 => self.regs.d,
            3 => self.regs.e,
            4 => self.regs.h,
            5 => self.regs.l,
            7 => self.regs.a,
            _ => unreachable!("r8_get index {idx}"),
        }
    }

    #[inline]
    pub(crate) fn r8_set(&mut self, idx: u8, v: u8) {
        match idx {
            0 => self.regs.b = v,
            1 => self.regs.c = v,
            2 => self.regs.d = v,
            3 => self.regs.e = v,
            4 => self.regs.h = v,
            5 => self.regs.l = v,
            7 => self.regs.a = v,
            _ => unreachable!("r8_set index {idx}"),
        }
    }

    /// 8-bit register read with IX/IY half-register substitution. Under a DD/FD
    /// prefix and *with no memory operand in the instruction*, index 4 reads
    /// IXH/IYH and index 5 reads IXL/IYL (undocumented). The decoder only calls
    /// this where that substitution applies; the plain `r8_get` is used for the
    /// register that accompanies a `(IX+d)` operand.
    #[inline]
    pub(crate) fn r8_get_idx(&self, idx: u8, im: Index) -> u8 {
        match (im, idx) {
            (Index::Ix, 4) => (self.regs.ix >> 8) as u8,
            (Index::Ix, 5) => self.regs.ix as u8,
            (Index::Iy, 4) => (self.regs.iy >> 8) as u8,
            (Index::Iy, 5) => self.regs.iy as u8,
            _ => self.r8_get(idx),
        }
    }

    #[inline]
    pub(crate) fn r8_set_idx(&mut self, idx: u8, im: Index, v: u8) {
        match (im, idx) {
            (Index::Ix, 4) => self.regs.ix = (self.regs.ix & 0x00ff) | ((v as u16) << 8),
            (Index::Ix, 5) => self.regs.ix = (self.regs.ix & 0xff00) | (v as u16),
            (Index::Iy, 4) => self.regs.iy = (self.regs.iy & 0x00ff) | ((v as u16) << 8),
            (Index::Iy, 5) => self.regs.iy = (self.regs.iy & 0xff00) | (v as u16),
            _ => self.r8_set(idx, v),
        }
    }

    /// 16-bit register pair for the `rp[]` table (index 3 = SP), honouring the
    /// IX/IY substitution for HL.
    #[inline]
    pub(crate) fn rp_get(&self, idx: u8, im: Index) -> u16 {
        match idx {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            2 => self.hl_index(im),
            3 => self.regs.sp,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub(crate) fn rp_set(&mut self, idx: u8, im: Index, v: u16) {
        match idx {
            0 => self.regs.set_bc(v),
            1 => self.regs.set_de(v),
            2 => self.hl_index_set(im, v),
            3 => self.regs.sp = v,
            _ => unreachable!(),
        }
    }

    /// 16-bit register pair for the `rp2[]` table (index 3 = AF), used by
    /// PUSH/POP. HL still substitutes to IX/IY.
    #[inline]
    pub(crate) fn rp2_get(&self, idx: u8, im: Index) -> u16 {
        match idx {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            2 => self.hl_index(im),
            3 => self.regs.af(),
            _ => unreachable!(),
        }
    }

    #[inline]
    pub(crate) fn rp2_set(&mut self, idx: u8, im: Index, v: u16) {
        match idx {
            0 => self.regs.set_bc(v),
            1 => self.regs.set_de(v),
            2 => self.hl_index_set(im, v),
            3 => self.regs.set_af(v),
            _ => unreachable!(),
        }
    }

    /// The active "HL" 16-bit value for this index mode.
    #[inline]
    pub(crate) fn hl_index(&self, im: Index) -> u16 {
        match im {
            Index::Hl => self.regs.hl(),
            Index::Ix => self.regs.ix,
            Index::Iy => self.regs.iy,
        }
    }

    #[inline]
    pub(crate) fn hl_index_set(&mut self, im: Index, v: u16) {
        match im {
            Index::Hl => self.regs.set_hl(v),
            Index::Ix => self.regs.ix = v,
            Index::Iy => self.regs.iy = v,
        }
    }

    /// Resolve the address for a `(HL)`/`(IX+d)`/`(IY+d)` memory operand. For the
    /// indexed forms this fetches the displacement byte and accounts the 5T
    /// internal add, and updates MEMPTR.
    pub(crate) fn ptr_addr<B: Bus>(&mut self, bus: &mut B, im: Index) -> u16 {
        match im {
            Index::Hl => self.regs.hl(),
            Index::Ix | Index::Iy => {
                let base = self.hl_index(im);
                let d = self.imm8(bus) as i8 as i16 as u16;
                let addr = base.wrapping_add(d);
                self.regs.wz = addr;
                bus.tick(5);
                addr
            }
        }
    }

    // --- Flag helpers --------------------------------------------------------

    #[inline]
    pub(crate) fn flag(&self, m: u8) -> bool {
        self.regs.f & m != 0
    }

    #[inline]
    pub(crate) fn set_flag(&mut self, m: u8, on: bool) {
        if on {
            self.regs.f |= m;
        } else {
            self.regs.f &= !m;
        }
    }
}

/// Standard reset/power-on register state for a Z80 (PC=0, SP/AF/regs = 0xFFFF
/// on real silicon; we zero everything except what the ROM relies on). Kept
/// simple: the ROM sets up its own world from PC=0.
impl Cpu {
    pub fn reset(&mut self) {
        self.regs = Regs::default();
        self.iff1 = false;
        self.iff2 = false;
        self.im = 0;
        self.halted = false;
        self.q = 0;
        self.q_prev = 0;
        self.ei_pending = false;
        // AF and SP come up as 0xFFFF on a real Z80.
        self.regs.set_af(0xFFFF);
        self.regs.sp = 0xFFFF;
    }
}
