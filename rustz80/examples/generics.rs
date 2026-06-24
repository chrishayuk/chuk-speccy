//! **Generic functions** compiled to Z80 by monomorphization. The dialect accepts
//! real generic Rust (`fn max<T: Ord + Copy>(…)`); every call instantiates a
//! width-specialized copy, so the *same* `clamp` serves both `u16` and `u8`. Purely
//! a lowering concern — codegen just sees extra named functions.
//!
//! Dialect program: [`samples/showcase/generic.rs`].
//!
//!     cargo run -p rustz80 --example generics

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/generic.rs");

// The oracle: the identical generic functions in plain Rust.
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
fn host_run() -> u16 {
    let a = clamp(50u16, 10, 40);
    let b = clamp(5u16, 10, 40);
    let u = clamp(200u8, 50, 150);
    a + b + u as u16
}

fn main() {
    // Show which monomorphic instances the compiler generated.
    let prog = rustz80::compile_program(SRC).expect("compile");
    let mut instances: Vec<&String> = prog.symbols.keys().filter(|k| k.contains('$')).collect();
    instances.sort();
    println!(
        "one generic source → {} monomorphic instances:",
        instances.len()
    );
    for sym in &instances {
        println!("    {sym}");
    }

    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();
    println!("\n  clamp(50,10,40)+clamp(5,10,40)+clamp(200u8,50,150)");
    println!("    z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ generics monomorphized + ran on the Z80 (= 200)");
}
