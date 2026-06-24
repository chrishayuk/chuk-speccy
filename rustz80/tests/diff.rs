//! Differential testing — the compiler's oracle (spec 07 §8). Each `check!` takes
//! one Rust block and runs it two ways: under **rustc** (a host `fn`) and through
//! **rustz80** onto our Z80 (compile → run → read `HL`). They must agree. The
//! single-source property is what makes this airtight: there's no second copy to
//! drift.

// `check!` blocks are stringified into dialect source, so they must use the
// dialect's long-form (`x = x + 1`, an explicit swap) — not Rust's `+=`/`swap`.
// `needless_range_loop` is allowed deliberately: these tests exercise `for i in a..b`
// index loops as the *subject under test*, not as a style to refactor away.
#![allow(
    clippy::assign_op_pattern,
    clippy::manual_swap,
    clippy::needless_range_loop
)]

/// A flat 64K RAM bus — enough to run a compiled function.
struct Ram {
    mem: Vec<u8>,
}

impl z80::Bus for Ram {
    fn read(&mut self, a: u16) -> u8 {
        self.mem[a as usize]
    }
    fn write(&mut self, a: u16, v: u8) {
        self.mem[a as usize] = v;
    }
    fn input(&mut self, _: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _: u16, _: u8) {}
    fn contend(&mut self, _: u16, _: u32) {}
    fn tick(&mut self, _: u32) {}
}

/// Load `bytes` at `ORG`, `CALL` it from a trampoline that `HALT`s on return,
/// run to the halt, and return `HL`.
fn run(bytes: &[u8]) -> u16 {
    let mut bus = Ram {
        mem: vec![0u8; 0x1_0000],
    };
    let org = rustz80::ORG;
    // trampoline @ 0x7000:  CALL org ; HALT
    bus.mem[0x7000] = 0xCD;
    bus.mem[0x7001] = org as u8;
    bus.mem[0x7002] = (org >> 8) as u8;
    bus.mem[0x7003] = 0x76;
    bus.mem[org as usize..org as usize + bytes.len()].copy_from_slice(bytes);

    let mut cpu = z80::Cpu::new();
    cpu.reset();
    cpu.regs.pc = 0x7000;
    cpu.regs.sp = 0xFFF0;
    for _ in 0..1_000_000 {
        if cpu.halted {
            break;
        }
        cpu.step(&mut bus);
    }
    assert!(cpu.halted, "function did not return");
    cpu.regs.hl()
}

/// Compile + run one block both ways and assert they match.
macro_rules! check {
    ($body:block) => {{
        #[allow(unused_assignments)]
        fn host() -> u16 $body
        let src = format!("fn f() -> u16 {}", stringify!($body));
        let bytes = rustz80::compile_fn(&src).unwrap_or_else(|e| panic!("compile failed: {e}\nsrc: {src}"));
        let got = run(&bytes);
        assert_eq!(got, host(), "rustz80 vs rustc diverged\nsrc: {src}\n  z80={got} host={}", host());
    }};
}

#[test]
fn arithmetic() {
    check!({
        let a = 7u16;
        let b = 6u16;
        a + b
    });
    check!({
        let a = 1000u16;
        let b = 24u16;
        let c = 6u16;
        (a - b) + c
    });
    check!({
        let a = 5u16;
        a - 5u16 + 100u16
    });
}

#[test]
fn if_else() {
    check!({
        let a = 3u16;
        let b = 8u16;
        let mut m = a;
        if b > a {
            m = b;
        }
        m
    });
    check!({
        let x = 42u16;
        let mut r = 0u16;
        if x == 42u16 {
            r = 1u16;
        } else {
            r = 2u16;
        }
        r
    });
}

#[test]
fn while_loops() {
    // sum 0..10 = 45
    check!({
        let mut s = 0u16;
        let mut i = 0u16;
        while i < 10u16 {
            s = s + i;
            i = i + 1u16;
        }
        s
    });
    // countdown: multiply-by-repeated-addition (7 * 6 without a mul runtime yet)
    check!({
        let mut acc = 0u16;
        let mut n = 7u16;
        while n != 0u16 {
            acc = acc + 6u16;
            n = n - 1u16;
        }
        acc
    });
}

#[test]
fn mul_div_rem() {
    // `*`/`/`/`%` go through the appended micro-runtime — checked against rustc.
    check!({ 7u16 * 6u16 });
    check!({
        let a = 123u16;
        let b = 45u16;
        a * b
    });
    check!({ 1000u16 / 7u16 });
    check!({ 1000u16 % 7u16 });
    check!({
        let a = 9u16;
        let b = 4u16;
        a / b * b + a % b
    }); // == a
    check!({
        let mut s = 0u16;
        let mut i = 1u16;
        while i <= 5u16 {
            s = s + i * i;
            i = i + 1u16;
        }
        s
    }); // 1+4+9+16+25 = 55
}

