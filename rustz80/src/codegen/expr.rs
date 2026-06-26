//! Expression codegen — evaluate an `Expr` into `HL` (arithmetic, bitwise, traps, u32).
use super::asm::*;
use super::runtime::*;
use super::Target;
use crate::ir::*;

/// Evaluate `e`, leaving the result in `HL`.
pub(super) fn gen_expr(a: &mut Asm, e: &Expr) {
    match e {
        Expr::Lit(n) => {
            a.byte(0x21);
            a.word(*n);
        }
        Expr::Var(slot) => {
            a.byte(0x2A);
            let addr = slot_addr(a.base, *slot);
            a.word(addr);
        }
        Expr::Bin(op, l, r, w) => {
            // Const-fold a literal-only op (e.g. `2 * 3 + 4`).
            if let (Expr::Lit(x), Expr::Lit(y)) = (&**l, &**r) {
                if let Some(v) = const_fold(*op, *x, *y) {
                    a.byte(0x21); // LD HL, v
                    a.word(v);
                    mask_to_width(a, *w);
                    return;
                }
            }
            match op {
                BinOp::Add => {
                    gen_expr(a, l);
                    a.byte(0xE5); // PUSH HL
                    gen_expr(a, r);
                    a.byte(0xD1); // POP DE  (DE = l)
                    a.byte(0x19); // ADD HL, DE
                }
                BinOp::Sub => gen_sub(a, l, r),
                // `x * const` → shift-and-add (no `__mul16`); else the runtime/trap.
                BinOp::Mul => match const_operand(l, r) {
                    Some((k, other)) => {
                        gen_expr(a, other);
                        gen_mul_const(a, k);
                    }
                    None => gen_mul(a, l, r),
                },
                // `x / 2^n` → shift right; else the runtime/trap.
                BinOp::Div => match lit_val(r) {
                    Some(k) if k.is_power_of_two() => {
                        gen_expr(a, l);
                        gen_shr_const(a, k.trailing_zeros());
                    }
                    _ => gen_divmod(a, l, r, false),
                },
                // `x % 2^n` → mask the low bits; else the runtime/trap.
                BinOp::Rem => match lit_val(r) {
                    Some(k) if k.is_power_of_two() => {
                        gen_expr(a, l);
                        gen_and_mask(a, k - 1);
                    }
                    _ => gen_divmod(a, l, r, true),
                },
                BinOp::Or => gen_bitwise(a, l, r, 0xB3, 0xB2), // OR E / OR D
                BinOp::And => gen_bitwise(a, l, r, 0xA3, 0xA2), // AND E / AND D
                BinOp::Xor => gen_bitwise(a, l, r, 0xAB, 0xAA), // XOR E / XOR D
                // Shift by a constant amount (RHS is always a literal).
                BinOp::Shl => {
                    gen_expr(a, l);
                    for _ in 0..lit_u8(r) {
                        a.byte(0x29); // ADD HL,HL  (logical << 1)
                    }
                }
                BinOp::Shr => {
                    gen_expr(a, l);
                    gen_shr_const(a, lit_u8(r) as u32);
                }
            }
            mask_to_width(a, *w);
        }
        Expr::Index(base, index, w) => {
            gen_elem_addr(a, *base, index); // HL = &base[index]
            match w {
                Width::Word => {
                    a.byte(0x5E); // LD E,(HL)
                    a.byte(0x23); // INC HL
                    a.byte(0x56); // LD D,(HL)
                    a.byte(0xEB); // EX DE,HL   -> HL = value
                }
                Width::Byte => {
                    a.byte(0x6E); // LD L,(HL)
                    a.byte(0x26); // LD H, 0    -> HL = zero-extended byte
                    a.byte(0x00);
                }
                Width::DWord => unreachable!("u32 array elements are unsupported"),
            }
        }
        Expr::Call(name, args) => {
            for arg in args {
                gen_expr(a, arg);
                a.byte(0xE5); // PUSH HL
            }
            const POP: [u8; 3] = [0xE1, 0xD1, 0xC1]; // HL, DE, BC
            for i in (0..args.len()).rev() {
                a.byte(POP[i]);
            }
            a.call(name);
        }
        Expr::Trunc(e) => {
            gen_expr(a, e);
            a.byte(0x26); // LD H, 0   (mask to u8)
            a.byte(0x00);
        }
        Expr::Peek(addr) => {
            gen_expr(a, addr); // HL = addr
            a.byte(0x6E); // LD L,(HL)   -- read mem[addr] into L
            a.byte(0x26); // LD H, 0     -> HL = zero-extended byte
            a.byte(0x00);
        }
        Expr::InPort(port) => {
            gen_expr(a, port); // HL = port
            a.byte(0x44); // LD B,H
            a.byte(0x4D); // LD C,L   (BC = port)
            a.byte(0xED);
            a.byte(0x78); // IN A,(C)
            a.byte(0x6F); // LD L,A
            a.byte(0x26); // LD H,0   -> HL = port byte
            a.byte(0x00);
        }
        Expr::AddrOf(slot) => {
            a.byte(0x21); // LD HL, &local
            let addr = slot_addr(a.base, *slot);
            a.word(addr);
        }
        Expr::Deref(ptr, off) => {
            gen_expr(a, ptr); // HL = base pointer
            gen_add_offset(a, *off);
            a.byte(0x5E); // LD E,(HL)
            a.byte(0x23); // INC HL
            a.byte(0x56); // LD D,(HL)
            a.byte(0xEB); // EX DE,HL   -> HL = u16 at *(ptr + off)
        }
        Expr::PtrIndex { ptr, off, index } => {
            gen_ptr_elem_addr(a, ptr, *off, index); // HL = ptr + off + index*2
            a.byte(0x5E); // LD E,(HL)
            a.byte(0x23); // INC HL
            a.byte(0x56); // LD D,(HL)
            a.byte(0xEB); // EX DE,HL   -> HL = u16 element
        }
        Expr::MulConst(e, k) => {
            gen_expr(a, e);
            gen_mul_const(a, *k);
        }
        Expr::LoadAt(addr, w) => {
            gen_expr(a, addr); // HL = byte address
            match w {
                Width::Word => {
                    a.byte(0x5E); // LD E,(HL)
                    a.byte(0x23); // INC HL
                    a.byte(0x56); // LD D,(HL)
                    a.byte(0xEB); // EX DE,HL
                }
                Width::Byte => {
                    a.byte(0x6E); // LD L,(HL)
                    a.byte(0x26); // LD H, 0  (zero-extend)
                    a.byte(0x00);
                }
                Width::DWord => unreachable!("u32 array/field elements are unsupported"),
            }
        }
        // `x as u16` — the low word of a `u32` value (the high word is discarded).
        Expr::Trunc32(e) => gen_expr32(a, e),
        // `halt(code)` — code in HL, then the HALT trap (no-op on real hardware).
        Expr::Halt(code) => {
            gen_expr(a, code);
            gen_trap(a, TRAP_HALT);
        }
        Expr::Lit32(_) | Expr::Var32(_) | Expr::Bin32(..) | Expr::Shift32 { .. } => {
            unreachable!("u32 node used in a 16-bit context (u32 params/returns unsupported)")
        }
    }
}

