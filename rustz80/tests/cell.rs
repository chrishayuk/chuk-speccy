//! Tests for the `rustz80-cell` micro-VM runner (behind `--features cell`). Without the
//! feature this file compiles to nothing.
#![cfg(feature = "cell")]

use rustz80::cell::{self, Runner, Ty, DEFAULT_CYCLES};

#[test]
fn cell_program_compile_once_instantiate_cheap() {
    use rustz80::cell::{CellConfig, CellProgram};
    // Compile once → a cacheable program; instantiate many cheap runners (no re-parse).
    let prog = CellProgram::compile("fn run(a: u16, b: u16) -> u16 { a + b }").unwrap();
    assert!(prog.program().symbols.contains_key("run"));

    let mut r1 = Runner::new(&prog);
    let mut r2 = Runner::new(&prog); // independent machines from one program
    assert_eq!(r1.run(None, &[20, 22], DEFAULT_CYCLES).unwrap().result, 42);
    assert_eq!(r2.run(None, &[1, 2], DEFAULT_CYCLES).unwrap().result, 3);
    assert_eq!(r1.run(None, &[5, 5], DEFAULT_CYCLES).unwrap().result, 10); // no shared state

    // The policy travels with the compiled program.
    assert!(CellProgram::compile_with_config(
        "fn run() -> u16 { peek(0u16) as u16 }",
        CellConfig::sandboxed()
    )
    .is_err());
}

#[test]
fn state_cell_named_io() {
    use rustz80::cell::StateCell;
    // The agent surface: set named inputs → run → read named outputs, no raw addresses.
    let src = "struct State { x: u16, y: u16, score: u16 }
               impl State { fn run(&mut self) -> u16 { self.score = self.x * self.x + self.y; self.score } }";
    let mut cell = StateCell::bind(src, "State", None).unwrap(); // entry defaults to State::run
    cell.set("x", 6).unwrap();
    cell.set("y", 5).unwrap();
    let rep = cell.run(DEFAULT_CYCLES).unwrap();
    assert_eq!(rep.result, 41); // 6*6 + 5
    assert_eq!(cell.get("score"), Some(41));

    // Reuse with new inputs — no leakage (score re-zeroed by the reset before re-run).
    cell.set("x", 2).unwrap();
    cell.set("y", 3).unwrap();
    cell.run(DEFAULT_CYCLES).unwrap();
    assert_eq!(cell.get("score"), Some(7)); // 2*2 + 3

    // Unknown / non-existent fields.
    assert!(cell.set("nope", 1).is_err());
    assert_eq!(cell.get("nope"), None);
    let mut names: Vec<&str> = cell.fields().collect();
    names.sort();
    assert_eq!(names, ["score", "x", "y"]);
}

#[test]
fn report_json_is_abi_v1() {
    // The frozen v1 report schema: leads with the ABI version, then the documented keys.
    use rustz80::cell::ABI_VERSION;
    assert_eq!(ABI_VERSION, 1);
    let mut r = Runner::compile("fn run(a: u16, b: u16) -> u16 { a * b }").unwrap();
    let json = r.run(None, &[6, 7], DEFAULT_CYCLES).unwrap().to_json();
    assert!(
        json.starts_with(&format!("{{\"abi\":{ABI_VERSION},")),
        "got: {json}"
    );
    for key in [
        "\"entry\":\"run\"",
        "\"result\":42",
        "\"regs\":[42,",
        "\"cycles\":",
        "\"trapped_ops\":",
        "\"budget\":",
        "\"halt\":\"returned\"",
        "\"code_bytes\":",
        "\"functions\":",
        "\"symbols\":{",
        "\"memory_touched\":[",
        "\"reads\":{",
    ] {
        assert!(json.contains(key), "v1 schema missing `{key}` in {json}");
    }
}

#[test]
fn report_counts_trapped_ops() {
    // The honest cost companion to `cycles`: `mul`/`div` traps read as ~free in cycles, so
    // count them so a reward function can't be gamed by routing work through traps.
    let mut r = Runner::compile("fn run(a: u16, b: u16) -> u16 { a * b + a * b + a / b }").unwrap();
    let rep = r.run(None, &[6, 2], DEFAULT_CYCLES).unwrap();
    assert_eq!(rep.result, 27); // 12 + 12 + 3
    assert_eq!(rep.trapped_ops, 3); // two muls + one div
    assert!(rep.to_json().contains("\"trapped_ops\":3"));

    // Pure add/shift code traps nothing.
    let mut add = Runner::compile("fn run(a: u16, b: u16) -> u16 { a + b + a }").unwrap();
    assert_eq!(
        add.run(None, &[6, 2], DEFAULT_CYCLES).unwrap().trapped_ops,
        0
    );

    // The fast (batch) path reports the same count — input-independent for straight-line.
    let many = r
        .run_many_fast(None, &[&[6, 2], &[3, 3]], DEFAULT_CYCLES)
        .unwrap();
    assert!(many.iter().all(|f| f.trapped_ops == 3));
}