/// Run a multi-function program from its `entry` symbol.
fn run_program(prog: &rustz80::Program, entry: &str) -> u16 {
    let mut bus = Ram {
        mem: vec![0u8; 0x1_0000],
    };
    let target = prog.symbols[entry];
    bus.mem[0x7000] = 0xCD;
    bus.mem[0x7001] = target as u8;
    bus.mem[0x7002] = (target >> 8) as u8;
    bus.mem[0x7003] = 0x76;
    let org = rustz80::ORG as usize;
    bus.mem[org..org + prog.code.len()].copy_from_slice(&prog.code);
    let mut cpu = z80::Cpu::new();
    cpu.reset();
    cpu.regs.pc = 0x7000;
    cpu.regs.sp = 0xFFF0;
    for _ in 0..1_000_000 {
        if cpu.halted {
            break;
        }
        cpu.step(&mut bus);
    }
    assert!(cpu.halted, "program did not return");
    cpu.regs.hl()
}

/// Like [`run_program`] but return the first three result registers `[HL, DE, BC]` —
/// to inspect a tuple return's register layout.
fn run_program_regs(prog: &rustz80::Program, entry: &str) -> [u16; 3] {
    let mut bus = Ram {
        mem: vec![0u8; 0x1_0000],
    };
    let target = prog.symbols[entry];
    bus.mem[0x7000] = 0xCD;
    bus.mem[0x7001] = target as u8;
    bus.mem[0x7002] = (target >> 8) as u8;
    bus.mem[0x7003] = 0x76;
    let org = rustz80::ORG as usize;
    bus.mem[org..org + prog.code.len()].copy_from_slice(&prog.code);
    let mut cpu = z80::Cpu::new();
    cpu.reset();
    cpu.regs.pc = 0x7000;
    cpu.regs.sp = 0xFFF0;
    for _ in 0..1_000_000 {
        if cpu.halted {
            break;
        }
        cpu.step(&mut bus);
    }
    assert!(cpu.halted, "program did not return");
    [cpu.regs.hl(), cpu.regs.de(), cpu.regs.bc()]
}

#[test]
fn function_calls() {
    // 1 + 2 args + the calling convention (HL/DE/BC), checked against rustc.
    fn add(a: u16, b: u16) -> u16 {
        a + b
    }
    fn sq(x: u16) -> u16 {
        x * x
    }
    fn f(a: u16, b: u16, c: u16) -> u16 {
        a + b * c
    }
    fn main_host() -> u16 {
        add(40, 2) + sq(5) - f(1, 2, 3)
    }

    let src = "
        fn add(a: u16, b: u16) -> u16 { a + b }
        fn sq(x: u16) -> u16 { x * x }
        fn f(a: u16, b: u16, c: u16) -> u16 { a + b * c }
        fn run() -> u16 { add(40u16, 2u16) + sq(5u16) - f(1u16, 2u16, 3u16) }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), main_host()); // 42 + 25 - 7 = 60
}

#[test]
fn scalar_u8() {
    // Non-overflowing u8 arithmetic widened to u16.
    check!({
        let a = 100u8;
        let b = 50u8;
        (a + b) as u16
    }); // 150
        // u8 wrapping must match rustc's wrapping_* exactly.
    check!({
        let a = 200u8;
        let b = 100u8;
        a.wrapping_add(b) as u16
    }); // 300 wraps to 44
    check!({
        let a = 10u8;
        let b = 20u8;
        a.wrapping_sub(b) as u16
    }); // wraps to 246
    check!({
        let a = 20u8;
        let b = 20u8;
        a.wrapping_mul(b) as u16
    }); // 400 wraps to 144
        // u16 -> u8 cast truncates to the low byte.
    check!({
        let x = 300u16;
        (x as u8) as u16
    }); // 44
        // u8 loop counter with widening reads.
    check!({
        let mut sum = 0u16;
        let mut i = 0u8;
        while (i as u16) < 5u16 {
            sum = sum + i as u16;
            i = i.wrapping_add(1u8);
        }
        sum
    }); // 0+1+2+3+4 = 10
}