/// The first literal operand as a `u8` shift amount (the lowering guarantees a literal).
pub(super) fn lit_u8(e: &Expr) -> u8 {
    match e {
        Expr::Lit(k) => *k as u8,
        _ => unreachable!("shift amount must be a constant"),
    }
}

/// The literal value of `e`, if it is one.
pub(super) fn lit_val(e: &Expr) -> Option<u16> {
    match e {
        Expr::Lit(k) => Some(*k),
        _ => None,
    }
}

/// For a commutative op, a literal operand `(k, other)` if exactly one side is a literal.
pub(super) fn const_operand<'a>(l: &'a Expr, r: &'a Expr) -> Option<(u16, &'a Expr)> {
    match (lit_val(l), lit_val(r)) {
        (Some(k), None) => Some((k, r)),
        (None, Some(k)) => Some((k, l)),
        _ => None, // both-literal is const-folded; neither falls through
    }
}

/// Fold a literal-only binary op at compile time (`None` = leave to the runtime: a
/// `Div`/`Rem` by zero, or a shift, which the normal path handles).
pub(super) fn const_fold(op: BinOp, x: u16, y: u16) -> Option<u16> {
    Some(match op {
        BinOp::Add => x.wrapping_add(y),
        BinOp::Sub => x.wrapping_sub(y),
        BinOp::Mul => x.wrapping_mul(y),
        BinOp::And => x & y,
        BinOp::Or => x | y,
        BinOp::Xor => x ^ y,
        BinOp::Div if y != 0 => x / y,
        BinOp::Rem if y != 0 => x % y,
        _ => return None,
    })
}

