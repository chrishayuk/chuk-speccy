//! A **32-bit xorshift RNG** — the SDK `Rng` core, in the dialect. A `u32` state stepped
//! with `^` and constant `<<` / `>>` shifts (one crossing the word boundary), each step
//! truncated to a `u16`. `u32` is a two-slot value computed in the `HL:DE` pair. Same
//! source under rustc and rustz80.
//!
//! Dialect program: [`samples/showcase/rng32.rs`].
//!
//!     cargo run -p rustz80 --example rng32

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/rng32.rs");

fn host_run() -> u16 {
    let mut x: u32 = 2463534242;
    let mut sum = 0u16;
    for _ in 0..8 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        sum = sum.wrapping_add(x as u16);
    }
    sum
}

fn main() {
    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();
    println!("xorshift32 (u32 state) — 8 steps, sum of the low words:");
    println!("  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ a 32-bit RNG ran on the Z80 (u32 in the HL:DE pair)");
}
