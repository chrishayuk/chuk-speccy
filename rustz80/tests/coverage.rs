//! Coverage-focused tests for paths the differential/feature suites don't reach on
//! their own: error & rejection arms (each should be a *clean* compile error), prelude
//! handle routing, the frame-loop generator, the `inport` intrinsic, and array-struct
//! fields accessed through a `self` pointer.

use rustz80::{compile_fn, compile_program, PreludeConfig};

/// `compile_fn` must reject this source.
fn bad_fn(src: &str) {
    assert!(compile_fn(src).is_err(), "expected a compile error: {src}");
}
/// `compile_program` must reject this source.
fn bad_prog(src: &str) {
    assert!(
        compile_program(src).is_err(),
        "expected a compile error: {src}"
    );
}

#[test]
fn expr_rejections() {
    bad_fn("fn f() -> u16 { 1.5f32 as u16 }"); // unsupported literal
    bad_fn("fn f() -> u16 { let a = 1u16; a << 2u16 }"); // unsupported arithmetic op
    bad_fn("fn f() -> u16 { nope::<u16>() }"); // turbofish on a non-generic call
    bad_fn("fn f(a: u16, b: u16, c: u16, d: u16) -> u16 { a }"); // > 3 params
    bad_fn("fn f() -> u16 { g(1u16, 2u16, 3u16, 4u16) }"); // > 3 call args
}

#[test]
fn struct_field_rejections() {
    // Reading / assigning a whole multi-slot field (tuple) as a scalar.
    bad_prog("struct S { p: (u16, u16) } fn run() -> u16 { let s = S { p: (1u16, 2u16) }; s.p }");
    bad_prog(
        "struct S { p: (u16, u16) } \
         impl S { fn z(&mut self) { self.p = 5u16; } } \
         fn run() -> u16 { 0u16 }",
    );
    // A tuple field initialised with the wrong arity.
    bad_prog("struct S { p: (u16, u16) } fn run() -> u16 { let s = S { p: (1u16,) }; 0u16 }");
    // An array field initialised with the wrong number of elements.
    bad_prog("struct S { a: [u16; 4] } fn run() -> u16 { let s = S { a: [1u16, 2u16] }; s.a[0] }");
    // A method receiver that isn't a struct.
    bad_fn("fn f() -> u16 { let x = 1u16; x.bump(2u16) }");
}

#[test]
fn layout_rejections() {
    bad_prog("struct S(u16, u16); fn run() -> u16 { 0u16 }"); // tuple struct (unnamed fields)
    bad_prog("struct S { a: [u32; 2] } fn run() -> u16 { 0u16 }"); // non-u16 array field
    bad_prog("struct S { a: (u16, [u16; 2]) } fn run() -> u16 { 0u16 }"); // non-scalar tuple element
    bad_prog("struct S { a: &u16 } fn run() -> u16 { 0u16 }"); // unsupported field type (reference)
    bad_prog("enum E { A = 1u16 + 1u16 } fn run() -> u16 { 0u16 }"); // non-literal discriminant
    bad_prog("impl (u16, u16) { fn m(&self) {} } fn run() -> u16 { 0u16 }"); // unsupported impl target
    bad_prog("static X: u16 = 1; fn run() -> u16 { 0u16 }"); // unsupported item kind
    bad_prog("struct S { a: u16 } fn run() -> u16 { let s = S { b: 1u16 }; 0u16 }"); // no such field
    bad_prog("struct S { a: u16 } fn run() -> u16 { let s = S { a: 1u16 }; s.0 }"); // `.0` on a named struct
    bad_fn("fn f() -> u16 { let n = 4u16; let a = [0u16; n]; a[0] }"); // non-literal array length
}

#[test]
fn enum_explicit_discriminants() {
    // Explicit `= N` discriminants exercise the enum-literal parser.
    let prog = compile_program(
        "enum E { A = 5u16, B = 10u16, C = 20u16 }
         fn run() -> u16 { let x = E::B; x }",
    )
    .expect("compile");
    assert_eq!(run_fn(&prog, "run", 0), 10);
}

#[test]
fn generics_rejections() {
    // Lifetime params are rejected (it must have a type/const param to be collected
    // as generic and reach the rejection).
    bad_prog("fn g<'a, T>(x: T) -> u16 { 0u16 } fn run() -> u16 { g(1u16) }");
    // Can't infer a type argument that no value parameter carries.
    bad_prog("fn g<T>() -> u16 { 0u16 } fn run() -> u16 { g() }");
    // A const argument can't be inferred — it needs a turbofish.
    bad_prog("fn g<const N: usize>() -> u16 { let a = [0u16; N]; a[0] } fn run() -> u16 { g() }");
    // Wrong number of generic arguments, and a kind mismatch.
    bad_prog("fn id<T>(x: T) -> T { x } fn run() -> u16 { id::<u16, u8>(1u16) }");
    bad_prog("fn id<T>(x: T) -> T { x } fn run() -> u16 { id::<5>(1u16) }"); // type param, const arg
}

