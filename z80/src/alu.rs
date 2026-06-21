//! ALU: the flag-setting arithmetic, logic, rotate/shift, bit, and misc
//! operations, as methods on [`Cpu`]. Flag rules follow the documented Z80
//! behaviour; XF/YF (bits 5/3) are taken from the result for the common ops
//! (correct for ADD/SUB/INC/...) — the operand-sourced XF/YF edge cases for
//! `CP`, block ops, and `IN` are handled at their call sites. See
//! `docs/01-core-emulator-spec.md` §4.3.

use crate::cpu::Cpu;
use crate::flags::*;

impl Cpu {
    // --- 8-bit add / sub family ---------------------------------------------

    pub(crate) fn add_a(&mut self, val: u8, carry: bool) {
        let a = self.regs.a;
        let c = carry as u16;
        let r = a as u16 + val as u16 + c;
        let res = r as u8;
        let mut f = res & (SF | YF | XF);
        if res == 0 {
            f |= ZF;
        }
        if (a & 0x0f) + (val & 0x0f) + c as u8 > 0x0f {
            f |= HF;
        }
        if (a ^ val) & 0x80 == 0 && (a ^ res) & 0x80 != 0 {
            f |= PF;
        }
        if r & 0x100 != 0 {
            f |= CF;
        }
        self.regs.a = res;
        self.regs.f = f;
        self.q = self.regs.f;
    }

    /// SUB/SBC/CP. When `store` is false this is CP: the result is discarded and
    /// XF/YF come from the *operand*, not the result.
    pub(crate) fn sub_a(&mut self, val: u8, carry: bool, store: bool) {
        let a = self.regs.a;
        let c = carry as u16;
        let r = (a as u16).wrapping_sub(val as u16).wrapping_sub(c);
        let res = r as u8;
        let mut f = NF;
        if store {
            f |= res & (SF | YF | XF);
        } else {
            // CP: undocumented bits from the operand.
            f |= (res & SF) | (val & (YF | XF));
        }
        if res == 0 {
            f |= ZF;
        }
        if (a & 0x0f).wrapping_sub(val & 0x0f).wrapping_sub(c as u8) & 0x10 != 0 {
            f |= HF;
        }
        if (a ^ val) & 0x80 != 0 && (a ^ res) & 0x80 != 0 {
            f |= PF;
        }
        if r & 0x100 != 0 {
            f |= CF;
        }
        if store {
            self.regs.a = res;
        }
        self.regs.f = f;
        self.q = self.regs.f;
    }

    pub(crate) fn and_a(&mut self, val: u8) {
        let res = self.regs.a & val;
        self.regs.a = res;
        self.regs.f = SZ53P[res as usize] | HF;
        self.q = self.regs.f;
    }

    pub(crate) fn xor_a(&mut self, val: u8) {
        let res = self.regs.a ^ val;
        self.regs.a = res;
        self.regs.f = SZ53P[res as usize];
        self.q = self.regs.f;
    }

    pub(crate) fn or_a(&mut self, val: u8) {
        let res = self.regs.a | val;
        self.regs.a = res;
        self.regs.f = SZ53P[res as usize];
        self.q = self.regs.f;
    }

    /// Dispatch the 8-entry ALU table `[add,adc,sub,sbc,and,xor,or,cp]`.
    pub(crate) fn alu(&mut self, op: u8, val: u8) {
        match op {
            0 => self.add_a(val, false),
            1 => self.add_a(val, self.flag(CF)),
            2 => self.sub_a(val, false, true),
            3 => self.sub_a(val, self.flag(CF), true),
            4 => self.and_a(val),
            5 => self.xor_a(val),
            6 => self.or_a(val),
            7 => self.sub_a(val, false, false), // CP
            _ => unreachable!(),
        }
    }

    // --- INC / DEC 8-bit (carry preserved) ----------------------------------

