// Three classic integer algorithms, in the rustz80 dialect — each leaning on a
// different control-flow feature. Call each entry by symbol with args in HL/DE.
//   gcd(a, b)  -> Euclid's algorithm  (`while` + `%`)
//   isqrt(n)   -> Newton's method     (early `return`)
//   fib(n)     -> Fibonacci           (`loop` + `break`)

fn gcd(a: u16, b: u16) -> u16 {
    let mut x = a;
    let mut y = b;
    while y != 0u16 {
        let t = y;
        y = x % y;
        x = t;
    }
    x
}

fn isqrt(n: u16) -> u16 {
    if n < 2u16 {
        return n; // early return for the trivial cases
    }
    let mut x = n;
    let mut y = (x + 1u16) / 2u16;
    while y < x {
        // Newton iteration, converging from above.
        x = y;
        y = (x + n / x) / 2u16;
    }
    x
}

fn fib(n: u16) -> u16 {
    let mut a = 0u16;
    let mut b = 1u16;
    let mut i = 0u16;
    loop {
        if i >= n {
            break;
        }
        let t = a + b;
        a = b;
        b = t;
        i = i + 1u16;
    }
    a
}
