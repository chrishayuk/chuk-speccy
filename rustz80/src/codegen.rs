//! Naive Z80 codegen (Stage 1). `HL` is the working accumulator, `DE` secondary;
//! locals (incl. parameters) live in a fixed RAM scratch region (the "virtual
//! register file") and expressions evaluate via the stack. Functions follow the
//! spec-07 calling convention; `*`/`/`/`%` call an appended micro-runtime.
//! Correct first — peephole/strength-reduce come in Stage 2.

use crate::ir::*;
use std::collections::HashMap;

/// Locals: slot `i` lives at `SCRATCH + i*2` (`u16` each). Each function reuses
/// the same region (Stage 1 has no recursion / overlapping live ranges yet).
const SCRATCH: u16 = 0x9000;

/// `__mul16`: HL = HL * DE (low 16). Shift-add; clobbers AF/BC/DE.
const MUL16: &[u8] = &[
    0x44, 0x4D, // ld b,h ; ld c,l   (BC = multiplicand)
    0x21, 0x00, 0x00, // ld hl,0     (product)
    0x3E, 0x10, // ld a,16
    0xCB, 0x3A, 0xCB, 0x1B, // srl d ; rr e   (DE >>= 1, bit -> CF)
    0x30, 0x01, // jr nc,+1
    0x09, // add hl,bc
    0xCB, 0x21, 0xCB, 0x10, // sla c ; rl b    (BC <<= 1)
    0x3D, 0x20, 0xF2, // dec a ; jr nz
    0xC9, // ret
];

/// `__divmod16`: HL/DE -> HL=quotient, DE=remainder (divisor < 0x8000).
/// Restoring division; clobbers AF/BC.
const DIVMOD16: &[u8] = &[
    0x44, 0x4D, // ld b,h ; ld c,l   (BC = dividend)
    0x21, 0x00, 0x00, // ld hl,0     (remainder)
    0x3E, 0x10, // ld a,16
    0xCB, 0x21, 0xCB, 0x10, // sla c ; rl b   (BC <<= 1, MSB -> CF)
    0xED, 0x6A, // adc hl,hl   (rem = rem*2 + bit)
    0xED, 0x52, // sbc hl,de   (rem -= divisor)
    0x30, 0x03, // jr nc,+3 -> set
    0x19, // add hl,de   (restore)
    0x18, 0x01, // jr +1 -> cont
    0x0C, // set: inc c   (quotient bit)
    0x3D, 0x20, 0xEF, // cont: dec a ; jr nz
    0xEB, // ex de,hl    (DE = remainder)
    0x60, 0x69, // ld h,b ; ld l,c   (HL = quotient)
    0xC9, // ret
];

struct Asm {
    org: u16,
    code: Vec<u8>,
    labels: Vec<Option<u16>>,
    label_fixups: Vec<(usize, usize)>,
    symbols: HashMap<String, u16>,
    call_fixups: Vec<(usize, String)>,
    needs_mul: bool,
    needs_div: bool,
    /// Slot offset for the function currently being emitted, so each function's
    /// locals occupy a disjoint scratch region (correct for non-recursive calls;
    /// real stack frames are a later stage).
    base: u16,
    /// Enclosing loops as `(continue target, break target)` labels — the innermost
    /// is last. `continue`/`break` jump to the top entry's targets.
    loop_stack: Vec<(usize, usize)>,
    /// The current function's epilogue label — `return` jumps here (the value is
    /// already in `HL`).
    func_end: Option<usize>,
}

