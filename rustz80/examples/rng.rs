//! A 16-bit **linear-congruential PRNG** driven on the Z80. Shows: `wrapping_mul` /
//! `wrapping_add` (mod-2^16 arithmetic via the `__mul16` micro-runtime) and `^`,
//! plus passing arguments (seed, count) in `HL`/`DE`.
//!
//! Dialect program: [`samples/showcase/lcg.rs`].
//!
//!     cargo run -p rustz80 --example rng

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/lcg.rs");

fn host_rng_hash(seed: u16, n: u16) -> u16 {
    let mut state = seed;
    let mut acc = 0u16;
    for _ in 0..n {
        state = state.wrapping_mul(25173).wrapping_add(13849);
        acc ^= state;
    }
    acc
}

fn main() {
    let (seed, n) = (1u16, 1000u16);
    let got = cpu::run_value(SRC, "rng_hash", &[seed, n]);
    let want = host_rng_hash(seed, n);

    // Show the first few raw outputs for flavour.
    let mut s = seed;
    print!("LCG stream (seed {seed}):");
    for _ in 0..6 {
        s = s.wrapping_mul(25173).wrapping_add(13849);
        print!(" {s}");
    }
    println!(" …");
    println!("  XOR-hash of {n} outputs  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ {n} wrapping multiplies agree with rustc");
}
