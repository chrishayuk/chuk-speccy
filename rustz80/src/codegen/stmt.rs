//! Statement codegen — effects (`gen_stmt`), returns, fills, and condition branches.
use super::asm::*;
use super::expr::*;
use super::runtime::*;
use super::Target;
use crate::ir::*;

/// Emit a function's return values into the result convention `HL`/`DE`/`BC`: none
/// for a void fn, `HL` for a scalar, two/three registers for a tuple. Each value is
/// pushed, then popped into its register in reverse so the first lands in `HL`.
pub(super) fn gen_return(a: &mut Asm, rets: &[Expr]) {
    match rets.len() {
        0 => {}
        1 => gen_expr(a, &rets[0]),
        n => {
            for e in rets {
                gen_expr(a, e);
                a.byte(0xE5); // PUSH HL
            }
            const POP: [u8; 3] = [0xE1, 0xD1, 0xC1]; // HL, DE, BC
            for i in (0..n).rev() {
                a.byte(POP[i]);
            }
        }
    }
}

/// Fill `count` slots at local `base` with `value`. Every array element is one 2-byte
/// slot (a `u8` lives in the low byte, `H = 0`), so the fill is always slot-stride.
/// Spectrum: store the first slot then `LDIR` (compact, fast — beats `N` unrolled stores).
/// Cell: an `ED FE` fill trap (host-native).
pub(super) fn gen_fill(a: &mut Asm, base: usize, count: usize, value: &Expr) {
    if count == 0 {
        return;
    }
    let addr = slot_addr(a.base, base);
    match a.target {
        Target::Spectrum48 => {
            gen_expr(a, value); // HL = value
            a.byte(0x22); // LD (addr),HL    (first slot)
            a.word(addr);
            if count >= 2 {
                a.byte(0x21); // LD HL, addr        (src)
                a.word(addr);
                a.byte(0x11); // LD DE, addr+2      (dst)
                a.word(addr.wrapping_add(2));
                a.byte(0x01); // LD BC, (count-1)*2
                a.word((count as u16 - 1) * 2);
                a.byte(0xED);
                a.byte(0xB0); // LDIR  (propagates the slot forward)
            }
        }
        Target::Cell => {
            gen_expr(a, value);
            a.byte(0xE5); // PUSH HL  (value)
            a.byte(0x21); // LD HL, addr   (base)
            a.word(addr);
            a.byte(0x01); // LD BC, count  (slots)
            a.word(count as u16);
            a.byte(0xD1); // POP DE   (value)
            gen_trap(a, TRAP_FILL16);
        }
    }
}

