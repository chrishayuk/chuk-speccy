//! Three classic integer algorithms, each calling a different control-flow feature:
//! **gcd** (Euclid — `while` + `%`), **isqrt** (Newton's method — early `return`), and
//! **fib** (`loop` + `break`). One compiled program; the harness calls each entry by
//! symbol with its arguments in `HL`/`DE`.
//!
//! Dialect program: [`samples/showcase/numerics.rs`].
//!
//!     cargo run -p rustz80 --example numerics

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/numerics.rs");

fn host_gcd(mut a: u16, mut b: u16) -> u16 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}
fn host_isqrt(n: u16) -> u16 {
    (n as f64).sqrt() as u16
}
fn host_fib(n: u16) -> u16 {
    let (mut a, mut b) = (0u16, 1u16);
    for _ in 0..n {
        let t = a + b;
        a = b;
        b = t;
    }
    a
}

fn check(label: &str, got: u16, want: u16) {
    println!("  {label:<22} z80 = {got:<6} rustc = {want}");
    assert_eq!(got, want, "{label}: z80 and rustc disagree");
}

fn main() {
    println!("classic integer algorithms on the Z80:");
    check(
        "gcd(1071, 462)",
        cpu::run_value(SRC, "gcd", &[1071, 462]),
        host_gcd(1071, 462),
    );
    check(
        "isqrt(10000)",
        cpu::run_value(SRC, "isqrt", &[10000]),
        host_isqrt(10000),
    );
    // fib(23) is the largest whose every intermediate still fits in u16 (fib(25)
    // would overflow — and the dialect's `+` wraps where rustc's checks, so we stay
    // in range to keep the two bit-identical).
    check("fib(23)", cpu::run_value(SRC, "fib", &[23]), host_fib(23));
    println!("  ✓ gcd=21, isqrt=100, fib(23)=28657 — all agree");
}