#[test]
fn stmt_rejections() {
    bad_fn("fn f() -> u16 { let x; x }"); // `let` without initializer
    bad_fn("fn f() -> u16 { loop { break 1u16; } }"); // break with a value
    bad_fn("fn f() -> u16 { 'a: loop { break 'a; } }"); // labeled loop
    bad_fn("fn f() -> u16 { break; 0u16 }"); // break outside a loop
    bad_fn("fn f() -> u16 { continue; 0u16 }"); // continue outside a loop
    bad_fn("fn f() -> u16 { let a = [1u16]; for x in a { } 0u16 }"); // for over a non-range
    bad_fn("fn f() -> u16 { for i in 0u16.. { } 0u16 }"); // unbounded range
    bad_fn("fn f() -> u16 { let (a, b) = 5u16; a + b }"); // tuple binding from a scalar
    bad_fn("fn f() -> u16 { let (a, b, c, d) = m(); a }"); // > 3 tuple binding
    bad_fn("fn f() -> u16 { match 1u16 { 1.5f32 => 0u16, _ => 1u16 } }"); // non-int pattern
}

#[test]
fn more_rejections() {
    // expr.rs
    bad_fn("fn f() -> u16 { let x = 1u16..5u16; 0u16 }"); // a range used as a value
    bad_prog("struct S { a: u16 } fn run() -> u16 { let s = S { a: 1u16 }; s.a.b }"); // nested named field
    bad_prog(
        "struct S { x: u16 } \
         impl S { fn m(&self, a: u16, b: u16, c: u16) -> u16 { a } } \
         fn run() -> u16 { let s = S { x: 0u16 }; s.m(1u16, 2u16, 3u16) }",
    ); // method receiver + 3 args > 3 registers
    bad_fn("fn f() -> u16 { 5u16.bump(1u16) }"); // method on a non-variable receiver
                                                 // stmt.rs
    bad_fn("fn f() -> u16 { let (a, b) = (1u16, 2u16, 3u16); a }"); // tuple-binding arity
    bad_fn("fn f() -> u16 { 'a: for i in 0u16..4u16 { } 0u16 }"); // labeled for
    bad_fn("fn f() -> u16 { while i_am_true() { fn z() {} } 0u16 }"); // item statement in a block
    bad_fn("fn f() -> u16 { let [a, b] = [1u16, 2u16]; a }"); // slice let pattern
    bad_prog("fn run() -> u16 { let n = 1u16; match n { 'a' => 0u16, _ => 1u16 } }"); // char pattern
    bad_prog("fn run() -> u16 { let n = 1u16; match n { (1u16, 2u16) => 0u16, _ => 1u16 } }");
    // tuple pattern
}

#[test]
fn more_branches() {
    // Generic fn with a void return (no return type) and one with a *concrete* return
    // type; an explicit type turbofish; an else-if chain; a typed `let`; a bare
    // `return`; and non-comparison conditions (treated as "non-zero").
    let prog = compile_program(
        "fn store<T>(addr: u16, x: T) { poke(addr, x as u8); }
         fn five<T>(x: T) -> u16 { 5u16 }
         fn id<T>(x: T) -> T { x }
         fn early() { return; }
         fn run() -> u16 {
             let y: u16 = 3u16;            // typed let
             store(49152u16, 9u16);        // void generic
             early();
             let mut r = id::<u16>(7u16);  // explicit type turbofish
             r = r + five(0u16);           // concrete-return generic -> +5
             if y == 1u16 { r = r + 1u16; } else if y == 3u16 { r = r + 100u16; } else { r = r + 2u16; }
             if (y - 3u16) { r = r + 1000u16; }  // paren, non-zero (false here)
             if y + 1u16 { r = r + 10u16; }       // non-comparison condition (true)
             r
         }",
    )
    .expect("compile");
    // 7 + 5 + 100 + 10 = 122
    assert_eq!(run_fn(&prog, "run", 0), 122);
}

#[test]
fn bool_and_inport() {
    // bool literals widen to 0/1; `inport` reads an I/O port.
    let prog = compile_program(
        "fn f() -> u16 {
            let t = true;
            let v = inport(254u16);
            let mut r = 0u16;
            if t { r = v; }
            r
        }
        fn run() -> u16 { f() }",
    )
    .expect("compile");
    // 0xFF on an unmapped port → low byte 255; `t` is true so r = 255.
    assert_eq!(run_fn(&prog, "run", 0), 255);
}

