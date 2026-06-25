//! Tests for the `rustz80-cell` micro-VM runner (behind `--features cell`). Without the
//! feature this file compiles to nothing.
#![cfg(feature = "cell")]

use rustz80::cell::{self, Runner, DEFAULT_CYCLES};

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