    pub(crate) fn inc8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        let mut f = (self.regs.f & CF) | (r & (SF | YF | XF));
        if r == 0 {
            f |= ZF;
        }
        if r & 0x0f == 0 {
            f |= HF;
        }
        if r == 0x80 {
            f |= PF;
        }
        self.regs.f = f;
        self.q = self.regs.f;
        r
    }

    pub(crate) fn dec8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        let mut f = (self.regs.f & CF) | NF | (r & (SF | YF | XF));
        if r == 0 {
            f |= ZF;
        }
        if r & 0x0f == 0x0f {
            f |= HF;
        }
        if r == 0x7f {
            f |= PF;
        }
        self.regs.f = f;
        self.q = self.regs.f;
        r
    }

    // --- 16-bit add family --------------------------------------------------

    /// ADD HL,rp (or IX/IY,rp). S/Z/PV preserved; H/C/Y/X from the result.
    pub(crate) fn add16(&mut self, a: u16, b: u16) -> u16 {
        let r = a as u32 + b as u32;
        let res = r as u16;
        let mut f = self.regs.f & (SF | ZF | PF);
        f |= ((res >> 8) as u8) & (YF | XF);
        if (a & 0x0fff) + (b & 0x0fff) > 0x0fff {
            f |= HF;
        }
        if r & 0x1_0000 != 0 {
            f |= CF;
        }
        self.regs.f = f;
        self.regs.wz = a.wrapping_add(1);
        self.q = self.regs.f;
        res
    }

    /// ADC HL,rp — full flags, stores into HL.
    pub(crate) fn adc16(&mut self, b: u16) {
        let a = self.regs.hl();
        let c = (self.flag(CF)) as u32;
        let r = a as u32 + b as u32 + c;
        let res = r as u16;
        let mut f = ((res >> 8) as u8) & (SF | YF | XF);
        if res == 0 {
            f |= ZF;
        }
        if (a & 0x0fff) + (b & 0x0fff) + c as u16 > 0x0fff {
            f |= HF;
        }
        if (a ^ b) & 0x8000 == 0 && (a ^ res) & 0x8000 != 0 {
            f |= PF;
        }
        if r & 0x1_0000 != 0 {
            f |= CF;
        }
        self.regs.set_hl(res);
        self.regs.f = f;
        self.regs.wz = a.wrapping_add(1);
        self.q = self.regs.f;
    }

    /// SBC HL,rp — full flags, stores into HL.
    pub(crate) fn sbc16(&mut self, b: u16) {
        let a = self.regs.hl();
        let c = (self.flag(CF)) as u32;
        let r = (a as u32).wrapping_sub(b as u32).wrapping_sub(c);
        let res = r as u16;
        let mut f = NF | (((res >> 8) as u8) & (SF | YF | XF));
        if res == 0 {
            f |= ZF;
        }
        if (a & 0x0fff)
            .wrapping_sub(b & 0x0fff)
            .wrapping_sub(c as u16)
            & 0x1000
            != 0
        {
            f |= HF;
        }
        if (a ^ b) & 0x8000 != 0 && (a ^ res) & 0x8000 != 0 {
            f |= PF;
        }
        if r & 0x1_0000 != 0 {
            f |= CF;
        }
        self.regs.set_hl(res);
        self.regs.f = f;
        self.regs.wz = a.wrapping_add(1);
        self.q = self.regs.f;
    }

    // --- Accumulator rotates (RLCA/RRCA/RLA/RRA) ----------------------------
    // S/Z/PV preserved; H=N=0; C from the rotated-out bit; Y/X from result.

    pub(crate) fn rlca(&mut self) {
        let a = self.regs.a;
        let carry = a & 0x80 != 0;
        let res = a.rotate_left(1);
        self.acc_rotate_flags(res, carry);
    }

    pub(crate) fn rrca(&mut self) {
        let a = self.regs.a;
        let carry = a & 0x01 != 0;
        let res = a.rotate_right(1);
        self.acc_rotate_flags(res, carry);
    }

    pub(crate) fn rla(&mut self) {
        let a = self.regs.a;
        let carry = a & 0x80 != 0;
        let res = (a << 1) | self.flag(CF) as u8;
        self.acc_rotate_flags(res, carry);
    }

    pub(crate) fn rra(&mut self) {
        let a = self.regs.a;
        let carry = a & 0x01 != 0;
        let res = (a >> 1) | ((self.flag(CF) as u8) << 7);
        self.acc_rotate_flags(res, carry);
    }

    fn acc_rotate_flags(&mut self, res: u8, carry: bool) {
        let mut f = self.regs.f & (SF | ZF | PF);
        f |= res & (YF | XF);
        if carry {
            f |= CF;
        }
        self.regs.a = res;
        self.regs.f = f;
        self.q = self.regs.f;
    }

    // --- CB rotates/shifts (full SZ53P + carry) -----------------------------

    /// `kind`: 0=RLC 1=RRC 2=RL 3=RR 4=SLA 5=SRA 6=SLL(undoc) 7=SRL.
    pub(crate) fn cb_rot(&mut self, kind: u8, v: u8) -> u8 {
        let (res, carry) = match kind {
            0 => (v.rotate_left(1), v & 0x80 != 0),
            1 => (v.rotate_right(1), v & 0x01 != 0),
            2 => ((v << 1) | self.flag(CF) as u8, v & 0x80 != 0),
            3 => ((v >> 1) | ((self.flag(CF) as u8) << 7), v & 0x01 != 0),
            4 => (v << 1, v & 0x80 != 0),
            5 => ((v >> 1) | (v & 0x80), v & 0x01 != 0),
            6 => ((v << 1) | 1, v & 0x80 != 0), // SLL: shifts in a 1 (undocumented)
            7 => (v >> 1, v & 0x01 != 0),
            _ => unreachable!(),
        };
        let mut f = SZ53P[res as usize];
        if carry {
            f |= CF;
        }
        self.regs.f = f;
        self.q = self.regs.f;
        res
    }

    /// BIT n,r. `xf_yf_src` supplies the undocumented bits 5/3 (the register
    /// value for `BIT n,r`, or MEMPTR-high for `BIT n,(HL)`).
    pub(crate) fn bit(&mut self, n: u8, v: u8, xf_yf_src: u8) {
        let bit_set = v & (1 << n) != 0;
        let mut f = (self.regs.f & CF) | HF;
        if !bit_set {
            f |= ZF | PF;
        }
        if n == 7 && bit_set {
            f |= SF;
        }
        f |= xf_yf_src & (YF | XF);
        self.regs.f = f;
        self.q = self.regs.f;
    }

    // --- Misc accumulator ops -----------------------------------------------

    pub(crate) fn daa(&mut self) {
        let a = self.regs.a;
        let n = self.flag(NF);
        let mut correction = 0u8;
        let mut carry = self.flag(CF);
        if self.flag(HF) || (a & 0x0f) > 9 {
            correction |= 0x06;
        }
        if carry || a > 0x99 {
            correction |= 0x60;
            carry = true;
        }
        let res = if n {
            a.wrapping_sub(correction)
        } else {
            a.wrapping_add(correction)
        };
        let mut f = SZ53P[res as usize];
        if n {
            f |= NF;
            if self.flag(HF) && (a & 0x0f) < 6 {
                f |= HF;
            }
        } else if (a & 0x0f) > 9 {
            f |= HF;
        }
        if carry {
            f |= CF;
        }
        self.regs.a = res;
        self.regs.f = f;
        self.q = self.regs.f;
    }

    pub(crate) fn cpl(&mut self) {
        self.regs.a = !self.regs.a;
        let f = (self.regs.f & (SF | ZF | PF | CF)) | HF | NF | (self.regs.a & (YF | XF));
        self.regs.f = f;
        self.q = self.regs.f;
    }

    pub(crate) fn neg(&mut self) {
        let a = self.regs.a;
        self.regs.a = 0;
        self.sub_a(a, false, true);
    }

    pub(crate) fn scf(&mut self) {
        // CF=1, H=N=0. The undocumented YF/XF follow the Q-quirk: they come from
        // A if the previous instruction touched F, else from (A | F). Both cases
        // collapse to `((q_prev ^ F) | A)` — see docs §4.3.
        let f = (self.regs.f & (SF | ZF | PF)) | CF | (self.scf_ccf_xy());
        self.regs.f = f;
        self.q = f;
    }

    pub(crate) fn ccf(&mut self) {
        let old_c = self.flag(CF);
        let xy = self.scf_ccf_xy();
        let mut f = self.regs.f & (SF | ZF | PF);
        if old_c {
            f |= HF; // H = old carry
        }
        if !old_c {
            f |= CF;
        }
        f |= xy;
        self.regs.f = f;
        self.q = f;
    }

    /// The Q-quirk YF/XF contribution for SCF/CCF.
    #[inline]
    fn scf_ccf_xy(&self) -> u8 {
        ((self.q_prev ^ self.regs.f) | self.regs.a) & (YF | XF)
    }
}