/// `HL = l * r` (full 16-bit, neither operand constant). Spectrum: the software runtime.
/// Cell: an `ED FE` host trap, serviced natively by the cell bus.
pub(super) fn gen_mul(a: &mut Asm, l: &Expr, r: &Expr) {
    // `x * x` (one variable squared) — load it once and fan it out to the operand
    // registers, instead of evaluating + reloading the operand twice. Restricted to a bare
    // `Var` so it stays side-effect-free (`f() * f()` must still evaluate twice).
    let square = matches!((l, r), (Expr::Var(s1), Expr::Var(s2)) if s1 == s2);
    match a.target {
        Target::Spectrum48 => {
            if square {
                gen_expr(a, l); // HL = x
                a.byte(0x54);
                a.byte(0x5D); // ld d,h ; ld e,l   (DE = x)
            } else {
                gen_pair(a, l, r); // HL = r, DE = l
            }
            a.call("__mul16"); // HL = HL * DE
            a.needs_mul = true;
        }
        Target::Cell => {
            if square {
                gen_expr(a, l); // HL = x
                a.byte(0x54);
                a.byte(0x5D); // ld d,h ; ld e,l   (DE = x)
            } else {
                gen_expr(a, l);
                a.byte(0xE5); // PUSH HL  (l)
                gen_expr(a, r); // HL = r
                a.byte(0xD1); // POP DE   (DE = l)
            }
            a.byte(0x44);
            a.byte(0x4D); // ld b,h ; ld c,l   (BC = the value left in HL)
            gen_trap(a, TRAP_MUL16); // HL = BC * DE
        }
    }
}

/// `HL = l / r` (or `l % r` if `rem`), neither a power of two. Spectrum: the software
/// runtime. Cell: an `ED FE` host trap.
pub(super) fn gen_divmod(a: &mut Asm, l: &Expr, r: &Expr, rem: bool) {
    match a.target {
        Target::Spectrum48 => {
            gen_pair(a, r, l); // HL = l, DE = r
            a.call("__divmod16"); // HL = l/r, DE = l%r
            a.needs_div = true;
            if rem {
                a.byte(0xEB); // EX DE,HL  -> HL = remainder
            }
        }
        Target::Cell => {
            gen_expr(a, r);
            a.byte(0xE5); // PUSH HL  (r = divisor)
            gen_expr(a, l);
            a.byte(0x44);
            a.byte(0x4D); // ld b,h ; ld c,l   (BC = l = dividend)
            a.byte(0xD1); // POP DE   (DE = r = divisor)
            gen_trap(a, TRAP_DIVMOD16); // HL = BC/DE, DE = BC%DE
            if rem {
                a.byte(0xEB); // EX DE,HL  -> HL = remainder
            }
        }
    }
}

/// `HL >>= n` (logical), as `SRL H; RR L` per step.
pub(super) fn gen_shr_const(a: &mut Asm, n: u32) {
    for _ in 0..n {
        a.byte(0xCB);
        a.byte(0x3C); // SRL H
        a.byte(0xCB);
        a.byte(0x1D); // RR L
    }
}