#[test]
fn arrays() {
    // literal-indexed read/write
    check!({
        let mut a = [0u16; 4];
        a[0] = 10u16;
        a[1] = 20u16;
        a[2] = 30u16;
        a[3] = 40u16;
        a[1] + a[3]
    }); // 60
        // array literal + variable index (needs `as usize` — valid host Rust)
    check!({
        let a = [3u16, 1u16, 4u16, 1u16, 5u16];
        let mut sum = 0u16;
        let mut i = 0u16;
        while i < 5u16 {
            sum = sum + a[i as usize];
            i = i + 1u16;
        }
        sum
    }); // 14
        // fill via loop, read back
    check!({
        let mut sq = [0u16; 8];
        let mut i = 0u16;
        while i < 8u16 {
            sq[i as usize] = i * i;
            i = i + 1u16;
        }
        sq[7]
    }); // 49
        // in-place reverse, then read both ends
    check!({
        let mut a = [1u16, 2u16, 3u16, 4u16, 5u16];
        let mut i = 0u16;
        while i < 2u16 {
            let t = a[i as usize];
            a[i as usize] = a[(4u16 - i) as usize];
            a[(4u16 - i) as usize] = t;
            i = i + 1u16;
        }
        a[0] * 100u16 + a[4]
    }); // 5*100 + 1 = 501
}

#[test]
fn byte_arrays() {
    // u8 arrays store/load one byte per element; values widen to u16 with `as`.
    check!({
        let mut a = [0u8; 4];
        a[2] = 200u8;
        a[2] as u16
    }); // 200
    check!({
        let a = [10u8, 20u8, 30u8, 250u8];
        a[0] as u16 + a[3] as u16
    }); // 260
        // Low-byte truncation must match `as u8`.
    check!({
        let mut a = [0u8; 2];
        a[0] = 300u16 as u8;
        a[0] as u16
    }); // 300 as u8 = 44
        // Fill a byte array in a loop, read back.
    check!({
        let mut a = [0u8; 5];
        let mut i = 0u16;
        while i < 5u16 {
            a[i as usize] = (i * 10u16) as u8;
            i = i + 1u16;
        }
        a[4] as u16
    }); // 40
}

#[test]
fn structs() {
    // A struct literal, field reads/writes, and a struct passed across functions
    // by mutating fields locally — checked against rustc.
    struct Point {
        x: u16,
        y: u16,
    }
    fn host() -> u16 {
        let mut p = Point { x: 3, y: 4 };
        p.x = p.x + 10;
        p.y = p.y * 2;
        p.x * 100 + p.y // 13*100 + 8 = 1308
    }
    let src = "
        struct Point { x: u16, y: u16 }
        fn run() -> u16 {
            let mut p = Point { x: 3u16, y: 4u16 };
            p.x = p.x + 10u16;
            p.y = p.y * 2u16;
            p.x * 100u16 + p.y
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host());
}

#[test]
fn structs_compose_with_functions() {
    // Pass scalar fields into functions and combine the results.
    struct V {
        x: u16,
        y: u16,
    }
    fn area(w: u16, h: u16) -> u16 {
        w * h
    }
    fn host() -> u16 {
        let a = V { x: 6, y: 7 };
        let b = V { x: 3, y: 4 };
        area(a.x, a.y) + area(b.x, b.y) // 42 + 12 = 54
    }
    let src = "
        struct V { x: u16, y: u16 }
        fn area(w: u16, h: u16) -> u16 { w * h }
        fn run() -> u16 {
            let a = V { x: 6u16, y: 7u16 };
            let b = V { x: 3u16, y: 4u16 };
            area(a.x, a.y) + area(b.x, b.y)
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host());
}

#[test]
#[allow(unused_assignments)]
fn enum_and_match() {
    // C-like enum + match on it (arms assign a result), checked against rustc.
    #[allow(dead_code)]
    enum Color {
        Red,
        Green,
        Blue,
    }
    fn host() -> u16 {
        let c = Color::Green;
        let mut v = 0u16;
        match c {
            Color::Red => v = 100,
            Color::Green => v = 200,
            Color::Blue => v = 300,
        }
        v
    }
    let src = "
        enum Color { Red, Green, Blue }
        fn run() -> u16 {
            let c = Color::Green;
            let mut v = 0u16;
            match c {
                Color::Red => v = 100u16,
                Color::Green => v = 200u16,
                Color::Blue => v = 300u16,
            }
            v
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host());
}

