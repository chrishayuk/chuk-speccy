//! Absolute difference |a - b| between two values.
//! tags: math, distance, diff
fn run(a: u16, b: u16) -> u16 {
    let mut d = 0u16;
    if a > b { d = a - b; } else { d = b - a; }
    d
}