/// `HL &= mask` (a compile-time constant), byte-wise through the accumulator.
pub(super) fn gen_and_mask(a: &mut Asm, mask: u16) {
    a.byte(0x7D); // LD A,L
    a.byte(0xE6); // AND lo
    a.byte(mask as u8);
    a.byte(0x6F); // LD L,A
    a.byte(0x7C); // LD A,H
    a.byte(0xE6); // AND hi
    a.byte((mask >> 8) as u8);
    a.byte(0x67); // LD H,A
}

/// Wrap `HL` to a byte (`u8`) by zeroing `H`.
pub(super) fn mask_to_width(a: &mut Asm, w: Width) {
    if w == Width::Byte {
        a.byte(0x26); // LD H, 0
        a.byte(0x00);
    }
}

/// Evaluate a `u32` expression into the `HL:DE` pair (`HL` = low word, `DE` = high word).
pub(super) fn gen_expr32(a: &mut Asm, e: &Expr) {
    match e {
        Expr::Lit32(n) => {
            a.byte(0x21); // LD HL, low16
            a.word(*n as u16);
            a.byte(0x11); // LD DE, high16
            a.word((*n >> 16) as u16);
        }
        Expr::Var32(slot) => {
            let addr = slot_addr(a.base, *slot);
            a.byte(0x2A); // LD HL,(addr)      low word
            a.word(addr);
            a.byte(0xED);
            a.byte(0x5B); // LD DE,(addr+2)    high word
            a.word(addr.wrapping_add(2));
        }
        Expr::Trunc32(e) => gen_expr32(a, e),
        Expr::Bin32(op, l, r) => {
            gen_expr32(a, l);
            a.byte(0xD5); // PUSH DE   (l.high)
            a.byte(0xE5); // PUSH HL   (l.low)
            gen_expr32(a, r); // HL = r.low, DE = r.high
            a.byte(0xC1); // POP BC    (l.low)
            gen_bitwise_bc(a, op, false); // HL = r.low OP l.low
            a.byte(0xEB); // EX DE,HL  -> HL = r.high
            a.byte(0xC1); // POP BC    (l.high)
            gen_bitwise_bc(a, op, true); // HL = r.high OP l.high; EX back below
            a.byte(0xEB); // EX DE,HL  -> HL = low, DE = high
        }
        Expr::Shift32 { left, e, k } => {
            gen_expr32(a, e); // HL:DE = lo:hi
            for _ in 0..*k {
                if *left {
                    // DE:HL << 1  (low first, carry up)
                    a.byte(0xCB);
                    a.byte(0x25); // SLA L
                    a.byte(0xCB);
                    a.byte(0x14); // RL H
                    a.byte(0xCB);
                    a.byte(0x13); // RL E
                    a.byte(0xCB);
                    a.byte(0x12); // RL D
                } else {
                    // DE:HL >> 1  (high first, carry down)
                    a.byte(0xCB);
                    a.byte(0x3A); // SRL D
                    a.byte(0xCB);
                    a.byte(0x1B); // RR E
                    a.byte(0xCB);
                    a.byte(0x1C); // RR H
                    a.byte(0xCB);
                    a.byte(0x1D); // RR L
                }
            }
        }
        _ => unreachable!("not a u32 expression"),
    }
}

/// `HL = HL <op> BC` for one 16-bit word of a `u32` bitwise op (`| & ^`), word-wise
/// through the accumulator. `_high` is documentation only — the op is the same per word.
pub(super) fn gen_bitwise_bc(a: &mut Asm, op: &BinOp, _high: bool) {
    let (oc, ob) = match op {
        BinOp::Or => (0xB1, 0xB0),  // OR C / OR B
        BinOp::And => (0xA1, 0xA0), // AND C / AND B
        BinOp::Xor => (0xA9, 0xA8), // XOR C / XOR B
        _ => unreachable!("u32 supports only | & ^"),
    };
    a.byte(0x7D); // LD A,L
    a.byte(oc); // <op> C   -> A = L <op> C
    a.byte(0x6F); // LD L,A
    a.byte(0x7C); // LD A,H
    a.byte(ob); // <op> B   -> A = H <op> B
    a.byte(0x67); // LD H,A
}

