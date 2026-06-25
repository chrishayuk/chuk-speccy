// cell-bench scoring function — the same math in native Rust, Wasm, and Python.
// Inputs stay in 0..64 so `x*x + y*y + x*3` never overflows u16 → all runtimes agree.
fn run(x: u16, y: u16) -> u16 {
    x * x + y * y + x * 3u16
}