#[test]
fn struct_field_state_matches_host() {
    // Closes the B3 seam against the host oracle (not against hardcoded literals): run a
    // struct program through the cell, snapshot EVERY field via `struct_layout`, and assert
    // field-by-field equality with the same logic under rustc. This proves the
    // host-vs-cell *field-state* equality through the layout map — the literal B3 claim —
    // the way `diff.rs` proves it for the `HL` return value.
    let src = "struct State { x: u16, y: u16, sum: u16, prod: u16, big: u16 }
               impl State {
                   fn run(&mut self) -> u16 {
                       self.sum = self.x.wrapping_add(self.y);
                       self.prod = self.x.wrapping_mul(self.y);
                       if self.x > self.y { self.big = self.x; } else { self.big = self.y; }
                       self.sum
                   }
               }";
    // The rustc oracle — the identical logic on a host struct.
    #[derive(Default)]
    struct State {
        x: u16,
        y: u16,
        sum: u16,
        prod: u16,
        big: u16,
    }
    impl State {
        fn run(&mut self) -> u16 {
            self.sum = self.x.wrapping_add(self.y);
            self.prod = self.x.wrapping_mul(self.y);
            if self.x > self.y {
                self.big = self.x;
            } else {
                self.big = self.y;
            }
            self.sum
        }
    }

    const BASE: u16 = 0xB000;
    let layout = rustz80::struct_layout(src, "State").unwrap();
    let addr = |f: &str| BASE + layout.iter().find(|l| l.name == f).unwrap().offset * 2;
    let mut r = Runner::compile(src).unwrap();

    for (x, y) in [
        (3u16, 4u16),
        (40000, 40000),
        (7, 7),
        (0, 9),
        (255, 256),
        (12345, 9999),
    ] {
        // cell: set inputs by name, run, read every field back through the layout map.
        let inputs = vec![
            (addr("x"), Ty::U16, x as u64),
            (addr("y"), Ty::U16, y as u64),
        ];
        let result = r
            .run_with_inputs(Some("State::run"), &[BASE], &inputs, DEFAULT_CYCLES)
            .unwrap()
            .result;
        // host: the same program under rustc.
        let mut host = State {
            x,
            y,
            ..Default::default()
        };
        let host_result = host.run();

        assert_eq!(result, host_result, "return value ({x},{y})");
        for (name, hv) in [
            ("x", host.x),
            ("y", host.y),
            ("sum", host.sum),
            ("prod", host.prod),
            ("big", host.big),
        ] {
            assert_eq!(
                r.peek_u16(addr(name)),
                hv,
                "field `{name}` diverged from host on ({x},{y})"
            );
        }
    }
}

#[test]
fn run_many_fast_matches_single() {
    // The batch path (entry resolved once) agrees with per-call run_fast.
    let mut r = Runner::compile("fn run(x: u16, y: u16) -> u16 { x * x + y }").unwrap();
    let sets: [&[u16]; 3] = [&[3, 1], &[6, 5], &[10, 0]];
    let many = r.run_many_fast(None, &sets, DEFAULT_CYCLES).unwrap();
    assert_eq!(
        many.iter().map(|f| f.result).collect::<Vec<_>>(),
        vec![10, 41, 100]
    );
    for (f, s) in many.iter().zip(sets.iter()) {
        let single = r.run_fast(None, s, DEFAULT_CYCLES).unwrap();
        assert_eq!(
            (f.result, f.cycles, f.halt),
            (single.result, single.cycles, single.halt)
        );
    }
}

#[test]
fn cell_pool_reuses_buses() {
    use rustz80::cell::{CellPool, CellProgram};
    let p1 = CellProgram::compile("fn run(a: u16) -> u16 { a + 1u16 }").unwrap();
    let p2 = CellProgram::compile("fn run(a: u16) -> u16 { a * 2u16 }").unwrap();
    let mut pool = CellPool::new();
    assert_eq!(pool.idle_count(), 0);

    let mut r = pool.acquire(&p1);
    assert_eq!(r.run(None, &[10], DEFAULT_CYCLES).unwrap().result, 11);
    pool.release(r);
    assert_eq!(pool.idle_count(), 1);

    // A different program reuses the pooled bus — no leakage from p1, correct result.
    let mut r = pool.acquire(&p2);
    assert_eq!(pool.idle_count(), 0); // the idle bus was taken, not a new alloc
    assert_eq!(r.run(None, &[10], DEFAULT_CYCLES).unwrap().result, 20);
    pool.release(r);
    assert_eq!(pool.idle_count(), 1);

    // Two concurrent cells → pool grows to the high-water mark of 2.
    let a = pool.acquire(&p1);
    let b = pool.acquire(&p2);
    pool.release(a);
    pool.release(b);
    assert_eq!(pool.idle_count(), 2);
}

#[test]
fn run_many_fast_fast_path_matches_authentic() {
    // A straight-line cell exercising much of the fast executor — mul/div/rem traps, the
    // 8-bit bitwise path, const shift-add — must match the authentic interpreter exactly
    // (result + cycles + halt) on every input. This is the differential guard on the fast
    // engine: any divergence fails here.
    let mut r = Runner::compile(
        "fn run(a: u16, b: u16) -> u16 { (a * b + a / b) % 100u16 + (a & b) + a * 3u16 }",
    )
    .unwrap();
    let sets: [&[u16]; 5] = [&[60, 7], &[1000, 3], &[7, 7], &[40000, 123], &[3, 9]];
    let many = r.run_many_fast(None, &sets, DEFAULT_CYCLES).unwrap();
    for (f, s) in many.iter().zip(sets.iter()) {
        let auth = r.run_fast(None, s, DEFAULT_CYCLES).unwrap();
        assert_eq!(
            (f.result, f.cycles, f.halt),
            (auth.result, auth.cycles, auth.halt),
            "fast vs authentic diverged on {s:?}"
        );
    }
}

