//! **Code-size report** — compile a program and print the byte size of every emitted
//! function, including each monomorphized generic *instance* and the appended mul/div
//! runtime. Shows what generics/tuples actually cost in Z80 bytes.
//!
//!     cargo run -p rustz80 --example report

// The program from the generics + tuples showcases, plus a couple of plain fns — so
// the report has instances (`max$u16`, `min$u8`, …), a tuple return, and the runtime.
const SRC: &str = r#"
    fn max<T: Ord + Copy>(a: T, b: T) -> T { let mut r = a; if b > a { r = b; } r }
    fn min<T: Ord + Copy>(a: T, b: T) -> T { let mut r = a; if b < a { r = b; } r }
    fn clamp<T: Ord + Copy>(x: T, lo: T, hi: T) -> T { min(max(x, lo), hi) }
    fn divmod(a: u16, b: u16) -> (u16, u16) { (a / b, a % b) }
    fn run() -> u16 {
        let a = clamp(50u16, 10u16, 40u16);
        let b = clamp(200u8, 50u8, 150u8);
        let (q, r) = divmod(1000u16, 7u16);
        a + b as u16 + q + r
    }
"#;

fn main() {
    let prog = rustz80::compile_program(SRC).expect("compile");
    let report = prog.size_report();

    println!("{:<16} {:>6} {:>6}   kind", "function", "addr", "bytes");
    println!("{}", "-".repeat(40));
    let mut total = 0u16;
    let (mut n_inst, mut n_rt) = (0, 0);
    for f in &report {
        let kind = if f.instance {
            n_inst += 1;
            "instance"
        } else if f.name.starts_with("__") {
            n_rt += 1;
            "runtime"
        } else {
            "fn"
        };
        println!("{:<16} 0x{:04X} {:>6}   {kind}", f.name, f.addr, f.size);
        total += f.size;
    }
    println!("{}", "-".repeat(40));
    println!(
        "{} symbols ({n_inst} instances, {n_rt} runtime), {total} bytes total",
        report.len()
    );
    assert_eq!(
        total as usize,
        prog.code.len(),
        "sizes cover the whole image"
    );
}
