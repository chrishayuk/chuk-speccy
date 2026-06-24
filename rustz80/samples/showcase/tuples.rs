// Multiple return values via tuples, in the rustz80 dialect. A tuple return lands
// in the HL/DE/BC registers (up to three values), destructured at the call site.
//   divmod(a, b)    -> (a / b, a % b)
//   stats(a, b, c)  -> (min, max, sum)

fn divmod(a: u16, b: u16) -> (u16, u16) {
    (a / b, a % b)
}

fn stats(a: u16, b: u16, c: u16) -> (u16, u16, u16) {
    let mut lo = a;
    let mut hi = a;
    if b < lo {
        lo = b;
    }
    if c < lo {
        lo = c;
    }
    if b > hi {
        hi = b;
    }
    if c > hi {
        hi = c;
    }
    (lo, hi, a + b + c)
}

// Pack both calls' results into one u16 so the value is easy to check end to end.
fn run() -> u16 {
    let (q, r) = divmod(47u16, 5u16); // (9, 2)
    let (lo, hi, sum) = stats(7u16, 2u16, 5u16); // (2, 7, 14)
    q * 1000u16 + r * 100u16 + hi * 10u16 + lo + sum // 9000 + 200 + 70 + 2 + 14 = 9286
}