#[test]
fn fast_executor_matches_authentic_across_ops() {
    // Drive a spread of straight-line cells through the fast path and assert each matches
    // the authentic interpreter (result + cycles + halt) — covering and validating the
    // executor's opcode arms: traps, bitwise, const-mul, array indexing (HL loads + INC),
    // tuples (BC), and raw memory.
    let cells = [
        "fn run(a: u16, b: u16) -> u16 { a * b + a / b + a % b }",
        "fn run(a: u16, b: u16) -> u16 { (a & b) + (a | b) + (a ^ b) }",
        "fn run(a: u16, b: u16) -> u16 { a * 7u16 + b * 3u16 }",
        "fn run(i: u16, a: u16, b: u16) -> u16 { let arr = [a, b, a + b]; arr[i as usize] }",
        "fn run(a: u16, b: u16) -> (u16, u16, u16) { (a * b, a + b, a) }",
        "fn run(a: u16, b: u16) -> u16 { let arr = [a; 4]; arr[0] + b }", // [v; N] fill → fallback
        "fn run(a: u16, b: u16) -> u16 { halt(a); b }",                   // halt trap → fallback
    ];
    // last input has b = 0 → exercises the divide-by-zero arm (both engines agree).
    let inputs: [&[u16]; 5] = [
        &[2, 3, 5],
        &[60, 4, 9],
        &[1, 1000, 7],
        &[2, 40000, 255],
        &[5, 0, 2],
    ];
    for src in cells {
        let mut r = Runner::compile(src).unwrap();
        let many = r.run_many_fast(None, &inputs, DEFAULT_CYCLES).unwrap();
        for (f, inp) in many.iter().zip(inputs.iter()) {
            let auth = r.run_fast(None, inp, DEFAULT_CYCLES).unwrap();
            assert_eq!(
                (f.result, f.cycles, f.halt),
                (auth.result, auth.cycles, auth.halt),
                "fast vs authentic diverged: `{src}` on {inp:?}"
            );
        }
    }
}

#[test]
fn run_many_fast_falls_back_for_branches() {
    // A looping cell is not straight-line → run_many_fast transparently falls back to the
    // authentic interpreter, still correct per input.
    let mut r = Runner::compile(
        "fn run(n: u16) -> u16 {
             let mut s = 0u16; let mut i = 0u16;
             while i < n { s = s + i; i = i + 1u16; } s
         }",
    )
    .unwrap();
    let sets: [&[u16]; 3] = [&[0], &[5], &[100]];
    let many = r.run_many_fast(None, &sets, DEFAULT_CYCLES).unwrap();
    for (f, s) in many.iter().zip(sets.iter()) {
        let auth = r.run_fast(None, s, DEFAULT_CYCLES).unwrap();
        assert_eq!(
            (f.result, f.cycles, f.halt),
            (auth.result, auth.cycles, auth.halt)
        );
    }
    assert_eq!(many[1].result, 10); // 0+1+2+3+4
    assert_eq!(many[2].result, 4950); // sum 0..100
}

#[test]
fn cartridge_roundtrip_and_inspect() {
    use rustz80::cell::{Cartridge, CartridgeOpts, CellConfig, ABI_VERSION};
    let src = "fn run(a: u16, b: u16) -> u16 { a * b }";
    let cart = Cartridge::compile(
        src,
        CellConfig::sandboxed(),
        CartridgeOpts {
            id: Some("mul.v1".into()),
            summary: "product".into(),
            tags: vec!["math".into(), "demo".into()],
            entry: None, // resolves to `run`
        },
    )
    .unwrap();

    // Round-trip through bytes: manifest + program survive, and it still runs.
    let bytes = cart.to_bytes();
    let back = Cartridge::from_bytes(&bytes).unwrap();
    assert_eq!(back.manifest, cart.manifest);
    assert_eq!(back.manifest.id, "mul.v1");
    assert_eq!(back.manifest.entry, "run");
    assert_eq!(back.manifest.abi_version, ABI_VERSION);
    assert!(!back.manifest.compiler_version.is_empty());
    assert_eq!(
        Runner::new(&back.program)
            .run(None, &[6, 7], DEFAULT_CYCLES)
            .unwrap()
            .result,
        42
    );

    // Inspection surfaces the manifest for a tool index.
    let j = back.to_json();
    for key in [
        "\"id\":\"mul.v1\"",
        "\"entry\":\"run\"",
        "\"tags\":[\"math\",\"demo\"]",
        "\"abi\":1",
    ] {
        assert!(j.contains(key), "inspect json missing {key}: {j}");
    }
    assert!(back.to_human().contains("mul.v1"));

    // Foreign / truncated bytes are rejected, not panicked; bad entry errors.
    assert!(Cartridge::from_bytes(b"nope!!").is_err());
    assert!(Cartridge::from_bytes(&bytes[..bytes.len() - 4]).is_err());
    assert!(Cartridge::compile(
        src,
        CellConfig::sandboxed(),
        CartridgeOpts {
            entry: Some("missing".into()),
            ..Default::default()
        }
    )
    .is_err());
}

