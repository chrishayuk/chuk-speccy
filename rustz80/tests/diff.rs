//! Differential testing — the compiler's oracle (spec 07 §8). Each `check!` takes
//! one Rust block and runs it two ways: under **rustc** (a host `fn`) and through
//! **rustz80** onto our Z80 (compile → run → read `HL`). They must agree. The
//! single-source property is what makes this airtight: there's no second copy to
//! drift.

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
    let mut bus = Ram { mem: vec![0u8; 0x1_0000] };
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
    let mut bus = Ram { mem: vec![0u8; 0x1_0000] };
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
fn unsupported_is_an_error() {
    // f32 is outside the dialect → a clear compile error (the host-only signal).
    assert!(rustz80::compile_fn("fn f() -> u16 { let x = 1.5f32; 0u16 }").is_err());
}
