//! Greatest common divisor (Euclid's algorithm).
//! tags: math, bench, gcd, number
fn run(a: u16, b: u16) -> u16 {
    let mut x = a;
    let mut y = b;
    while y != 0u16 { let t = x % y; x = y; y = t; }
    x
}