#[test]
fn cartridge_carries_typed_signature() {
    use rustz80::cell::{Cartridge, CartridgeOpts, CellConfig};
    // fn-args signature, surviving the round-trip + surfaced in inspect.
    let c = Cartridge::compile(
        "fn run(a: u16, b: u16) -> u16 { a + b }",
        CellConfig::sandboxed(),
        CartridgeOpts::default(),
    )
    .unwrap();
    assert_eq!(
        c.manifest.signature.params,
        vec![("a".into(), "u16".into()), ("b".into(), "u16".into())]
    );
    assert_eq!(c.manifest.signature.ret, "u16");
    assert!(c.manifest.signature.state.is_empty());
    let back = Cartridge::from_bytes(&c.to_bytes()).unwrap();
    assert_eq!(back.manifest, c.manifest); // signature round-trips
    assert!(back
        .to_human()
        .contains("signature: run(a: u16, b: u16) -> u16"));
    assert!(back.to_json().contains(
        "\"signature\":{\"params\":[[\"a\",\"u16\"],[\"b\",\"u16\"]],\"ret\":\"u16\",\"state\":[]}"
    ));

    // `&mut self` method → the named typed state (struct fields with types).
    let src = "struct State { x: u16, y: u16, score: u16 }
               impl State { fn run(&mut self) -> u16 { self.score = self.x + self.y; self.score } }";
    let s = Cartridge::compile(
        src,
        CellConfig::sandboxed(),
        CartridgeOpts {
            entry: Some("State::run".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.manifest.signature.params.is_empty());
    assert_eq!(
        s.manifest.signature.state,
        vec![
            ("x".into(), "u16".into()),
            ("y".into(), "u16".into()),
            ("score".into(), "u16".into())
        ]
    );
    assert!(Cartridge::from_bytes(&s.to_bytes())
        .unwrap()
        .to_human()
        .contains("state: { x: u16, y: u16, score: u16 }"));
}

#[test]
fn cartridge_permissive_and_empty_manifest_branches() {
    use rustz80::cell::{Cartridge, CartridgeOpts, CellConfig};
    // Permissive (no ceilings → ∞/null) + empty summary/tags (→ "(no summary)" / "—").
    let cart = Cartridge::compile(
        "fn run() -> u16 { 0u16 }",
        CellConfig::permissive(),
        CartridgeOpts::default(),
    )
    .unwrap();
    let human = cart.to_human();
    assert!(
        human.contains("(no summary)") && human.contains("tags: —"),
        "got: {human}"
    );
    assert!(human.contains("max_code=∞") && human.contains("max_touched=∞"));
    assert!(cart
        .to_json()
        .contains("\"max_code\":null,\"max_touched\":null"));
    assert_eq!(cart.manifest.id, "run"); // defaulted to the entry name

    // Neither `run` nor `main`, no explicit entry → error.
    assert!(Cartridge::compile(
        "fn helper() -> u16 { 1u16 }",
        CellConfig::permissive(),
        CartridgeOpts::default()
    )
    .is_err());
}

#[test]
fn cli_compile_inspect_and_errors() {
    let dir = std::env::temp_dir().join("rustz80_cart_test");
    std::fs::create_dir_all(&dir).unwrap();
    let rs = dir.join("c.rs");
    let cellfile = dir.join("c.cell");
    std::fs::write(
        &rs,
        "fn run(a: u16, b: u16) -> u16 { poke(40000u16, a as u8); a + b }",
    )
    .unwrap();
    let (rs, cellfile) = (
        rs.to_str().unwrap().to_string(),
        cellfile.to_str().unwrap().to_string(),
    );

    // compile exercising most options (poke needs --allow-raw-memory).
    let out = cell::run_cli(&[
        "compile".into(),
        rs.clone(),
        "-o".into(),
        cellfile.clone(),
        "--entry".into(),
        "run".into(),
        "--id".into(),
        "add.v1".into(),
        "--summary".into(),
        "adds two".into(),
        "--tags".into(),
        "math, demo".into(),
        "--allow-raw-memory".into(),
        "--max-touched".into(),
        "256".into(),
    ])
    .unwrap();
    assert!(
        out.contains("wrote") && out.contains("add.v1"),
        "got: {out}"
    );

    // inspect: human (no --json) and json.
    assert!(cell::run_cli(&["inspect".into(), cellfile.clone()])
        .unwrap()
        .contains("add.v1"));
    assert!(
        cell::run_cli(&["inspect".into(), cellfile, "--json".into()])
            .unwrap()
            .contains("\"tags\":[\"math\",\"demo\"]")
    );

    // error paths: unknown command, no args, compile without -o, unknown option, missing file.
    assert!(cell::run_cli(&["frobnicate".into()]).is_err());
    assert!(cell::run_cli(&[]).is_err());
    assert!(cell::run_cli(&["compile".into(), rs.clone()]).is_err());
    assert!(cell::run_cli(&[
        "compile".into(),
        rs,
        "-o".into(),
        "/x".into(),
        "--bogus".into()
    ])
    .is_err());
    assert!(cell::run_cli(&["inspect".into(), "/no/such.cell".into()]).is_err());
}

#[test]
fn cell_image_roundtrip() {
    use rustz80::cell::{CellConfig, CellProgram};
    let src = "fn run(a: u16, b: u16) -> u16 { a * b }";
    let prog = CellProgram::compile_with_config(src, CellConfig::sandboxed()).unwrap();
    let bytes = prog.to_bytes();
    assert!(
        bytes.len() < 128,
        "image should be tiny (got {})",
        bytes.len()
    );

    // Reload without re-parsing — same code + symbols, runs to the same result, policy kept.
    let back = CellProgram::from_bytes(&bytes).unwrap();
    assert_eq!(back.program().code, prog.program().code);
    assert_eq!(back.program().symbols, prog.program().symbols);
    assert_eq!(
        Runner::new(&back)
            .run(None, &[6, 7], DEFAULT_CYCLES)
            .unwrap()
            .result,
        42
    );

    // Foreign / truncated bytes are rejected, not panicked.
    assert!(CellProgram::from_bytes(b"nope").is_err());
    assert!(CellProgram::from_bytes(&bytes[..bytes.len() - 3]).is_err());
}

#[test]
fn cell80_halt_with_code() {
    use rustz80::cell::Halt;
    // `halt(code)` stops the run early with a status code.
    let mut r = Runner::compile(
        "fn run(n: u16) -> u16 {
             let mut i = 0u16;
             while i < 1000u16 { if i == n { halt(7u16); } i = i + 1u16; }
             0u16
         }",
    )
    .unwrap();
    let early = r.run(None, &[5], DEFAULT_CYCLES).unwrap();
    assert_eq!(early.halt, Halt::Halted(7));
    assert!(!early.returned);

    // n never hit → the loop completes and returns normally.
    let full = r.run(None, &[2000], DEFAULT_CYCLES).unwrap();
    assert_eq!(full.halt, Halt::Returned);
    assert_eq!(full.result, 0);
    assert!(early.cycles < full.cycles, "halt(5) should stop far sooner");

    // `halt` compiles for the authentic Spectrum target too (a no-op `ED FE` there).
    assert!(rustz80::compile_program("fn run() -> u16 { halt(1u16); 0u16 }").is_ok());
}

#[test]
fn cell80_array_init_is_a_block_op() {
    use rustz80::cell::{CellProgram, Runner};
    // A big `[v; N]` init is one block op, not N unrolled stores — so the code stays tiny
    // (it would be ~hundreds of bytes unrolled). Result still correct.
    let src = "fn run() -> u16 { let a = [9u16; 256]; a[0] + a[255] }";
    let cp = CellProgram::compile(src).unwrap();
    assert!(
        cp.program().code.len() < 64,
        "256-element fill should not unroll (got {} bytes)",
        cp.program().code.len()
    );
    assert_eq!(
        Runner::new(&cp)
            .run(None, &[], DEFAULT_CYCLES)
            .unwrap()
            .result,
        18
    ); // 9 + 9
}

#[test]
fn cell80_traps_mul_div_natively() {
    use rustz80::cell::{CellProgram, Runner};
    let src = "fn run(a: u16, b: u16) -> u16 { a * b + a / b + a % b }";

    // Cell mode: `*`/`/`/`%` lower to ED FE host traps — no software runtime appended.
    let cp = CellProgram::compile(src).unwrap();
    assert!(
        !cp.program().symbols.contains_key("__mul16"),
        "cell mode shouldn't append __mul16"
    );
    assert!(!cp.program().symbols.contains_key("__divmod16"));
    let got = Runner::new(&cp)
        .run(None, &[60, 7], DEFAULT_CYCLES)
        .unwrap()
        .result;
    assert_eq!(got, 60u16 * 7 + 60 / 7 + 60 % 7); // 420 + 8 + 4 = 432 (matches rustc)

    // Authentic Spectrum compile still uses (and appends) the software routines.
    let spec = rustz80::compile_program(src).unwrap();
    assert!(spec.symbols.contains_key("__mul16") && spec.symbols.contains_key("__divmod16"));
}

#[test]
fn run_fast_matches_run() {
    use rustz80::cell::Halt;
    // The hot path must agree with the full Report on result/regs/cycles/halt.
    let mut r = Runner::compile("fn run(a: u16, b: u16) -> (u16, u16) { (a * a + b, a) }").unwrap();
    let full = r.run(None, &[6, 5], DEFAULT_CYCLES).unwrap();
    let fast = r.run_fast(None, &[6, 5], DEFAULT_CYCLES).unwrap();
    assert_eq!(fast.result, full.result); // 6*6 + 5 = 41
    assert_eq!(fast.regs, full.regs);
    assert_eq!(fast.cycles, full.cycles);
    assert_eq!(fast.halt, full.halt);
    assert_eq!(fast.halt, Halt::Returned);

    // Budget overrun is reported, not hung, on the fast path too.
    let mut spin =
        Runner::compile("fn run() -> u16 { let mut i = 0u16; loop { i = i + 1u16; } }").unwrap();
    assert_eq!(
        spin.run_fast(None, &[], 1000).unwrap().halt,
        Halt::CycleBudget
    );
}

#[test]
fn captures_all_result_registers() {
    // A tuple return leaves the values in HL/DE/BC — read them all back.
    let mut r = Runner::compile("fn run(a: u16, b: u16) -> (u16, u16) { (a / b, a % b) }").unwrap();
    let rep = r.run(None, &[47, 5], DEFAULT_CYCLES).unwrap();
    assert_eq!(rep.result, 9); // HL = quotient
    assert_eq!(rep.regs[0], 9); // HL
    assert_eq!(rep.regs[1], 2); // DE = remainder
}

#[test]
fn typed_state_read_back() {
    // A program that writes known bytes; read them back typed from post-run memory.
    let mut r = Runner::compile(
        "fn run() -> u16 {
             poke(40000u16, 0x34u8); poke(40001u16, 0x12u8);  // u16 0x1234 @ 40000
             poke(40002u16, 0x78u8); poke(40003u16, 0x56u8);  // u32 high word
             0u16
         }",
    )
    .unwrap();
    r.run(None, &[], DEFAULT_CYCLES).unwrap();
    assert_eq!(r.peek_u8(40000), 0x34);
    assert_eq!(r.peek_u16(40000), 0x1234);
    assert_eq!(r.peek_u32(40000), 0x5678_1234);
    let vals = r.read_named(&[
        ("a".into(), 40000, Ty::U16),
        ("b".into(), 40000, Ty::U32),
        ("c".into(), 40003, Ty::U8),
    ]);
    assert_eq!(
        vals,
        vec![
            ("a".into(), 0x1234u64),
            ("b".into(), 0x5678_1234u64),
            ("c".into(), 0x56u64),
        ]
    );
}

