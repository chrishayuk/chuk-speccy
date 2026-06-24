//! The headline container: **`Entities<Cell, const N>`** — a const-generic struct whose
//! field is an array of structs. Each capacity is a separate monomorphic instance
//! (`Entities$4`, `Entities$8`), laid out and compiled independently; `N` bounds `add`.
//! No heap, fixed capacity, deterministic. Same source under rustc and rustz80.
//!
//! Dialect program: [`samples/showcase/entities.rs`].
//!
//!     cargo run -p rustz80 --example entities

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/entities.rs");

#[derive(Clone, Copy)]
struct Cell {
    x: u16,
    y: u16,
}
struct Entities<const N: usize> {
    data: [Cell; N],
    len: u16,
}
impl<const N: usize> Entities<N> {
    fn add(&mut self, x: u16, y: u16) {
        if self.len < N as u16 {
            self.data[self.len as usize] = Cell { x, y };
            self.len += 1;
        }
    }
    fn checksum(&self) -> u16 {
        (0..self.len as usize)
            .map(|i| self.data[i].x * 100 + self.data[i].y)
            .sum()
    }
}
fn host_run() -> u16 {
    let mut a = Entities {
        data: [Cell { x: 0, y: 0 }; 4],
        len: 0,
    };
    a.add(1, 2);
    a.add(3, 4);
    let mut b = Entities {
        data: [Cell { x: 0, y: 0 }; 8],
        len: 0,
    };
    b.add(5, 6);
    b.add(7, 8);
    b.add(9, 10);
    a.checksum() + b.checksum()
}

fn main() {
    let prog = rustz80::compile_program(SRC).expect("compile");
    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();
    println!("Entities<Cell, const N> at two capacities — one source, two instances:");
    for sym in ["Entities$4::add", "Entities$8::add"] {
        println!("  • {sym}  @ {:#06x}", prog.symbols[sym]);
    }
    println!("  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ a const-generic entity pool (`[Cell; N]` field) ran on the Z80 (= 2530)");
}