#[test]
#[allow(unused_assignments)]
fn match_literals_with_wildcard_and_enum_param() {
    #[allow(dead_code)]
    enum Op {
        Add,
        Sub,
        Mul,
    }
    fn apply(op: Op, a: u16, b: u16) -> u16 {
        let mut r = 0u16;
        match op {
            Op::Add => r = a + b,
            Op::Sub => r = a - b,
            Op::Mul => r = a * b,
        }
        r
    }
    fn classify(n: u16) -> u16 {
        let mut r = 0u16;
        match n {
            0 => r = 10,
            1 => r = 20,
            _ => r = 99,
        }
        r
    }
    fn host() -> u16 {
        apply(Op::Add, 7, 6) + apply(Op::Mul, 4, 5) + classify(0) + classify(1) + classify(7)
    }
    let src = "
        enum Op { Add, Sub, Mul }
        fn apply(op: Op, a: u16, b: u16) -> u16 {
            let mut r = 0u16;
            match op {
                Op::Add => r = a + b,
                Op::Sub => r = a - b,
                Op::Mul => r = a * b,
            }
            r
        }
        fn classify(n: u16) -> u16 {
            let mut r = 0u16;
            match n {
                0u16 => r = 10u16,
                1u16 => r = 20u16,
                _ => r = 99u16,
            }
            r
        }
        fn run() -> u16 {
            apply(Op::Add, 7u16, 6u16) + apply(Op::Mul, 4u16, 5u16)
                + classify(0u16) + classify(1u16) + classify(7u16)
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host()); // 13 + 20 + 10 + 20 + 99 = 162
}

#[test]
fn methods_and_self() {
    // `&mut self` mutation through a pointer, plus a `&self` reader.
    struct Counter {
        n: u16,
    }
    impl Counter {
        fn bump(&mut self, by: u16) {
            self.n = self.n + by;
        }
        fn doubled(&self) -> u16 {
            self.n + self.n
        }
    }
    fn host() -> u16 {
        let mut c = Counter { n: 10 };
        c.bump(5);
        c.bump(7);
        c.doubled() // (10+5+7)*2 = 44
    }
    let src = "
        struct Counter { n: u16 }
        impl Counter {
            fn bump(&mut self, by: u16) { self.n = self.n + by; }
            fn doubled(&self) -> u16 { self.n + self.n }
        }
        fn run() -> u16 {
            let mut c = Counter { n: 10u16 };
            c.bump(5u16);
            c.bump(7u16);
            c.doubled()
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host());
}

#[test]
fn methods_call_self_and_two_structs() {
    // A method calling another method on `self`, and two structs sharing a name.
    struct Vec2 {
        x: u16,
        y: u16,
    }
    impl Vec2 {
        fn sum(&self) -> u16 {
            self.x + self.y
        }
        fn scaled_sum(&self, k: u16) -> u16 {
            self.sum() * k
        }
    }
    struct Sq {
        w: u16,
    }
    impl Sq {
        fn area(&self) -> u16 {
            self.w * self.w
        }
    }
    fn host() -> u16 {
        let v = Vec2 { x: 3, y: 4 };
        let b = Sq { w: 5 };
        v.scaled_sum(10) + b.area() // 7*10 + 25 = 95
    }
    let src = "
        struct Vec2 { x: u16, y: u16 }
        impl Vec2 {
            fn sum(&self) -> u16 { self.x + self.y }
            fn scaled_sum(&self, k: u16) -> u16 { self.sum() * k }
        }
        struct Sq { w: u16 }
        impl Sq { fn area(&self) -> u16 { self.w * self.w } }
        fn run() -> u16 {
            let v = Vec2 { x: 3u16, y: 4u16 };
            let b = Sq { w: 5u16 };
            v.scaled_sum(10u16) + b.area()
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host());
}

#[test]
fn bitwise() {
    check!({ 12u16 | 10u16 }); // 14
    check!({ 12u16 & 10u16 }); // 8
    check!({ 12u16 ^ 10u16 }); // 6
    check!({
        let a = 0xF0u8;
        let b = 0x0Fu8;
        (a | b) as u16
    }); // 255
    check!({
        let a = 200u8;
        let b = 0x0Fu8;
        (a & b) as u16
    }); // 200 & 15 = 8
}

/// Run a no-result program (entry `run`) on a 64K RAM bus and return the bus.
fn run_to_memory(prog: &rustz80::Program, entry: &str) -> Vec<u8> {
    let mut bus = Ram {
        mem: vec![0u8; 0x1_0000],
    };
    let target = prog.symbols[entry];
    bus.mem[0x7000] = 0xCD;
    bus.mem[0x7001] = target as u8;
    bus.mem[0x7002] = (target >> 8) as u8;
    bus.mem[0x7003] = 0x76;
    let org = rustz80::ORG as usize;
    bus.mem[org..org + prog.code.len()].copy_from_slice(&prog.code);
    let mut cpu = z80::Cpu::new();
    cpu.reset();
    cpu.regs.pc = 0x7000;
    cpu.regs.sp = 0xFFF0;
    for _ in 0..1_000_000 {
        if cpu.halted {
            break;
        }
        cpu.step(&mut bus);
    }
    assert!(cpu.halted, "program did not return");
    bus.mem
}

