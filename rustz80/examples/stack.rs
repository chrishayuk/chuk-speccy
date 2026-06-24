//! A **const-generic fixed-capacity stack** — `struct Stack<const N: usize>` whose
//! const param sizes the `[u16; N]` array and bounds `push`. Used at two capacities,
//! so the compiler emits two specialized instances (`Stack$4`, `Stack$8`), each with
//! its own layout and methods. This is the shape `Entities<T, N>` needs.
//!
//! Dialect program: [`samples/showcase/stack.rs`].
//!
//!     cargo run -p rustz80 --example stack

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/stack.rs");

struct Stack<const N: usize> {
    data: [u16; N],
    len: usize,
}
impl<const N: usize> Stack<N> {
    fn push(&mut self, v: u16) {
        if self.len < N {
            self.data[self.len] = v;
            self.len += 1;
        }
    }
    fn sum(&self) -> u16 {
        self.data[..self.len].iter().sum()
    }
}
fn host_run() -> u16 {
    let mut a: Stack<4> = Stack {
        data: [0; 4],
        len: 0,
    };
    for v in [1, 2, 3, 4, 5] {
        a.push(v);
    }
    let mut b: Stack<8> = Stack {
        data: [0; 8],
        len: 0,
    };
    for v in 1..=8 {
        b.push(v);
    }
    a.sum() * 1000 + b.sum()
}

fn main() {
    let prog = rustz80::compile_program(SRC).expect("compile");
    let mut insts: Vec<&String> = prog.symbols.keys().filter(|k| k.contains('$')).collect();
    insts.sort();
    println!("const-generic struct instances: {insts:?}");

    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();
    println!("  Stack<4> (cap-capped) sum*1000 + Stack<8> sum  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ a fixed-capacity stack ran at two sizes on the Z80 (= 10036)");
}