impl Asm {
    fn new(org: u16) -> Self {
        Asm {
            org,
            code: Vec::new(),
            labels: Vec::new(),
            label_fixups: Vec::new(),
            symbols: HashMap::new(),
            call_fixups: Vec::new(),
            needs_mul: false,
            needs_div: false,
            base: 0,
            loop_stack: Vec::new(),
            func_end: None,
        }
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
    fn jump(&mut self, opcode: u8, l: usize) {
        self.byte(opcode);
        self.label_fixups.push((self.code.len(), l));
        self.word(0);
    }
    /// Emit `CALL name` (resolved to the symbol address at finish).
    fn call(&mut self, name: &str) {
        self.byte(0xCD);
        self.call_fixups.push((self.code.len(), name.to_string()));
        self.word(0);
    }
    fn define(&mut self, name: &str) {
        let here = self.here();
        self.symbols.insert(name.to_string(), here);
    }
    fn finish(mut self) -> (Vec<u8>, HashMap<String, u16>) {
        // Append the micro-runtime routines that were used.
        if self.needs_mul {
            self.define("__mul16");
            self.code.extend_from_slice(MUL16);
        }
        if self.needs_div {
            self.define("__divmod16");
            self.code.extend_from_slice(DIVMOD16);
        }
        for (pos, l) in &self.label_fixups {
            let a = self.labels[*l].expect("unplaced label");
            self.code[*pos] = a as u8;
            self.code[*pos + 1] = (a >> 8) as u8;
        }
        for (pos, name) in &self.call_fixups {
            let a = *self
                .symbols
                .get(name)
                .unwrap_or_else(|| panic!("unknown call target {name}"));
            self.code[*pos] = a as u8;
            self.code[*pos + 1] = (a >> 8) as u8;
        }
        (self.code, self.symbols)
    }
}

fn slot_addr(base: u16, slot: usize) -> u16 {
    SCRATCH + (base + slot as u16) * 2
}

/// Compile a whole program (functions laid out in order, micro-runtime appended).
///
/// If `entry` is set, a tiny `DI; CALL entry; EI; RET` trampoline is emitted **at
/// `org`** so callers can `USR org`. The `DI` matters: the compiler keeps live
/// values in `DE`/`BC` across instructions, but the Spectrum's interrupt routine
/// clobbers `BC`/`DE` (its keyboard scan), so an interrupt mid-computation would
/// corrupt arithmetic. Disabling interrupts for the run avoids that; `EI` restores
/// them before returning to BASIC.
pub fn codegen_program(
    funcs: &[(String, Func)],
    org: u16,
    entry: Option<&str>,
) -> (Vec<u8>, HashMap<String, u16>) {
    let mut a = Asm::new(org);
    if let Some(e) = entry {
        a.byte(0xF3); // DI
        a.call(e); // CALL entry
        a.byte(0xFB); // EI
        a.byte(0xC9); // RET
    }
    let mut base = 0u16;
    for (name, func) in funcs {
        a.define(name);
        a.base = base;
        emit_func(&mut a, func);
        base += func.n_locals as u16;
    }
    a.finish()
}

/// A generic **frame-synced entry loop** at `org`: zero a `state_bytes` region at
/// `state_base`, then each interrupt do `EI; HALT; DI; CALL entry(state_base, 0, 0);
/// JP loop` — interrupts on only for the `HALT` frame-sync, off during `entry` (so
/// its arithmetic isn't corrupted by the ROM's keyboard scan). The compiler knows
/// nothing about "games": `entry`, `state_base`, and `state_bytes` are the caller's.
pub fn codegen_loop(
    funcs: &[(String, Func)],
    org: u16,
    entry: &str,
    state_base: u16,
    state_bytes: u16,
) -> Vec<u8> {
    let mut a = Asm::new(org);
    a.byte(0xF3); // DI
                  // Zero the state region (memset via LD (HL),0 + LDIR).
    if state_bytes >= 2 {
        a.byte(0x21);
        a.word(state_base); // LD HL, STATE
        a.byte(0x36);
        a.byte(0x00); // LD (HL), 0
        a.byte(0x11);
        a.word(state_base + 1); // LD DE, STATE+1
        a.byte(0x01);
        a.word(state_bytes - 1); // LD BC, n-1
        a.byte(0xED);
        a.byte(0xB0); // LDIR
    } else if state_bytes == 1 {
        a.byte(0x21);
        a.word(state_base);
        a.byte(0x36);
        a.byte(0x00);
    }
    let loop_l = a.label();
    a.place(loop_l);
    a.byte(0xFB); // EI
    a.byte(0x76); // HALT     (wait for the 50 Hz frame interrupt)
    a.byte(0xF3); // DI
    a.byte(0x21);
    a.word(state_base); // LD HL, &state   (first arg)
    a.byte(0x11);
    a.word(0); // LD DE, 0   (second arg, unused)
    a.byte(0x01);
    a.word(0); // LD BC, 0   (third arg, unused)
    a.call(entry); // CALL entry
    a.jump(0xC3, loop_l); // JP loop

    let mut base = 0u16;
    for (name, func) in funcs {
        a.define(name);
        a.base = base;
        emit_func(&mut a, func);
        base += func.n_locals as u16;
    }
    a.finish().0
}

fn emit_func(a: &mut Asm, f: &Func) {
    // Prologue: copy parameters from the convention registers into their slots.
    for i in 0..f.params {
        let addr = slot_addr(a.base, i);
        match i {
            0 => {
                a.byte(0x22); // LD (addr), HL
                a.word(addr);
            }
            1 => {
                a.byte(0xED); // LD (addr), DE
                a.byte(0x53);
                a.word(addr);
            }
            2 => {
                a.byte(0xED); // LD (addr), BC
                a.byte(0x43);
                a.word(addr);
            }
            _ => unreachable!(),
        }
    }
    // The epilogue label — `return` jumps here. The body and tail fall through to
    // it; an early `return` skips the tail (its value is already in `HL`).
    let end = a.label();
    a.func_end = Some(end);
    for s in &f.body {
        gen_stmt(a, s);
    }
    gen_return(a, &f.ret);
    a.place(end);
    a.func_end = None;
    a.byte(0xC9); // RET
}

/// Emit a function's return values into the result convention `HL`/`DE`/`BC`: none
/// for a void fn, `HL` for a scalar, two/three registers for a tuple. Each value is
/// pushed, then popped into its register in reverse so the first lands in `HL`.
fn gen_return(a: &mut Asm, rets: &[Expr]) {
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

/// Evaluate `e`, leaving the result in `HL`.
fn gen_expr(a: &mut Asm, e: &Expr) {
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
            match op {
                BinOp::Add => {
                    gen_expr(a, l);
                    a.byte(0xE5); // PUSH HL
                    gen_expr(a, r);
                    a.byte(0xD1); // POP DE  (DE = l)
                    a.byte(0x19); // ADD HL, DE
                }
                BinOp::Sub => gen_sub(a, l, r),
                BinOp::Mul => {
                    gen_pair(a, l, r); // HL = r, DE = l
                    a.call("__mul16"); // HL = l * r
                    a.needs_mul = true;
                }
                BinOp::Div => {
                    gen_pair(a, r, l); // HL = l, DE = r
                    a.call("__divmod16"); // HL = l / r
                    a.needs_div = true;
                }
                BinOp::Rem => {
                    gen_pair(a, r, l); // HL = l, DE = r
                    a.call("__divmod16"); // DE = l % r
                    a.byte(0xEB); // EX DE, HL  -> HL = remainder
                    a.needs_div = true;
                }
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
                    for _ in 0..lit_u8(r) {
                        a.byte(0xCB);
                        a.byte(0x3C); // SRL H
                        a.byte(0xCB);
                        a.byte(0x1D); // RR L   (logical >> 1)
                    }
                }
            }
            if *w == Width::Byte {
                a.byte(0x26); // LD H, 0   (wrap to u8)
                a.byte(0x00);
            }
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
        Expr::Lit32(_) | Expr::Var32(_) | Expr::Bin32(..) | Expr::Shift32 { .. } => {
            unreachable!("u32 node used in a 16-bit context (u32 params/returns unsupported)")
        }
    }
}