#[test]
fn prelude_handle_routing() {
    // A non-empty PreludeConfig: handle params (`Frame`/`Input`) route method calls to
    // free prelude functions, dropping the receiver — exercises the whole prelude path.
    let cfg = PreludeConfig::new()
        .route("Frame", "pixel", "__px")
        .route("Input", "held", "__held");
    let src = "
        fn __px(x: u16, y: u16) { poke(16384u16 + x, y as u8); }
        fn __held(b: u16) -> u16 { b }
        fn draw(frame: &mut Frame, input: &Input) {
            frame.pixel(3u16, 7u16);
            let h = input.held(4u16);
            poke(16390u16, h as u8);
        }
    ";
    let file: syn::File = syn::parse_str(src).unwrap();
    let funcs = rustz80::lower_program(&file, &cfg).expect("lower");
    assert!(funcs.iter().any(|(n, _)| n == "draw"));

    // An unrouted method on a handle is a clean error.
    let bad = "fn draw(frame: &mut Frame) { frame.nope(1u16); }";
    let f2: syn::File = syn::parse_str(bad).unwrap();
    assert!(rustz80::lower_program(&f2, &cfg).is_err());
}

#[test]
fn frame_loop_generator() {
    // `codegen_loop` (the SDK's frame-synced entry) — both the multi-byte and the
    // single-byte state-zeroing prologues.
    let file: syn::File = syn::parse_str("fn update() { poke(45056u16, 1u8); }").unwrap();
    let funcs = rustz80::lower_program(&file, &PreludeConfig::new()).unwrap();
    for state_bytes in [6u16, 1, 0] {
        let code = rustz80::codegen_loop(&funcs, rustz80::ORG, "update", 0xB000, state_bytes);
        assert_eq!(code[0], 0xF3, "frame loop starts with DI"); // DI
        assert!(code.len() > 8);
    }
}

#[test]
fn array_struct_field_through_pointer() {
    // A method filling and summing a `[u16; N]` field via `self.data[i]` — the
    // PtrIndex / PtrStoreIndex codegen, reached by calling the method with a `self`
    // pointer to a zeroed region.
    let src = "
        struct Buf { data: [u16; 4], total: u16 }
        impl Buf {
            fn work(&mut self) -> u16 {
                let mut i = 0u16;
                while i < 4u16 { self.data[i as usize] = i * 10u16; i = i + 1u16; }
                let mut s = 0u16;
                let mut j = 0u16;
                while j < 4u16 { s = s + self.data[j as usize]; j = j + 1u16; }
                self.total = s;
                s
            }
        }
    ";
    let prog = compile_program(src).expect("compile");
    // Call Buf::work with `self` pointing at a zeroed scratch region (0xC000).
    let (hl, mem) = run_method(&prog, "Buf::work", 0xC000);
    assert_eq!(hl, 60); // 0 + 10 + 20 + 30
    let total = u16::from_le_bytes([mem[0xC008], mem[0xC009]]); // 5th slot = `total`
    assert_eq!(total, 60);
    let d2 = u16::from_le_bytes([mem[0xC004], mem[0xC005]]); // data[2]
    assert_eq!(d2, 20);
}

// --- a tiny CPU runner (mirrors the differential harness) ----------------------

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

/// Call `entry` with no args (HL/DE/BC = arg if given) and return `HL`.
fn run_fn(prog: &rustz80::Program, entry: &str, _arg: u16) -> u16 {
    run_method(prog, entry, 0).0
}

/// Call `entry` with `HL = self_ptr`, run to the trampoline `HALT`, return (HL, memory).
fn run_method(prog: &rustz80::Program, entry: &str, self_ptr: u16) -> (u16, Vec<u8>) {
    let mut bus = Ram {
        mem: vec![0u8; 0x1_0000],
    };
    let target = prog.symbols[entry];
    bus.mem[0x7000] = 0x21; // LD HL, self_ptr
    bus.mem[0x7001] = self_ptr as u8;
    bus.mem[0x7002] = (self_ptr >> 8) as u8;
    bus.mem[0x7003] = 0xCD; // CALL target
    bus.mem[0x7004] = target as u8;
    bus.mem[0x7005] = (target >> 8) as u8;
    bus.mem[0x7006] = 0x76; // HALT
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
    assert!(cpu.halted, "method did not return");
    (cpu.regs.hl(), bus.mem)
}
