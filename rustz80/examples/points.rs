//! An **array of structs** — `[Cell; N]` where each element is a multi-slot `Cell`.
//! Element field access (`pts[i].x`) reads at a computed address `&pts + i*stride +
//! field_offset`. This is the storage `Entities<Cell, N>` needs. Same source under
//! rustc and rustz80.
//!
//! Dialect program: [`samples/showcase/points.rs`].
//!
//!     cargo run -p rustz80 --example points

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/points.rs");

#[derive(Clone, Copy)]
struct Cell {
    x: u16,
    y: u16,
}
fn host_run() -> u16 {
    let mut pts = [Cell { x: 0, y: 0 }; 5];
    for (i, p) in pts.iter_mut().enumerate() {
        let i = i as u16;
        *p = Cell { x: i, y: i * i };
    }
    pts.iter().map(|p| p.x + p.y * 10).sum()
}

fn main() {
    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();
    println!("points (i, i*i) for i in 0..5, checksum sum(x + y*10)");
    println!("  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ an array of structs ran on the Z80 (= 310)");
}
