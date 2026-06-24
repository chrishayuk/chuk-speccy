//! Regression guard for the showcase programs in `samples/showcase/` (the dialect
//! sources that the `examples/` demos run). The examples self-check against a rustc
//! oracle when you `cargo run` them; this locks each result to a known constant so
//! the default `cargo test` catches a codegen regression even if no one runs them.
//!
//! Single source of truth: both this test and the matching `examples/*.rs` pull the
//! dialect program from the same file via `include_str!`, and share one CPU runner.

#[path = "../examples/common/cpu.rs"]
mod cpu;

fn val(src: &str, entry: &str, args: &[u16]) -> u16 {
    cpu::run_value(src, entry, args)
}

#[test]
fn sort_checksum() {
    let src = include_str!("../samples/showcase/sort.rs");
    assert_eq!(val(src, "sort_checksum", &[]), 330);
}

#[test]
fn sieve_count() {
    let src = include_str!("../samples/showcase/sieve.rs");
    assert_eq!(val(src, "count_primes", &[]), 25);
}

#[test]
fn rpn_vm() {
    let src = include_str!("../samples/showcase/rpn.rs");
    assert_eq!(val(src, "eval", &[]), 47); // 6*7 + 5
}

#[test]
fn vending_machine() {
    let src = include_str!("../samples/showcase/vending.rs");
    assert_eq!(val(src, "run", &[]), 220); // 2 vends, 20 credit
}

#[test]
fn lcg_rng() {
    let src = include_str!("../samples/showcase/lcg.rs");
    assert_eq!(val(src, "rng_hash", &[1, 1000]), 1376);
}

#[test]
fn numerics_trio() {
    let src = include_str!("../samples/showcase/numerics.rs");
    assert_eq!(val(src, "gcd", &[1071, 462]), 21);
    assert_eq!(val(src, "isqrt", &[10000]), 100);
    assert_eq!(val(src, "fib", &[23]), 28657);
}

#[test]
fn generics_clamp() {
    let src = include_str!("../samples/showcase/generic.rs");
    assert_eq!(val(src, "run", &[]), 200);
    // One generic source, monomorphized at both u16 and u8.
    let prog = rustz80::compile_program(src).unwrap();
    assert!(prog.symbols.contains_key("clamp$u16"));
    assert!(prog.symbols.contains_key("clamp$u8"));
}

#[test]
fn draw_to_screen() {
    let src = include_str!("../samples/showcase/draw.rs");
    let (_hl, mem) = cpu::run(src, "draw", &[]);
    // Box corners + both diagonals lit.
    let px = |x: u16, y: u16| {
        let a = 0x4000 + ((y & 0xC0) << 5) + ((y & 0x07) << 8) + ((y & 0x38) << 2) + (x >> 3);
        mem[a as usize] & (0x80 >> (x & 7)) != 0
    };
    assert!(px(0, 0) && px(63, 0) && px(0, 63) && px(63, 63));
    for i in 0..64u16 {
        assert!(px(i, i) && px(63 - i, i));
    }
}
