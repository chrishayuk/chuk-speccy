//! Benchmark the `rustz80-cell` micro-VM: per-cell **latency** (compile + run) and the
//! **emulated-Z80 throughput** (T-states/sec), across synthetic workloads + two real
//! showcases. No criterion — a plain `main` (adaptive timing windows).
//!
//!     cargo bench -p rustz80 --features cell --bench cell
//!
//! `run = total − compile`; throughput = `cycles / run_secs`. A real 48K Spectrum runs
//! at **3.5 MHz**, so the "×real" column is how much faster than period hardware the VM
//! steps. (The runner allocates a fresh 64 KiB bus per run — negligible for the heavy
//! steady-state rows, where the MHz figure is meaningful.)

use rustz80::cell::{self, Runner};
use std::time::Instant;

const BUDGET: u64 = 50_000_000; // high enough that nothing here hits it
const REAL_MHZ: f64 = 3.5;

/// Run `f` in ~120 ms windows; return mean seconds/iter.
fn time_per(mut f: impl FnMut()) -> f64 {
    f(); // warmup
    let mut iters = 0u64;
    let t = Instant::now();
    while t.elapsed().as_secs_f64() < 0.12 {
        f();
        iters += 1;
    }
    t.elapsed().as_secs_f64() / iters as f64
}

fn bench(name: &str, src: &str) {
    // Compile (one-time), warm run (reused bus, the compile-once/run-many path), and a
    // cold one-shot (`cell::run` — fresh alloc + compile + run) for comparison.
    let compile = time_per(|| {
        Runner::compile(src).unwrap();
    });
    let mut runner = Runner::compile(src).unwrap();
    let warm = time_per(|| {
        runner.run(None, &[], BUDGET).unwrap();
    });
    let cold = time_per(|| {
        cell::run(src, None, &[], BUDGET).unwrap();
    });

    let r = runner.run(None, &[], BUDGET).unwrap();
    let mhz = r.cycles as f64 / warm / 1e6; // M T-states / sec == MHz
    let note = if r.returned { "" } else { " [budget!]" };
    println!(
        "{name:11} {res:>6} {cyc:>9}  {code:>4}B   {comp:>6.1}   {wm:>8.2}   {cd:>7.1}   {mhz:>6.0}   {x:>5.0}×{note}",
        res = r.result,
        cyc = r.cycles,
        code = r.code_bytes,
        comp = compile * 1e6,
        wm = warm * 1e6,
        cd = cold * 1e6,
        x = mhz / REAL_MHZ,
    );
}

fn main() {
    // Synthetic workloads (entry `run`), spanning op mixes.
    let tiny = "fn run() -> u16 { 42u16 }";
    let add_loop = "fn run() -> u16 {
        let mut s = 0u16; let mut i = 0u16;
        while i < 30000u16 { s = s + i; i = i + 1u16; } s }";
    let mul_loop = "fn run() -> u16 {
        let mut s = 1u16; let mut i = 0u16;
        while i < 10000u16 { s = s.wrapping_mul(3u16); i = i + 1u16; } s }";
    let xorshift_1k = "fn run() -> u16 {
        let mut x: u32 = 2463534242u32; let mut i = 0u16;
        while i < 1000u16 {
            x = x ^ (x << 13u32); x = x ^ (x >> 17u32); x = x ^ (x << 5u32);
            i = i + 1u16;
        }
        x as u16 }";
    let rng32 = include_str!("../samples/showcase/rng32.rs");
    let entities = include_str!("../samples/showcase/entities.rs");

    println!("rustz80-cell — microseconds per cell, and emulated-Z80 throughput\n");
    println!(
        "{:11} {:>6} {:>9}  {:>5}  {:>7}  {:>9}  {:>8}  {:>6}  {:>6}",
        "workload", "result", "cycles", "code", "comp µs", "warm µs", "cold µs", "MHz", "×real"
    );
    println!("{}", "-".repeat(86));
    bench("tiny", tiny);
    bench("add_loop", add_loop);
    bench("mul_loop", mul_loop);
    bench("xorshift1k", xorshift_1k);
    bench("rng32", rng32);
    bench("entities", entities);
    println!(
        "\n(real 48K Spectrum = 3.5 MHz. warm = Runner reuse — bus reset, not realloc;\n \
         cold = one-shot cell::run — fresh alloc + compile + run. compile is one-time.)"
    );
}
