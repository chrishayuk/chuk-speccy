//! A **fixed-capacity entity pool** — a struct whose field is an array of structs
//! (`items: [Cell; 8]`), the `Entities<T, N>` shape. `push` appends through the
//! receiver pointer; `checksum` reads `self.items[i].x`/`.y`. No heap, fixed capacity,
//! deterministic. Same source under rustc and rustz80.
//!
//! Dialect program: [`samples/showcase/pool.rs`].
//!
//!     cargo run -p rustz80 --example pool

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/pool.rs");

#[derive(Clone, Copy)]
struct Cell {
    x: u16,
    y: u16,
}
struct Pool {
    items: [Cell; 8],
    len: u16,
}
impl Pool {
    fn push(&mut self, x: u16, y: u16) {
        if self.len < 8 {
            self.items[self.len as usize] = Cell { x, y };
            self.len += 1;
        }
    }
    fn checksum(&self) -> u16 {
        (0..self.len as usize)
            .map(|i| self.items[i].x * 100 + self.items[i].y)
            .sum()
    }
}
fn host_run() -> u16 {
    let mut p = Pool {
        items: [Cell { x: 0, y: 0 }; 8],
        len: 0,
    };
    p.push(1, 2);
    p.push(3, 4);
    p.push(5, 6);
    p.checksum() + p.items[0].x
}

fn main() {
    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();
    println!("a fixed-capacity pool of (x, y) cells: push 3, checksum + items[0].x");
    println!("  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ an Entities-shaped struct (`[Cell; N]` field) ran on the Z80 (= 913)");
}
