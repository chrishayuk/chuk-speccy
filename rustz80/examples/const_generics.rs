//! **Const generics** — a `const N: usize` parameter sizes a local `[u16; N]` array
//! and bounds the loops; each `triangle::<N>()` call instantiates a specialized copy.
//! The same source compiles under rustc and rustz80.
//!
//! Dialect program: [`samples/showcase/const_generics.rs`].
//!
//!     cargo run -p rustz80 --example const_generics

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/const_generics.rs");

fn triangle<const N: usize>() -> u16 {
    let mut a = [0u16; N];
    for (i, slot) in a.iter_mut().enumerate() {
        *slot = (i + 1) as u16;
    }
    a.iter().sum()
}
fn host_run() -> u16 {
    triangle::<4>() * 100 + triangle::<8>()
}

fn main() {
    // Show the per-size instances the compiler generated.
    let prog = rustz80::compile_program(SRC).expect("compile");
    let mut insts: Vec<&String> = prog.symbols.keys().filter(|k| k.contains('$')).collect();
    insts.sort();
    println!("const-generic instances: {insts:?}");

    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();
    println!("  triangle::<4>()*100 + triangle::<8>()  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ const generics monomorphized + ran on the Z80 (= 1036)");
}
