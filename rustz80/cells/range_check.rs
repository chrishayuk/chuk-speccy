//! Returns 1 if lo <= x <= hi, else 0.
//! tags: validation, validate, range, bounds, check
fn run(x: u16, lo: u16, hi: u16) -> u16 {
    let mut ok = 1u16;
    if x < lo { ok = 0u16; }
    if x > hi { ok = 0u16; }
    ok
}