#[test]
fn struct_layout_offsets() {
    let src = "struct State { x: u16, y: u16, arr: [u16; 4], score: u16 }";
    let l = rustz80::struct_layout(src, "State").unwrap();
    assert_eq!(
        l[0],
        rustz80::FieldLayout {
            name: "x".into(),
            offset: 0,
            slots: 1
        }
    );
    assert_eq!(l[1].offset, 1); // y
    assert_eq!((l[2].offset, l[2].slots), (2, 4)); // arr — 4 slots
    assert_eq!(l[3].offset, 6); // score, after the array
    assert!(rustz80::struct_layout(src, "Nope").is_err());
}

#[test]
fn typed_io_named_loop() {
    // The full agent loop by NAME: resolve field addresses from the layout, set typed
    // inputs, run, read typed outputs — the caller never touches raw addresses directly.
    let src = "struct State { x: u16, y: u16, score: u16 }
               impl State { fn run(&mut self) -> u16 { self.score = self.x + self.y * 10u16; self.score } }";
    const BASE: u16 = 0xB000;
    let layout = rustz80::struct_layout(src, "State").unwrap();
    let addr = |f: &str| BASE + layout.iter().find(|l| l.name == f).unwrap().offset * 2;

    let mut r = Runner::compile(src).unwrap();
    let inputs = vec![(addr("x"), Ty::U16, 3u64), (addr("y"), Ty::U16, 4u64)];
    let rep = r
        .run_with_inputs(Some("State::run"), &[BASE], &inputs, DEFAULT_CYCLES)
        .unwrap();
    assert_eq!(rep.result, 43); // 3 + 4*10
    let out = r.read_named(&[("score".into(), addr("score"), Ty::U16)]);
    assert_eq!(out, vec![("score".into(), 43u64)]);

    // Different inputs, same compiled cell (warm) — no leakage from the prior run.
    let rep2 = r
        .run_with_inputs(
            Some("State::run"),
            &[BASE],
            &[(addr("x"), Ty::U16, 100), (addr("y"), Ty::U16, 0)],
            DEFAULT_CYCLES,
        )
        .unwrap();
    assert_eq!(rep2.result, 100);
}

