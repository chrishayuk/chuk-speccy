//! Naive Z80 codegen (Stage 0). `HL` is the working accumulator, `DE` secondary;
//! locals live in a fixed RAM scratch region (the "virtual register file") and
//! expressions evaluate via the stack. Correct first — peephole/strength-reduce
//! come in Stage 2.

use crate::ir::*;

/// Locals: slot `i` lives at `SCRATCH + i*2` (`u16` each).
const SCRATCH: u16 = 0x9000;

/// A tiny assembler with forward-reference label patching.
struct Asm {
    org: u16,
    code: Vec<u8>,
    labels: Vec<Option<u16>>,
    fixups: Vec<(usize, usize)>, // (operand position in `code`, label id)
}

impl Asm {
    fn new(org: u16) -> Self {
        Asm { org, code: Vec::new(), labels: Vec::new(), fixups: Vec::new() }
    }
    fn here(&self) -> u16 {
        self.org.wrapping_add(self.code.len() as u16)
    }
    fn byte(&mut self, b: u8) {
        self.code.push(b);
    }
    fn word(&mut self, w: u16) {
        self.code.push(w as u8);
        self.code.push((w >> 8) as u8);
    }
    fn label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }
    fn place(&mut self, l: usize) {
        let here = self.here();
        self.labels[l] = Some(here);
    }
    /// Emit `opcode <addr:label>` (e.g. `JP`, `JP cc`).
    fn jump(&mut self, opcode: u8, l: usize) {
        self.byte(opcode);
        self.fixups.push((self.code.len(), l));
        self.word(0); // placeholder
    }
    fn finish(mut self) -> Vec<u8> {
        for (pos, l) in &self.fixups {
            let addr = self.labels[*l].expect("unplaced label");
            self.code[*pos] = addr as u8;
            self.code[*pos + 1] = (addr >> 8) as u8;
        }
        self.code
    }
}

pub fn codegen(f: &Func, org: u16) -> Vec<u8> {
    let mut a = Asm::new(org);
    for s in &f.body {
        gen_stmt(&mut a, s);
    }
    if let Some(e) = &f.ret {
        gen_expr(&mut a, e); // result in HL
    }
    a.byte(0xC9); // RET
    a.finish()
}

fn slot_addr(slot: usize) -> u16 {
    SCRATCH + (slot as u16) * 2
}

/// Evaluate `e`, leaving the result in `HL`.
fn gen_expr(a: &mut Asm, e: &Expr) {
    match e {
        Expr::Lit(n) => {
            a.byte(0x21); // LD HL, nn
            a.word(*n);
        }
        Expr::Var(slot) => {
            a.byte(0x2A); // LD HL, (addr)
            a.word(slot_addr(*slot));
        }
        Expr::Bin(BinOp::Add, l, r) => {
            gen_expr(a, l);
            a.byte(0xE5); // PUSH HL
            gen_expr(a, r);
            a.byte(0xD1); // POP DE   (DE = l)
            a.byte(0x19); // ADD HL, DE  (HL = r + l)
        }
        Expr::Bin(BinOp::Sub, l, r) => {
            gen_sub(a, l, r); // HL = l - r
        }
    }
}

/// `HL = left - right`, with flags from the subtraction (carry = borrow).
fn gen_sub(a: &mut Asm, left: &Expr, right: &Expr) {
    gen_expr(a, right);
    a.byte(0xE5); // PUSH HL  (right)
    gen_expr(a, left);
    a.byte(0xD1); // POP DE   (DE = right)
    a.byte(0xB7); // OR A     (clear carry)
    a.byte(0xED);
    a.byte(0x52); // SBC HL, DE  (HL = left - right)
}

fn gen_stmt(a: &mut Asm, s: &Stmt) {
    match s {
        Stmt::Assign(slot, e) => {
            gen_expr(a, e);
            a.byte(0x22); // LD (addr), HL
            a.word(slot_addr(*slot));
        }
        Stmt::If(cond, then, els) => {
            let else_l = a.label();
            let end_l = a.label();
            gen_cond_skip(a, cond, else_l); // jump to else when cond is false
            for s in then {
                gen_stmt(a, s);
            }
            a.jump(0xC3, end_l); // JP end
            a.place(else_l);
            for s in els {
                gen_stmt(a, s);
            }
            a.place(end_l);
        }
        Stmt::While(cond, body) => {
            let top = a.label();
            let end = a.label();
            a.place(top);
            gen_cond_skip(a, cond, end); // exit loop when cond is false
            for s in body {
                gen_stmt(a, s);
            }
            a.jump(0xC3, top); // JP top
            a.place(end);
        }
    }
}

/// Emit a comparison and a conditional jump to `target` taken when the condition
/// is **false** (used to skip an `if`/`while` body).
fn gen_cond_skip(a: &mut Asm, cond: &Cond, target: usize) {
    // Pick the subtraction operands and the "false" jump opcode per comparison.
    // After `SBC HL,DE`: carry = (left < right), zero = (left == right).
    const JP_NC: u8 = 0xD2;
    const JP_C: u8 = 0xDA;
    const JP_NZ: u8 = 0xC2;
    const JP_Z: u8 = 0xCA;
    let (left, right, jp_false) = match cond.cmp {
        Cmp::Lt => (&cond.lhs, &cond.rhs, JP_NC), // a<b true on carry → skip if NC
        Cmp::Ge => (&cond.lhs, &cond.rhs, JP_C),
        Cmp::Eq => (&cond.lhs, &cond.rhs, JP_NZ),
        Cmp::Ne => (&cond.lhs, &cond.rhs, JP_Z),
        Cmp::Gt => (&cond.rhs, &cond.lhs, JP_NC), // a>b ≡ b<a
        Cmp::Le => (&cond.rhs, &cond.lhs, JP_C),  // a<=b ≡ !(b<a)
    };
    gen_sub(a, left, right);
    a.jump(jp_false, target);
}
