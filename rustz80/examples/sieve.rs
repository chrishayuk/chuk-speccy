//! **Sieve of Eratosthenes** — count the primes below 100 on the Z80. Shows: a `[u8;
//! N]` byte array used as a flag table, nested `while` loops, and `for`.
//!
//! Dialect program: [`samples/showcase/sieve.rs`].
//!
//!     cargo run -p rustz80 --example sieve

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/sieve.rs");

fn host_count_primes() -> u16 {
    (2u16..100).filter(|&n| (2..n).all(|d| n % d != 0)).count() as u16
}

fn main() {
    let got = cpu::run_value(SRC, "count_primes", &[]);
    let want = host_count_primes();

    println!("primes below 100");
    println!("  count  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ the sieve agrees (there are 25)");
}
