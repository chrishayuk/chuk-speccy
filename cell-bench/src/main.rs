//! `cell-bench` — an apples-to-apples comparison of **tiny-snippet execution** across
//! runtimes, the honest test of where `rustz80-cell` belongs (see roadmap B3).
//!
//! Task: score `N` candidate states with `score(x, y) = x*x + y*y + x*3` (the canonical
//! agent op — evaluate a candidate). Contenders: **native Rust** (the floor), **Wasmtime**
//! (a JIT Wasm runtime, warm instance), **rustz80-cell** (warm `Runner`), and **Python**
//! (a `python3` subprocess — the "let the agent run code to check something" pattern).
//!
//! The point isn't raw speed (Wasm JITs to native and wins compute). It's that for tiny
//! bounded snippets the cell is in a usable latency band while being far smaller, more
//! inspectable, and deterministic. Standalone crate so Wasmtime's deps stay out of the
//! emulator workspace.
//!
//!     cargo run --release --manifest-path cell-bench/Cargo.toml

use std::hint::black_box;
use std::process::Command;
use std::time::Instant;

const N: usize = 1000;
const CELL_BUDGET: u64 = 2_000_000;
const CELL_SRC: &str = include_str!("../score.rs");
const WAT: &str = r#"(module
  (func (export "run") (param i32 i32) (result i32)
    (i32.add
      (i32.add (i32.mul (local.get 0) (local.get 0))
               (i32.mul (local.get 1) (local.get 1)))
      (i32.mul (local.get 0) (i32.const 3)))))"#;

fn cand(i: usize) -> (u16, u16) {
    ((i % 64) as u16, (i.wrapping_mul(13) % 64) as u16)
}
fn native_score(x: u16, y: u16) -> u16 {
    x * x + y * y + x * 3
}

/// Time `f` (one full N-candidate pass returning the score-sum) in ~200 ms windows;
/// return `(seconds per call, sum)`.
fn bench(mut f: impl FnMut() -> u64) -> (f64, u64) {
    let sum = f(); // warmup + the value to cross-check
    let mut iters = 0u64;
    let t = Instant::now();
    while t.elapsed().as_secs_f64() < 0.2 {
        f();
        iters += 1;
    }
    (t.elapsed().as_secs_f64() / iters as f64 / N as f64, sum)
}

/// Python via a `python3 -c` subprocess: a batch pass (the whole N loop) and a single-eval
/// run (the interpreter-startup tax). Returns `(per-call amortized, startup, sum)`.
fn py_bench() -> Option<(f64, f64, u64)> {
    let run = |code: &str| -> Option<(f64, String)> {
        let t = Instant::now();
        let out = Command::new("python3").arg("-c").arg(code).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some((
            t.elapsed().as_secs_f64(),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ))
    };
    let batch = format!("s=0\nfor i in range({N}):\n x=i%64;y=(i*13)%64;s+=x*x+y*y+x*3\nprint(s)");
    let (batch_t, out) = run(&batch)?;
    let sum: u64 = out.parse().ok()?;
    let (startup_t, _) = run("print(1*1+0*0+1*3)")?; // one eval — measures startup
    Some((batch_t / N as f64, startup_t, sum))
}

fn us(secs: f64) -> String {
    format!("{:.3}", secs * 1e6)
}

fn main() {
    println!("cell-bench — score(x,y) = x*x + y*y + x*3, over N={N} candidates\n");

    // 1. Native Rust — the floor.
    let (native_pc, native_sum) = bench(|| {
        (0..N)
            .map(|i| {
                let (x, y) = cand(black_box(i));
                black_box(native_score(black_box(x), black_box(y))) as u64
            })
            .sum()
    });

    // 2. Wasmtime — a JIT Wasm runtime, warm instance.
    let (wasm_pc, wasm_cold, wasm_sum, wasm_bytes) = {
        use wasmtime::*;
        let t0 = Instant::now();
        let engine = Engine::default();
        let module = Module::new(&engine, WAT).expect("compile wat");
        let bytes = module.serialize().map(|b| b.len()).unwrap_or(0);
        let mut store = Store::new(&engine, ());
        let inst = Instance::new(&mut store, &module, &[]).expect("instantiate");
        let run = inst
            .get_typed_func::<(i32, i32), i32>(&mut store, "run")
            .expect("func");
        let cold = t0.elapsed().as_secs_f64();
        let (pc, sum) = bench(|| {
            (0..N)
                .map(|i| {
                    let (x, y) = cand(black_box(i));
                    run.call(&mut store, (x as i32, y as i32)).unwrap() as u64
                })
                .sum()
        });
        (pc, cold, sum, bytes)
    };

    // 3. rustz80-cell — warm Runner (compile once, run many).
    let (cell_pc, cell_cold, cell_sum, cell_bytes) = {
        use rustz80::cell::Runner;
        let t0 = Instant::now();
        let mut r = Runner::compile(CELL_SRC).expect("compile cell");
        let cold = t0.elapsed().as_secs_f64();
        let code = r.program().code.len();
        let (pc, sum) = bench(|| {
            (0..N)
                .map(|i| {
                    let (x, y) = cand(black_box(i));
                    r.run(None, &[x, y], CELL_BUDGET).unwrap().result as u64
                })
                .sum()
        });
        (pc, cold, sum, code)
    };

    // 4. Python subprocess.
    let py = py_bench();

    // --- table ---
    println!(
        "{:<14} {:>12} {:>12} {:>14}   result-sum",
        "runtime", "per-call", "cold setup", "batch(1000)"
    );
    println!("{}", "-".repeat(74));
    let row = |name: &str, pc: f64, cold: Option<f64>, sum: u64| {
        let cold = cold.map(|c| format!("{} µs", us(c))).unwrap_or("—".into());
        println!(
            "{:<14} {:>9} µs {:>12} {:>11} µs   {}",
            name,
            us(pc),
            cold,
            us(pc * N as f64),
            sum
        );
    };
    row("native Rust", native_pc, None, native_sum);
    row("wasmtime", wasm_pc, Some(wasm_cold), wasm_sum);
    row("rustz80-cell", cell_pc, Some(cell_cold), cell_sum);
    match py {
        Some((pc, startup, sum)) => row("python (subp)", pc, Some(startup), sum),
        None => println!("{:<14} (python3 not available — skipped)", "python"),
    }

    // --- correctness + qualitative footer ---
    let sums = [native_sum, wasm_sum, cell_sum]
        .iter()
        .chain(py.iter().map(|(_, _, s)| s))
        .copied()
        .collect::<Vec<_>>();
    assert!(
        sums.windows(2).all(|w| w[0] == w[1]),
        "runtimes disagree on the score sum: {sums:?}"
    );
    println!("\nall runtimes agree (sum = {native_sum}).");
    println!(
        "code size: rustz80-cell {cell_bytes} B Z80 vs wasmtime {wasm_bytes} B compiled module."
    );
    println!(
        "python per-call is amortized over the batch (one interpreter startup ≈ its cold column);\n\
         a fresh subprocess *per* candidate would pay that startup 1000×.\n\
         the cell's edge isn't speed — it's a tiny, deterministic, inspectable sandbox\n\
         (64K, no WASI/imports, cycle-bounded, typed state read-back)."
    );
}