#[test]
fn pixels_to_screen() {
    // A `plot()` written in the dialect (div/mod screen math + a mask table),
    // writing pixels through the poke/peek raw-memory intrinsics. Verified against
    // the canonical ZX Spectrum address formula computed independently in Rust.
    let src = "
        fn plot(x: u16, y: u16) {
            let masks = [128u8, 64u8, 32u8, 16u8, 8u8, 4u8, 2u8, 1u8];
            let addr = 16384u16
                + (y / 64u16) * 2048u16
                + (y % 8u16) * 256u16
                + ((y / 8u16) % 8u16) * 32u16
                + x / 8u16;
            let m = masks[(x % 8u16) as usize];
            poke(addr, peek(addr) | m);
        }
        fn run() {
            plot(0u16, 0u16);
            plot(255u16, 191u16);
            plot(128u16, 96u16);
            plot(7u16, 1u16);
            plot(1u16, 100u16);
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    let mem = run_to_memory(&prog, "run");

    let pixels = [(0u16, 0u16), (255, 191), (128, 96), (7, 1), (1, 100)];
    let mut want = vec![0u8; 0x1_0000];
    for (x, y) in pixels {
        let addr = 0x4000 + ((y & 0xC0) << 5) + ((y & 0x07) << 8) + ((y & 0x38) << 2) + (x >> 3);
        want[addr as usize] |= 0x80 >> (x & 7);
    }
    assert_eq!(
        &mem[0x4000..0x5800],
        &want[0x4000..0x5800],
        "screen bytes differ"
    );
}

#[test]
fn for_loops() {
    // Exclusive range: sum 0..10 = 45.
    check!({
        let mut s = 0u16;
        for i in 0u16..10u16 {
            s = s + i;
        }
        s
    });
    // Inclusive range: sum 1..=5 = 15.
    check!({
        let mut s = 0u16;
        for i in 1u16..=5u16 {
            s = s + i;
        }
        s
    });
    // Variable upper bound, evaluated once.
    check!({
        let n = 6u16;
        let mut s = 0u16;
        for i in 0u16..n {
            s = s + i;
        }
        s
    }); // 0+1+2+3+4+5 = 15
        // `for` indexing an array (the render-loop shape).
    check!({
        let a = [3u16, 1u16, 4u16, 1u16, 5u16];
        let mut sum = 0u16;
        for i in 0u16..5u16 {
            sum = sum + a[i as usize];
        }
        sum
    }); // 14
        // Nested loops with distinct counters.
    check!({
        let mut s = 0u16;
        for i in 0u16..3u16 {
            for j in 0u16..3u16 {
                s = s + i * j;
            }
        }
        s
    }); // (0+1+2)*(0+1+2) = 9
        // `for _ in` — a counted loop whose variable is unused in the body.
    check!({
        let mut count = 0u16;
        for _ in 0u16..7u16 {
            count = count + 1u16;
        }
        count
    }); // 7
}

#[test]
fn loop_break_continue() {
    // `loop` exited by `break`.
    check!({
        let mut i = 0u16;
        loop {
            if i == 5u16 {
                break;
            }
            i = i + 1u16;
        }
        i
    }); // 5
        // do-while shape: body runs before the break test.
    check!({
        let mut i = 0u16;
        let mut s = 0u16;
        loop {
            s = s + i;
            i = i + 1u16;
            if i > 4u16 {
                break;
            }
        }
        s
    }); // 0+1+2+3+4 = 10
        // `continue` in a `while` re-checks the condition (no step skipped).
    check!({
        let mut i = 0u16;
        let mut s = 0u16;
        while i < 10u16 {
            i = i + 1u16;
            if (i % 2u16) == 1u16 {
                continue;
            }
            s = s + i;
        }
        s
    }); // evens 2+4+6+8+10 = 30
        // `continue` in a `for` MUST run the induction step (else it never ends).
    check!({
        let mut s = 0u16;
        for i in 0u16..10u16 {
            if (i % 2u16) == 0u16 {
                continue;
            }
            s = s + i;
        }
        s
    }); // odds 1+3+5+7+9 = 25
        // `break` out of a `for`.
    check!({
        let mut s = 0u16;
        for i in 0u16..100u16 {
            if i == 5u16 {
                break;
            }
            s = s + i;
        }
        s
    }); // 0+1+2+3+4 = 10
}

#[test]
fn early_return() {
    // Early `return` from an `if` (skips the tail), plus `loop` + `return` — the
    // rejection-sampling idiom (a loop with no `break`, exited only by `return`).
    #[allow(clippy::needless_return)]
    fn classify(n: u16) -> u16 {
        if n == 0 {
            return 100;
        }
        if n == 1 {
            return 200;
        }
        n + 1000
    }
    fn first_ge(start: u16, limit: u16) -> u16 {
        let mut i = start;
        loop {
            if i >= limit {
                return i;
            }
            i = i + 2;
        }
    }
    fn host() -> u16 {
        classify(0) + classify(1) + classify(5) + first_ge(1, 10)
    }
    let src = "
        fn classify(n: u16) -> u16 {
            if n == 0u16 { return 100u16; }
            if n == 1u16 { return 200u16; }
            n + 1000u16
        }
        fn first_ge(start: u16, limit: u16) -> u16 {
            let mut i = start;
            loop {
                if i >= limit { return i; }
                i = i + 2u16;
            }
        }
        fn run() -> u16 {
            classify(0u16) + classify(1u16) + classify(5u16) + first_ge(1u16, 10u16)
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host()); // 100 + 200 + 1005 + 11 = 1316
}

#[test]
fn generics() {
    // Real generic Rust — monomorphized per call. `clamp` is generic *and* calls two
    // other generics, so one `clamp(_u16)` transitively instantiates `max`/`min` too.
    // The same source type-checks under rustc (bounds satisfied) and compiles here.
    fn max<T: Ord + Copy>(a: T, b: T) -> T {
        let mut r = a;
        if b > a {
            r = b;
        }
        r
    }
    fn min<T: Ord + Copy>(a: T, b: T) -> T {
        let mut r = a;
        if b < a {
            r = b;
        }
        r
    }
    fn clamp<T: Ord + Copy>(x: T, lo: T, hi: T) -> T {
        min(max(x, lo), hi)
    }
    fn host() -> u16 {
        let a = clamp(50u16, 10, 40); // 40
        let b = clamp(5u16, 10, 40); // 10
        let u = clamp(200u8, 50, 150); // 150 (a u8 instance)
        a + b + u as u16
    }
    let src = "
        fn max<T: Ord + Copy>(a: T, b: T) -> T { let mut r = a; if b > a { r = b; } r }
        fn min<T: Ord + Copy>(a: T, b: T) -> T { let mut r = a; if b < a { r = b; } r }
        fn clamp<T: Ord + Copy>(x: T, lo: T, hi: T) -> T { min(max(x, lo), hi) }
        fn run() -> u16 {
            let a = clamp(50u16, 10u16, 40u16);
            let b = clamp(5u16, 10u16, 40u16);
            let u = clamp(200u8, 50u8, 150u8);
            a + b + u as u16
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host()); // 40 + 10 + 150 = 200

    // The type argument is inferred (no turbofish), and `clamp` was instantiated at
    // both u16 and u8 — distinct monomorphic copies, each pulling in max/min.
    for sym in ["clamp$u16", "clamp$u8", "max$u16", "min$u8"] {
        assert!(prog.symbols.contains_key(sym), "missing instance {sym}");
    }
}

#[test]
fn generics_substitute_width() {
    // The instantiation's width is real: at u8 the body wraps (mod 256), at u16 it
    // does not — proving monomorphization substitutes the type, not just the name.
    // (`add` is only compiled here, not run on host, so the u8 overflow is fine.)
    let src = "
        fn add<T: core::ops::Add<Output = T> + Copy>(a: T, b: T) -> T { a + b }
        fn at_u16() -> u16 { add(200u16, 100u16) }
        fn at_u8() -> u16 { add(200u8, 100u8) as u16 }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "at_u16"), 300); // u16: no wrap
    assert_eq!(run_program(&prog, "at_u8"), 44); // u8: 300 wraps to 44
}