#[test]
fn ty_parse() {
    assert_eq!(Ty::parse("u8").unwrap(), Ty::U8);
    assert_eq!(Ty::parse("u16").unwrap(), Ty::U16);
    assert_eq!(Ty::parse("u32").unwrap(), Ty::U32);
    assert!(Ty::parse("u9").is_err());
}

#[test]
fn runner_reuse_is_deterministic() {
    // Compile once, run many: each run must reset the bus, so repeated runs (same args)
    // are bit-identical — same result, same T-states, same touched memory — and changing
    // args changes the result, with no leakage between runs.
    let mut r = Runner::compile(
        "fn run(n: u16) -> u16 { let mut a = [0u16; 8]; let mut s = 0u16;
             let mut i = 0u16; while i < 8u16 { a[i as usize] = i + n; i = i + 1u16; }
             let mut j = 0u16; while j < 8u16 { s = s + a[j as usize]; j = j + 1u16; } s }",
    )
    .expect("compile");

    assert!(r.program().symbols.contains_key("run")); // the compiled program is reachable
    let first = r.run(None, &[0], DEFAULT_CYCLES).unwrap(); // 0+1+..+7 = 28
    let again = r.run(None, &[0], DEFAULT_CYCLES).unwrap();
    assert_eq!(first.result, 28);
    assert_eq!(first.result, again.result, "reuse must be deterministic");
    assert_eq!(first.cycles, again.cycles, "same path → same T-states");
    assert_eq!(
        first.touched, again.touched,
        "same writes → same memory diff"
    );

    let bumped = r.run(None, &[10], DEFAULT_CYCLES).unwrap(); // (0..7)+8*10 = 28+80 = 108
    assert_eq!(bumped.result, 108);
    // Back to the original args still gives the original answer (no accumulated state).
    assert_eq!(r.run(None, &[0], DEFAULT_CYCLES).unwrap().result, 28);
}

#[test]
fn runs_and_reports() {
    // A small program: sum 1..=n. Run with an arg, check result/cost/symbols/memory.
    let src = "
        fn run(n: u16) -> u16 {
            let mut s = 0u16;
            let mut i = 1u16;
            while i <= n { s = s + i; i = i + 1u16; }
            s
        }
    ";
    let r = cell::run(src, None, &[10], DEFAULT_CYCLES).expect("run");
    assert_eq!(r.entry, "run"); // defaulted to `run`
    assert_eq!(r.entry_addr, rustz80::ORG);
    assert_eq!(r.result, 55); // 1+..+10
    assert!(r.returned, "should return within budget");
    assert!(r.cycles > 0 && r.cycles < DEFAULT_CYCLES);
    assert!(r.code_bytes > 0 && r.fn_count >= 1);
    assert!(r
        .symbols
        .iter()
        .any(|(n, a)| n == "run" && *a == rustz80::ORG));
    // The loop counter/accumulator live in the scratch "register file"; some RAM is hit.
    assert!(!r.touched.is_empty());
}

