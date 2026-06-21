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
fn unsupported_is_an_error() {
    // f32 is outside the dialect → a clear compile error (the host-only signal).
    assert!(rustz80::compile_fn("fn f() -> u16 { let x = 1.5f32; 0u16 }").is_err());
}
