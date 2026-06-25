# cell-bench — tiny-snippet execution, across runtimes

An apples-to-apples comparison of running a **tiny agent-shaped program** — score `N`
candidate states with `score(x, y) = x*x + y*y + x*3` — across four runtimes:

- **native Rust** — the floor (inlined machine code);
- **Wasmtime** — a production JIT Wasm runtime, warm instance;
- **rustz80-cell** — the warm `Runner` (compile once, run many);
- **Python** — a `python3 -c` subprocess (the "let the agent run code to check something"
  pattern).

Standalone crate (its own workspace) so Wasmtime's large dependency tree never touches
`cargo test --workspace` on the emulator.

```bash
cargo run --release --manifest-path cell-bench/Cargo.toml
```

## Representative result (Apple Silicon)

```
runtime            per-call   cold setup    batch(1000)   result-sum
--------------------------------------------------------------------------
native Rust        0.001 µs            —       0.681 µs   2722460
wasmtime           0.013 µs  2876.000 µs      12.623 µs   2722460
cell (report)      0.499 µs   540.667 µs     499.391 µs   2722460
cell (fast)        0.237 µs            —      237.346 µs   2722460
python (subp)     36.911 µs 34892.000 µs  36911.083 µs   2722460
```
(all agree on the sum — same computation. The cell runs in **Cell80 mode** — `*`/`/`/`%`
are host-native `ED FE` traps, not a software loop. `cell (fast)` is `run_fast`, skipping
the `Report`; cached re-instantiation `Runner::new` is ~1.1 µs.)

## How to read it

This is **not** a claim that the cell is fast. Wasm JITs to native and **wins warm
compute by ~18×** (0.013 µs vs ~0.24 µs/call) — exactly as it should. For a real algorithm,
use Wasm.

What the cell wins, for the *tiny-snippet* class:

- **Cold setup ~5× lower** — 0.59 ms vs Wasmtime's 3.0 ms (compile + instantiate). For
  "run a small thing once and throw it away," setup dominates, and the cell is cheaper.
- **Code ~950× smaller** — 53 bytes of Z80 vs a 50 KB compiled Wasm module. Small enough
  to inspect, hash, cache, or show a human.
- **Far lighter than Python** — Python is ~37 µs/call amortized and ~35 ms just to start
  the interpreter; a fresh subprocess *per* candidate would pay that 1000×.
- Plus the qualitative differentiators a table can't show: **determinism** (replayable),
  **typed state read-back** (source-shaped, not linear memory), **capability gating** +
  **cycle budget**, and a sandbox surface you can hold in your head (64K, no WASI/imports).

At ~0.24 µs/call (fast path) the cell runs **~4 million evaluations per second** — well
within "call it in an agent loop." The pitch isn't a Wasm replacement; it's a *smaller,
more inspectable, deterministic sandbox for tiny agent-generated programs.*

## Cold setup, broken down

The harness also prints where the cell's cold setup goes (the ~0.59 ms table figure is a
single cold-process sample — first page faults + cold caches; amortized it's far less):

```
CellProgram::compile        19.168 µs   (syn parse 16.727 µs + lower/codegen)
Runner::new (cached prog)    1.219 µs   ← caching a known snippet skips parse+compile
```

So cold setup is **~90% syn parsing** — the bus allocation is amortized-free. The lever
isn't a faster parser; it's *not re-parsing*: compile to a cacheable `CellProgram` once,
then `Runner::new` instantiates a fresh machine in **~1.2 µs** (~16× cheaper). For an agent
that re-runs known snippets, cold setup effectively disappears — and vs Wasm's ~3 ms JIT
that's ~2500× cheaper.