#[test]
fn budget_exceeded_is_reported_not_panicked() {
    // An infinite loop must stop at the budget and report `returned = false`.
    let src = "fn run() -> u16 { let mut i = 0u16; loop { i = i + 1u16; } }";
    let r = cell::run(src, None, &[], 1000).expect("run");
    assert!(!r.returned, "infinite loop should hit the budget");
    assert!(r.cycles >= 1000);
}

#[test]
fn monomorphic_instances_appear_in_symbols() {
    // Two capacities → two instances in the symbol map.
    let src = include_str!("../samples/showcase/entities.rs");
    let r = cell::run(src, None, &[], DEFAULT_CYCLES).expect("run");
    assert_eq!(r.result, 2530);
    assert!(r.symbols.iter().any(|(n, _)| n == "Entities$4::add"));
    assert!(r.symbols.iter().any(|(n, _)| n == "Entities$8::add"));
}

#[test]
fn missing_entry_errors_with_available_names() {
    let src = "fn run() -> u16 { 1u16 }";
    let err = cell::run(src, Some("nope"), &[], DEFAULT_CYCLES).unwrap_err();
    assert!(err.contains("nope") && err.contains("run"), "got: {err}");
}

#[test]
fn parse_args_decimal_and_hex() {
    assert_eq!(cell::parse_args("1,0x10,255").unwrap(), vec![1, 16, 255]);
    assert_eq!(cell::parse_args("").unwrap(), Vec::<u16>::new());
    assert!(cell::parse_args("notanum").is_err());
}

#[test]
fn report_formats_human_and_json() {
    let r = cell::run("fn run() -> u16 { 7u16 }", None, &[], DEFAULT_CYCLES).unwrap();
    let human = r.to_human();
    assert!(human.contains("result     7") && human.contains("returned"));
    let json = r.to_json();
    assert!(
        json.starts_with('{')
            && json.contains("\"result\":7")
            && json.contains("\"halt\":\"returned\"")
    );
}

#[test]
fn run_cli_end_to_end() {
    // Write a source to a temp file and drive the full CLI path (run → format).
    let dir = std::env::temp_dir().join("rustz80_cell_cli_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("prog.rs");
    std::fs::write(&path, "fn run(a: u16, b: u16) -> u16 { a + b }").unwrap();
    let p = path.to_str().unwrap().to_string();

    // --json with args
    let out = cell::run_cli(&[
        "run".into(),
        p.clone(),
        "--args".into(),
        "20,22".into(),
        "--json".into(),
    ])
    .unwrap();
    assert!(out.contains("\"result\":42"), "got: {out}");

    // human form, default budget
    let out = cell::run_cli(&["run".into(), p.clone(), "--args".into(), "1,2".into()]).unwrap();
    assert!(out.contains("result     3"));

    // a tiny budget reports overshoot rather than hanging/panicking
    let loopsrc = dir.join("loop.rs");
    std::fs::write(
        &loopsrc,
        "fn run() -> u16 { let mut i = 0u16; loop { i = i + 1u16; } }",
    )
    .unwrap();
    let out = cell::run_cli(&[
        "run".into(),
        loopsrc.to_str().unwrap().into(),
        "--cycles".into(),
        "500".into(),
    ])
    .unwrap();
    assert!(out.contains("BUDGET EXCEEDED"));

    // error paths
    assert!(cell::run_cli(&[]).is_err());
    assert!(cell::run_cli(&["wat".into()]).is_err());
    assert!(cell::run_cli(&["run".into(), p, "--bogus".into()]).is_err());
    assert!(cell::run_cli(&["run".into(), "/no/such/file.rs".into()]).is_err());
}

#[test]
fn run_cli_typed_read() {
    let dir = std::env::temp_dir().join("rustz80_cell_read_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.rs");
    std::fs::write(
        &path,
        "fn run() -> u16 { poke(40000u16, 42u8); poke(40001u16, 7u8); 0u16 }",
    )
    .unwrap();
    let p = path.to_str().unwrap().to_string();

    // This cell uses `poke`, so it needs --allow-raw-memory (sandboxed by default).
    let out = cell::run_cli(&[
        "run".into(),
        p.clone(),
        "--allow-raw-memory".into(),
        "--set".into(),
        "0x9c42:u16=0x00ff,40004:u16=9".into(), // hex addr/value AND decimal — both parse paths
        "--read".into(),
        "score@40000:u8,lives@0x9c41:u8,extra@0x9c42:u16,dec@40004:u16".into(), // 0x9c41 = 40001
        "--json".into(),
    ])
    .unwrap();
    assert!(
        out.contains("\"reads\":{\"score\":42,\"lives\":7,\"extra\":255,\"dec\":9}"),
        "got: {out}"
    );

    let human = cell::run_cli(&[
        "run".into(),
        p.clone(),
        "--allow-raw-memory".into(),
        "--read".into(),
        "score@40000:u8".into(),
    ])
    .unwrap();
    assert!(human.contains("reads      score=42"), "got: {human}");

    // bad specs
    assert!(cell::run_cli(&[
        "run".into(),
        p.clone(),
        "--allow-raw-memory".into(),
        "--read".into(),
        "noaddr".into()
    ])
    .is_err());
    assert!(cell::run_cli(&[
        "run".into(),
        p,
        "--allow-raw-memory".into(),
        "--read".into(),
        "x@40000:u9".into()
    ])
    .is_err());
}

#[test]
fn capabilities_gate_raw_memory_and_ports() {
    use rustz80::cell::CellConfig;
    // `poke`/`peek` need raw memory; `inport` needs ports — denied by default.
    let pokes = "fn run() -> u16 { poke(40000u16, 1u8); peek(40000u16) as u16 }";
    let ports = "fn run() -> u16 { inport(0xFEu16) as u16 }";
    assert!(Runner::compile_with_config(pokes, CellConfig::sandboxed()).is_err());
    assert!(Runner::compile_with_config(ports, CellConfig::sandboxed()).is_err());
    // Explicitly allowed → compiles.
    assert!(Runner::compile_with_config(pokes, CellConfig::permissive()).is_ok());
    let mut cfg = CellConfig::sandboxed();
    cfg.allow_ports = true;
    assert!(Runner::compile_with_config(ports, cfg).is_ok());
    // A pure-compute cell needs no capabilities — fine sandboxed.
    assert!(Runner::compile_with_config(
        "fn run(a: u16) -> u16 { a * 2u16 }",
        CellConfig::sandboxed()
    )
    .is_ok());
}

#[test]
fn safety_config_defaults_and_cli_flags() {
    use rustz80::cell::{CellConfig, Halt};
    // default() is the sandboxed policy.
    let d = CellConfig::default();
    assert!(!d.allow_raw_memory && !d.allow_ports && d.max_code_bytes.is_some());

    // A memory-limit run formats in both modes.
    let mut cfg = CellConfig::sandboxed();
    cfg.max_touched = Some(2);
    let mut r = Runner::compile_with_config(
        "fn run() -> u16 { let mut a = [0u16; 32]; let mut i = 0u16;
             while i < 32u16 { a[i as usize] = i; i = i + 1u16; } a[0] }",
        cfg,
    )
    .unwrap();
    let rep = r.run(None, &[], DEFAULT_CYCLES).unwrap();
    assert_eq!(rep.halt, Halt::MemoryLimit);
    assert!(rep.to_human().contains("MEMORY LIMIT"));
    assert!(rep.to_json().contains("\"halt\":\"memory_limit\""));

    // CLI safety flags parse + apply.
    let dir = std::env::temp_dir().join("rustz80_cell_safety_cli");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ok.rs");
    std::fs::write(&path, "fn run(a: u16) -> u16 { a + 1u16 }").unwrap();
    let p = path.to_str().unwrap().to_string();
    let out = cell::run_cli(&[
        "run".into(),
        p.clone(),
        "--max-code-bytes".into(),
        "8192".into(),
        "--max-touched".into(),
        "8192".into(),
        "--allow-ports".into(),
        "--args".into(),
        "41".into(),
    ])
    .unwrap();
    assert!(out.contains("result     42"), "got: {out}");
    // A too-tight code-size limit rejects.
    assert!(cell::run_cli(&["run".into(), p, "--max-code-bytes".into(), "2".into()]).is_err());
}

