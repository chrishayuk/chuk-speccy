//! Clamp a value to the inclusive range [lo, hi].
//! tags: math, range, clamp, bound
fn run(x: u16, lo: u16, hi: u16) -> u16 {
    let mut r = x;
    if x < lo { r = lo; }
    if x > hi { r = hi; }
    r
}
