//! **Generic structs + methods, and a tuple struct field** — `struct Vec2<T>` with
//! `impl<T> Vec2<T>`, plus a `Player` whose `pos` is a `(u16, u16)` accessed by
//! element (`self.pos.0`). The same source compiles under rustc and rustz80.
//!
//! Dialect program: [`samples/showcase/structs.rs`].
//!
//!     cargo run -p rustz80 --example structs

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/structs.rs");

// The oracle — the identical types in plain Rust.
struct Vec2<T> {
    x: T,
    y: T,
}
impl<T: Copy + core::ops::Add<Output = T>> Vec2<T> {
    fn sum(&self) -> T {
        self.x + self.y
    }
    fn shift(&mut self, d: T) {
        self.x = self.x + d;
        self.y = self.y + d;
    }
}
struct Player {
    pos: (u16, u16),
    score: u16,
}
impl Player {
    fn step(&mut self, dx: u16, dy: u16) {
        self.pos.0 += dx;
        self.pos.1 += dy;
        self.score += 1;
    }
    fn key(&self) -> u16 {
        self.pos.0 * 100 + self.pos.1
    }
}
fn host_run() -> u16 {
    let mut v = Vec2 { x: 3u16, y: 4u16 };
    v.shift(10);
    let s = v.sum();
    let mut p = Player {
        pos: (5, 6),
        score: 0,
    };
    p.step(2, 3);
    p.step(1, 0);
    s + p.key() + p.score
}

fn main() {
    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();
    println!("Vec2<u16>.sum() after shift(10) = 27");
    println!("Player.pos (tuple field) after two steps = (8, 9), score 2");
    println!("  packed  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ generic struct + tuple field ran on the Z80 (= 838)");
}
