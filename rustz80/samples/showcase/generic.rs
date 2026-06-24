// Generic functions, monomorphized to Z80, in the rustz80 dialect.
// The SAME source type-checks under rustc (the bounds are real) and compiles here —
// each call instantiates a width-specialized copy. `clamp` is itself generic and
// calls two more generics, so one `clamp` at u16 transitively pulls in max/min at
// u16, and `clamp` at u8 pulls in their u8 copies.
// Entry `run` -> 40 + 10 + 150 = 200.

fn max<T: Ord + Copy>(a: T, b: T) -> T {
    let mut r = a;
    if b > a {
        r = b;
    }
    r
}

fn min<T: Ord + Copy>(a: T, b: T) -> T {
    let mut r = a;
    if b < a {
        r = b;
    }
    r
}

fn clamp<T: Ord + Copy>(x: T, lo: T, hi: T) -> T {
    min(max(x, lo), hi)
}

fn run() -> u16 {
    let a = clamp(50u16, 10u16, 40u16); // clamp::<u16> -> 40
    let b = clamp(5u16, 10u16, 40u16); // clamp::<u16> -> 10
    let u = clamp(200u8, 50u8, 150u8); // clamp::<u8>  -> 150
    a + b + u as u16
}