/// The first literal operand as a `u8` shift amount (the lowering guarantees a literal).
fn lit_u8(e: &Expr) -> u8 {
    match e {
        Expr::Lit(k) => *k as u8,
        _ => unreachable!("shift amount must be a constant"),
    }
}

/// Evaluate a `u32` expression into the `HL:DE` pair (`HL` = low word, `DE` = high word).
fn gen_expr32(a: &mut Asm, e: &Expr) {
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
fn gen_bitwise_bc(a: &mut Asm, op: &BinOp, _high: bool) {
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
fn gen_mul_const(a: &mut Asm, k: u16) {
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
    } else {
        a.byte(0x11); // LD DE, k
        a.word(k);
        a.call("__mul16"); // HL = HL * k
        a.needs_mul = true;
    }
}

/// Leave `HL = ptr + off + index*2` — the address of a `u16` array element reached
/// through a pointer (`self.arr[index]`). `index*2` uses `ADD HL,HL` (no multiply
/// runtime); `index` is evaluated once.
fn gen_ptr_elem_addr(a: &mut Asm, ptr: &Expr, off: usize, index: &Expr) {
    gen_expr(a, index); // HL = index
    a.byte(0x29); // ADD HL,HL   (index * 2)
    a.byte(0xE5); // PUSH HL
    gen_expr(a, ptr); // HL = base pointer
    gen_add_offset(a, off); // HL = ptr + off
    a.byte(0xD1); // POP DE      (DE = index*2)
    a.byte(0x19); // ADD HL,DE   -> HL = ptr + off + index*2
}

/// `HL += off` (a small constant byte offset), if non-zero.
fn gen_add_offset(a: &mut Asm, off: usize) {
    if off != 0 {
        a.byte(0x11); // LD DE, off
        a.word(off as u16);
        a.byte(0x19); // ADD HL, DE
    }
}

/// `HL = left <op> right` (16-bit, byte-wise), where `op_e`/`op_d` are the
/// `OP E` / `OP D` opcodes (commutative, so operand order is irrelevant).
fn gen_bitwise(a: &mut Asm, l: &Expr, r: &Expr, op_e: u8, op_d: u8) {
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
fn gen_pair(a: &mut Asm, first: &Expr, second: &Expr) {
    gen_expr(a, first);
    a.byte(0xE5); // PUSH HL
    gen_expr(a, second);
    a.byte(0xD1); // POP DE  (DE = first)
}

/// Leave `HL = &base[index]` (each element is `u16`, so address = slot base + index*2).
fn gen_elem_addr(a: &mut Asm, base: usize, index: &Expr) {
    gen_expr(a, index); // HL = index
    a.byte(0x29); // ADD HL,HL  (index * 2)
    let base_addr = slot_addr(a.base, base);
    a.byte(0x11); // LD DE, base_addr
    a.word(base_addr);
    a.byte(0x19); // ADD HL, DE  -> element address
}

/// `HL = left - right`, flags from the subtraction (carry = borrow).
fn gen_sub(a: &mut Asm, left: &Expr, right: &Expr) {
    gen_pair(a, right, left); // HL = left, DE = right
    a.byte(0xB7); // OR A   (clear carry)
    a.byte(0xED);
    a.byte(0x52); // SBC HL, DE
}

fn gen_stmt(a: &mut Asm, s: &Stmt) {
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
fn gen_cond_skip(a: &mut Asm, cond: &Cond, target: usize) {
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