#[test]
fn limits_code_size_and_memory() {
    use rustz80::cell::{CellConfig, Halt};
    // A tiny code-size ceiling rejects at compile.
    let mut cfg = CellConfig::sandboxed();
    cfg.max_code_bytes = Some(4);
    assert!(Runner::compile_with_config("fn run() -> u16 { let mut s = 0u16; let mut i = 0u16; while i < 100u16 { s = s + i; i = i + 1u16; } s }", cfg).is_err());

    // A memory-touched ceiling aborts the run with Halt::MemoryLimit.
    let mut cfg = CellConfig::sandboxed();
    cfg.max_touched = Some(4);
    let mut r = Runner::compile_with_config(
        "fn run() -> u16 { let mut a = [0u16; 64]; let mut i = 0u16;
             while i < 64u16 { a[i as usize] = i; i = i + 1u16; } a[0] }",
        cfg,
    )
    .unwrap();
    let rep = r.run(None, &[], DEFAULT_CYCLES).unwrap();
    assert_eq!(rep.halt, Halt::MemoryLimit);
    assert!(!rep.returned);
}

#[test]
fn run_cli_typed_set() {
    let dir = std::env::temp_dir().join("rustz80_cell_set_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.rs");
    std::fs::write(
        &path,
        "struct State { x: u16, y: u16, score: u16 }
         impl State { fn run(&mut self) -> u16 { self.score = self.x + self.y; self.score } }",
    )
    .unwrap();
    let p = path.to_str().unwrap().to_string();

    let out = cell::run_cli(&[
        "run".into(),
        p.clone(),
        "--entry".into(),
        "State::run".into(),
        "--args".into(),
        "0xB000".into(),
        "--set".into(),
        "0xB000:u16=20,0xB002:u16=22".into(),
        "--read".into(),
        "score@0xB004:u16".into(),
        "--json".into(),
    ])
    .unwrap();
    assert!(
        out.contains("\"result\":42") && out.contains("\"score\":42"),
        "got: {out}"
    );

    // bad --set specs
    assert!(cell::run_cli(&["run".into(), p.clone(), "--set".into(), "noeq".into()]).is_err());
    assert!(cell::run_cli(&["run".into(), p, "--set".into(), "0xB000:u9=1".into()]).is_err());
}