pub(super) fn gen_stmt(a: &mut Asm, s: &Stmt) {
    match s {
        Stmt::Assign(slot, e) => {
            gen_expr(a, e);
            a.byte(0x22); // LD (addr), HL
            let addr = slot_addr(a.base, *slot);
            a.word(addr);
        }
        Stmt::StoreIndex(base, index, value, w) => {
            gen_expr(a, value);
            a.byte(0xE5); // PUSH HL  (value)
            gen_elem_addr(a, *base, index); // HL = &base[index]
            a.byte(0xD1); // POP DE   (DE = value)
            a.byte(0x73); // LD (HL),E   (low byte)
            if *w == Width::Word {
                a.byte(0x23); // INC HL
                a.byte(0x72); // LD (HL),D   (high byte)
            }
        }
        Stmt::Poke(addr, value) => {
            gen_expr(a, value);
            a.byte(0xE5); // PUSH HL  (value)
            gen_expr(a, addr); // HL = addr
            a.byte(0xD1); // POP DE   (DE = value)
            a.byte(0x73); // LD (HL),E   (store low byte)
        }
        Stmt::Store(ptr, off, value) => {
            gen_expr(a, value);
            a.byte(0xE5); // PUSH HL  (value)
            gen_expr(a, ptr); // HL = base pointer
            gen_add_offset(a, *off); // HL = &field
            a.byte(0xD1); // POP DE   (DE = value)
            a.byte(0x73); // LD (HL),E
            a.byte(0x23); // INC HL
            a.byte(0x72); // LD (HL),D
        }
        Stmt::PtrStoreIndex {
            ptr,
            off,
            index,
            value,
        } => {
            gen_expr(a, value);
            a.byte(0xE5); // PUSH HL  (value)
            gen_ptr_elem_addr(a, ptr, *off, index); // HL = &arr[index] (balanced push/pop)
            a.byte(0xD1); // POP DE   (DE = value)
            a.byte(0x73); // LD (HL),E
            a.byte(0x23); // INC HL
            a.byte(0x72); // LD (HL),D
        }
        Stmt::StoreAt(addr, value, w) => {
            gen_expr(a, value);
            a.byte(0xE5); // PUSH HL  (value)
            gen_expr(a, addr); // HL = byte address
            a.byte(0xD1); // POP DE   (DE = value)
            a.byte(0x73); // LD (HL),E   (low byte)
            if *w == Width::Word {
                a.byte(0x23); // INC HL
                a.byte(0x72); // LD (HL),D   (high byte)
            }
        }
        Stmt::Assign32(slot, e) => {
            gen_expr32(a, e); // HL = low word, DE = high word
            let addr = slot_addr(a.base, *slot);
            a.byte(0x22); // LD (addr),HL     low word
            a.word(addr);
            a.byte(0xED);
            a.byte(0x53); // LD (addr+2),DE   high word
            a.word(addr.wrapping_add(2));
        }
        Stmt::Fill { base, count, value } => gen_fill(a, *base, *count, value),
        Stmt::Eval(e) => {
            gen_expr(a, e); // result left in HL, discarded
        }
        Stmt::AssignTuple(slots, call) => {
            gen_expr(a, call); // leaves the returned tuple in HL/DE/BC
                               // Store each register to its slot — `LD (nn),HL/DE/BC` don't clobber
                               // the other registers, so order is free.
            const ST: [&[u8]; 3] = [&[0x22], &[0xED, 0x53], &[0xED, 0x43]];
            for (i, slot) in slots.iter().enumerate() {
                for &b in ST[i] {
                    a.byte(b);
                }
                a.word(slot_addr(a.base, *slot));
            }
        }
        Stmt::If(cond, then, els) => {
            let else_l = a.label();
            let end_l = a.label();
            gen_cond_skip(a, cond, else_l);
            for s in then {
                gen_stmt(a, s);
            }
            a.jump(0xC3, end_l);
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
            gen_cond_skip(a, cond, end);
            // `continue` re-checks the condition (top); `break` exits (end).
            a.loop_stack.push((top, end));
            for s in body {
                gen_stmt(a, s);
            }
            a.loop_stack.pop();
            a.jump(0xC3, top);
            a.place(end);
        }
        Stmt::Loop(body) => {
            let top = a.label();
            let end = a.label();
            a.place(top);
            a.loop_stack.push((top, end)); // continue -> top, break -> end
            for s in body {
                gen_stmt(a, s);
            }
            a.loop_stack.pop();
            a.jump(0xC3, top);
            a.place(end);
        }
        Stmt::ForRange {
            var,
            end,
            inclusive,
            width,
            body,
        } => {
            let top = a.label();
            let cont = a.label();
            let brk = a.label();
            a.place(top);
            // Skip to `brk` once the bound is reached (`var < end`, or `<=`).
            let cond = Cond {
                cmp: if *inclusive { Cmp::Le } else { Cmp::Lt },
                lhs: Expr::Var(*var),
                rhs: end.clone(),
            };
            gen_cond_skip(a, &cond, brk);
            // `continue` lands on the step (`cont`); `break` exits (`brk`).
            a.loop_stack.push((cont, brk));
            for s in body {
                gen_stmt(a, s);
            }
            a.loop_stack.pop();
            a.place(cont);
            // Induction step: `var = var + 1` (masked to the loop var's width).
            gen_stmt(
                a,
                &Stmt::Assign(
                    *var,
                    Expr::Bin(
                        BinOp::Add,
                        Box::new(Expr::Var(*var)),
                        Box::new(Expr::Lit(1)),
                        *width,
                    ),
                ),
            );
            a.jump(0xC3, top);
            a.place(brk);
        }
        Stmt::Break => {
            let (_, brk) = *a.loop_stack.last().expect("`break` outside a loop");
            a.jump(0xC3, brk);
        }
        Stmt::Continue => {
            let (cont, _) = *a.loop_stack.last().expect("`continue` outside a loop");
            a.jump(0xC3, cont);
        }
        Stmt::Return(val) => {
            if let Some(e) = val {
                gen_expr(a, e); // result in HL
            }
            let end = a.func_end.expect("`return` outside a function");
            a.jump(0xC3, end);
        }
    }
}

/// Emit a comparison and a conditional jump to `target`, taken when the condition
/// is **false** (used to skip an `if`/`while` body).
pub(super) fn gen_cond_skip(a: &mut Asm, cond: &Cond, target: usize) {
    const JP_NC: u8 = 0xD2;
    const JP_C: u8 = 0xDA;
    const JP_NZ: u8 = 0xC2;
    const JP_Z: u8 = 0xCA;
    // After `SBC HL,DE`: carry = (left < right), zero = (left == right).
    let (left, right, jp_false) = match cond.cmp {
        Cmp::Lt => (&cond.lhs, &cond.rhs, JP_NC),
        Cmp::Ge => (&cond.lhs, &cond.rhs, JP_C),
        Cmp::Eq => (&cond.lhs, &cond.rhs, JP_NZ),
        Cmp::Ne => (&cond.lhs, &cond.rhs, JP_Z),
        Cmp::Gt => (&cond.rhs, &cond.lhs, JP_NC), // a>b ≡ b<a
        Cmp::Le => (&cond.rhs, &cond.lhs, JP_C),  // a<=b ≡ !(b<a)
    };
    gen_sub(a, left, right);
    a.jump(jp_false, target);
}