#[test]
fn generic_structs() {
    // A generic struct + generic methods. The same source type-checks under rustc and
    // compiles here (type arguments erased to 16-bit, like any struct's fields).
    struct Pair<T> {
        a: T,
        b: T,
    }
    impl<T: Copy + core::ops::Add<Output = T>> Pair<T> {
        fn sum(&self) -> T {
            self.a + self.b
        }
        fn bump(&mut self, d: T) {
            self.a = self.a + d;
        }
    }
    fn host() -> u16 {
        let mut p = Pair { a: 30u16, b: 12u16 };
        p.bump(3); // a = 33
        let q = Pair { a: 5u16, b: 7u16 };
        p.sum() + q.sum() // 45 + 12 = 57
    }
    let src = "
        struct Pair<T> { a: T, b: T }
        impl<T> Pair<T> {
            fn sum(&self) -> T { self.a + self.b }
            fn bump(&mut self, d: T) { self.a = self.a + d; }
        }
        fn run() -> u16 {
            let mut p = Pair { a: 30u16, b: 12u16 };
            p.bump(3u16);
            let q = Pair { a: 5u16, b: 7u16 };
            p.sum() + q.sum()
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host()); // 57
}

#[test]
fn tuples() {
    // Multiple return values via tuples (in HL/DE/BC), destructured at the call site.
    fn divmod(a: u16, b: u16) -> (u16, u16) {
        (a / b, a % b)
    }
    fn minmax(a: u16, b: u16) -> (u16, u16) {
        let mut lo = a;
        let mut hi = b;
        if a > b {
            lo = b;
            hi = a;
        }
        (lo, hi)
    }
    fn three() -> (u16, u16, u16) {
        (1, 2, 3)
    }
    fn host() -> u16 {
        let (q, r) = divmod(1000, 7); // (142, 6)
        let (x, y) = (7u16, 3u16); // tuple-literal destructure
        let (lo, hi) = minmax(x, y); // (3, 7)
        let (a, b, c) = three(); // 3-tuple return
        q * 100 + r + hi * 10 + lo + a * 100 + b * 10 + c
    }
    let src = "
        fn divmod(a: u16, b: u16) -> (u16, u16) { (a / b, a % b) }
        fn minmax(a: u16, b: u16) -> (u16, u16) {
            let mut lo = a;
            let mut hi = b;
            if a > b { lo = b; hi = a; }
            (lo, hi)
        }
        fn three() -> (u16, u16, u16) { (1u16, 2u16, 3u16) }
        fn run() -> u16 {
            let (q, r) = divmod(1000u16, 7u16);
            let (x, y) = (7u16, 3u16);
            let (lo, hi) = minmax(x, y);
            let (a, b, c) = three();
            q * 100u16 + r + hi * 10u16 + lo + a * 100u16 + b * 10u16 + c
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    // 14206 + 73 + 123 = 14402
    assert_eq!(run_program(&prog, "run"), host());
}

#[test]
fn monomorphization() {
    // Two type parameters → a combined instance key; the same instantiation called
    // twice is emitted once (dedup); distinct types make distinct instances.
    fn id<T>(x: T) -> T {
        x
    }
    fn first<A, B>(a: A, _b: B) -> A {
        a
    }
    fn host() -> u16 {
        id(10u16) + id(5u8) as u16 + first(7u16, 2u8) + first(7u16, 2u8)
    }
    let src = "
        fn id<T>(x: T) -> T { x }
        fn first<A, B>(a: A, b: B) -> A { a }
        fn run() -> u16 {
            id(10u16) + id(5u8) as u16 + first(7u16, 2u8) + first(7u16, 2u8)
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host()); // 10 + 5 + 7 + 7 = 29

    let instances: Vec<&String> = prog.symbols.keys().filter(|k| k.contains('$')).collect();
    // id at u16 and u8, first at (u16, u8) once — exactly three instances.
    assert_eq!(instances.len(), 3, "instances: {instances:?}");
    for sym in ["id$u16", "id$u8", "first$u16_u8"] {
        assert!(prog.symbols.contains_key(sym), "missing {sym}");
    }
    // `first` called twice with the same types ⇒ a single instance (dedup).
    assert_eq!(
        prog.symbols
            .keys()
            .filter(|k| k.starts_with("first"))
            .count(),
        1
    );
}

#[test]
fn generic_methods() {
    // Methods on a generic struct, including a method calling another on `self`.
    struct Acc<T> {
        v: T,
    }
    impl<T: Copy + core::ops::Add<Output = T> + core::ops::Mul<Output = T>> Acc<T> {
        fn add(&mut self, d: T) {
            self.v = self.v + d;
        }
        fn doubled(&self) -> T {
            self.v + self.v
        }
        fn add_then_double(&mut self, d: T) -> T {
            self.add(d);
            self.doubled()
        }
    }
    fn host() -> u16 {
        let mut a = Acc { v: 10u16 };
        a.add_then_double(5) // (10+5)*2 = 30
    }
    let src = "
        struct Acc<T> { v: T }
        impl<T> Acc<T> {
            fn add(&mut self, d: T) { self.v = self.v + d; }
            fn doubled(&self) -> T { self.v + self.v }
            fn add_then_double(&mut self, d: T) -> T { self.add(d); self.doubled() }
        }
        fn run() -> u16 {
            let mut a = Acc { v: 10u16 };
            a.add_then_double(5u16)
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host()); // 30
}

#[test]
fn tuple_layout() {
    // A tuple return lands in HL/DE/BC in order — verify the register layout directly.
    let src = "
        fn pair() -> (u16, u16) { (42u16, 7u16) }
        fn triple() -> (u16, u16, u16) { (11u16, 22u16, 33u16) }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    let [hl, de, _] = run_program_regs(&prog, "pair");
    assert_eq!((hl, de), (42, 7), "2-tuple → (HL, DE)");
    assert_eq!(
        run_program_regs(&prog, "triple"),
        [11, 22, 33],
        "3-tuple → (HL, DE, BC)"
    );
}

#[test]
fn size_report_covers_image() {
    let src = "
        fn id<T>(x: T) -> T { x }
        fn run() -> u16 { id(1u16) + id(2u8) as u16 }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    let report = prog.size_report();

    // One entry per symbol; sizes tile the whole image with no gaps or overlaps.
    assert_eq!(report.len(), prog.symbols.len());
    let total: usize = report.iter().map(|f| f.size as usize).sum();
    assert_eq!(total, prog.code.len(), "sizes cover the whole image");
    assert!(report.iter().all(|f| f.size > 0), "every fn emits ≥ 1 byte");
    // Monomorphic instances are present and flagged.
    assert!(report.iter().any(|f| f.name == "id$u16" && f.instance));
    assert!(report.iter().any(|f| f.name == "id$u8" && f.instance));
    // Entries are in layout order (ascending address).
    assert!(report.windows(2).all(|w| w[0].addr <= w[1].addr));
}

#[test]
fn tuple_struct_fields() {
    // A struct with a tuple field, accessed by `.0`/`.1` — by value and through a
    // `&mut self` receiver.
    struct Sprite {
        pos: (u16, u16),
        id: u16,
    }
    impl Sprite {
        fn mv(&mut self, dx: u16, dy: u16) {
            self.pos.0 = self.pos.0 + dx;
            self.pos.1 = self.pos.1 + dy;
        }
        fn key(&self) -> u16 {
            self.pos.0 * 100 + self.pos.1 + self.id
        }
    }
    fn host() -> u16 {
        let mut s = Sprite { pos: (3, 4), id: 7 };
        s.mv(2, 5); // pos = (5, 9)
        s.key() // 5*100 + 9 + 7 = 516
    }
    let src = "
        struct Sprite { pos: (u16, u16), id: u16 }
        impl Sprite {
            fn mv(&mut self, dx: u16, dy: u16) {
                self.pos.0 = self.pos.0 + dx;
                self.pos.1 = self.pos.1 + dy;
            }
            fn key(&self) -> u16 { self.pos.0 * 100u16 + self.pos.1 + self.id }
        }
        fn run() -> u16 {
            let mut s = Sprite { pos: (3u16, 4u16), id: 7u16 };
            s.mv(2u16, 5u16);
            s.key()
        }
    ";
    let prog = rustz80::compile_program(src).expect("compile");
    assert_eq!(run_program(&prog, "run"), host()); // 516
}

#[test]
fn tuple_rejections() {
    // More than three return values has no register convention.
    assert!(rustz80::compile_program(
        "fn f() -> (u16, u16, u16, u16) { (1u16, 2u16, 3u16, 4u16) } fn run() -> u16 { 0u16 }"
    )
    .is_err());
    // A tuple binding needs a tuple literal or a function call as its RHS.
    assert!(rustz80::compile_fn("fn f() -> u16 { let (a, b) = 5u16; a + b }").is_err());
}

#[test]
fn control_flow_rejections() {
    // `break`/`continue` outside any loop are rejected (not dangling jumps).
    assert!(rustz80::compile_fn("fn f() -> u16 { break; 0u16 }").is_err());
    assert!(rustz80::compile_fn("fn f() -> u16 { continue; 0u16 }").is_err());
    // `break <value>` and labeled break/continue are outside the subset.
    assert!(rustz80::compile_fn("fn f() -> u16 { loop { break 1u16; } }").is_err());
    assert!(rustz80::compile_fn("fn f() -> u16 { 'a: loop { break 'a; } }").is_err());
    // `for` only accepts integer ranges, not arbitrary iterators.
    assert!(rustz80::compile_fn("fn f() -> u16 { let a = [1u16]; for x in a { } 0u16 }").is_err());
}

#[test]
fn unsupported_is_an_error() {
    // f32 is outside the dialect → a clear compile error (the host-only signal).
    assert!(rustz80::compile_fn("fn f() -> u16 { let x = 1.5f32; 0u16 }").is_err());
}