/// `HL *= k` for a compile-time constant: a power of two shifts (`ADD HL,HL`), else
/// the `__mul16` micro-runtime.
pub(super) fn gen_mul_const(a: &mut Asm, k: u16) {
    if k == 1 {
        return;
    }
    if k == 0 {
        a.byte(0x21); // LD HL, 0
        a.word(0);
        return;
    }
    if k.is_power_of_two() {
        for _ in 0..k.trailing_zeros() {
            a.byte(0x29); // ADD HL,HL
        }
        return;
    }
    // General constant: shift-and-add (no `__mul16`). Move the value to DE, build the
    // result in HL by `result = result*2 (+ value)` per bit from the top.
    a.byte(0xEB); // EX DE,HL   (DE = value)
    a.byte(0x21); // LD HL, 0   (result)
    a.word(0);
    let top = 15 - k.leading_zeros();
    for i in (0..=top).rev() {
        a.byte(0x29); // ADD HL,HL   (result <<= 1)
        if k & (1 << i) != 0 {
            a.byte(0x19); // ADD HL,DE   (result += value)
        }
    }
}

/// Leave `HL = ptr + off + index*2` — the address of a `u16` array element reached
/// through a pointer (`self.arr[index]`). `index*2` uses `ADD HL,HL` (no multiply
/// runtime); `index` is evaluated once.
pub(super) fn gen_ptr_elem_addr(a: &mut Asm, ptr: &Expr, off: usize, index: &Expr) {
    gen_expr(a, index); // HL = index
    a.byte(0x29); // ADD HL,HL   (index * 2)
    a.byte(0xE5); // PUSH HL
    gen_expr(a, ptr); // HL = base pointer
    gen_add_offset(a, off); // HL = ptr + off
    a.byte(0xD1); // POP DE      (DE = index*2)
    a.byte(0x19); // ADD HL,DE   -> HL = ptr + off + index*2
}

/// `HL += off` (a small constant byte offset), if non-zero.
pub(super) fn gen_add_offset(a: &mut Asm, off: usize) {
    if off != 0 {
        a.byte(0x11); // LD DE, off
        a.word(off as u16);
        a.byte(0x19); // ADD HL, DE
    }
}

/// `HL = left <op> right` (16-bit, byte-wise), where `op_e`/`op_d` are the
/// `OP E` / `OP D` opcodes (commutative, so operand order is irrelevant).
pub(super) fn gen_bitwise(a: &mut Asm, l: &Expr, r: &Expr, op_e: u8, op_d: u8) {
    gen_expr(a, l);
    a.byte(0xE5); // PUSH HL
    gen_expr(a, r);
    a.byte(0xD1); // POP DE       (DE = l, HL = r)
    a.byte(0x7D); // LD A,L
    a.byte(op_e); // OP E
    a.byte(0x6F); // LD L,A
    a.byte(0x7C); // LD A,H
    a.byte(op_d); // OP D
    a.byte(0x67); // LD H,A
}

/// Evaluate so that `HL = second`, `DE = first` (the operand layout the runtime
/// and `SBC` want).
pub(super) fn gen_pair(a: &mut Asm, first: &Expr, second: &Expr) {
    gen_expr(a, first);
    a.byte(0xE5); // PUSH HL
    gen_expr(a, second);
    a.byte(0xD1); // POP DE  (DE = first)
}

/// Leave `HL = &base[index]` (each element is `u16`, so address = slot base + index*2).
pub(super) fn gen_elem_addr(a: &mut Asm, base: usize, index: &Expr) {
    gen_expr(a, index); // HL = index
    a.byte(0x29); // ADD HL,HL  (index * 2)
    let base_addr = slot_addr(a.base, base);
    a.byte(0x11); // LD DE, base_addr
    a.word(base_addr);
    a.byte(0x19); // ADD HL, DE  -> element address
}

/// `HL = left - right`, flags from the subtraction (carry = borrow).
pub(super) fn gen_sub(a: &mut Asm, left: &Expr, right: &Expr) {
    gen_pair(a, right, left); // HL = left, DE = right
    a.byte(0xB7); // OR A   (clear carry)
    a.byte(0xED);
    a.byte(0x52); // SBC HL, DE
}
