//! **Multiple return values** via tuples. A tuple return lands in the `HL`/`DE`/`BC`
//! registers (up to three values at once), destructured at the call site. We call
//! `divmod` and `stats` directly and read the result registers back, then run the
//! packed `run` and check it against rustc.
//!
//! Dialect program: [`samples/showcase/tuples.rs`].
//!
//!     cargo run -p rustz80 --example tuples

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/tuples.rs");

fn host_divmod(a: u16, b: u16) -> (u16, u16) {
    (a / b, a % b)
}
fn host_stats(a: u16, b: u16, c: u16) -> (u16, u16, u16) {
    (a.min(b).min(c), a.max(b).max(c), a + b + c)
}
fn host_run() -> u16 {
    let (q, r) = host_divmod(47, 5);
    let (lo, hi, sum) = host_stats(7, 2, 5);
    q * 1000 + r * 100 + hi * 10 + lo + sum
}

fn main() {
    // Read the result registers straight back — several values returned at once.
    let [q, r, _] = cpu::run_regs(SRC, "divmod", &[47, 5]);
    let [lo, hi, sum] = cpu::run_regs(SRC, "stats", &[7, 2, 5]);
    println!("divmod(47, 5)   -> (q={q}, r={r})         [HL, DE]");
    println!("stats(7, 2, 5)  -> (min={lo}, max={hi}, sum={sum})  [HL, DE, BC]");
    assert_eq!((q, r), host_divmod(47, 5));
    assert_eq!((lo, hi, sum), host_stats(7, 2, 5));

    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();
    println!("\n  packed run()  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ tuple returns (HL/DE/BC) destructured on the Z80");
}
