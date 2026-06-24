//! **Insertion sort**, written in the `rustz80` dialect, compiled to Z80, and run on
//! the real CPU. Shows: arrays, nested loops, `break` to end the inner shift early,
//! `for` over a range, and order-sensitive indexing.
//!
//! The dialect program lives in [`samples/showcase/sort.rs`]; this harness compiles
//! it, runs it on the `z80` CPU, and checks the result against plain rustc.
//!
//!     cargo run -p rustz80 --example sorting

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/sort.rs");

/// The same algorithm in plain Rust — the oracle.
fn host_sort_checksum() -> u16 {
    let mut a = [5u16, 2, 8, 1, 9, 3, 7, 4, 6, 0];
    a.sort_unstable();
    a.iter().enumerate().map(|(k, &v)| v * (k as u16 + 1)).sum()
}

fn main() {
    let got = cpu::run_value(SRC, "sort_checksum", &[]);
    let want = host_sort_checksum();

    // Sorted 0..=9, the checksum is sum_{k=0}^{9} k*(k+1) = 330.
    println!("insertion sort of [5,2,8,1,9,3,7,4,6,0]");
    println!("  order-checksum  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ sorted correctly on the Z80");
}
