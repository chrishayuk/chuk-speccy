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
- **Code ~1070× smaller** — 47 bytes of Z80 vs a 50 KB compiled Wasm module. Small enough
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

## The full lifecycle (the disposable-tool story)

Startup matters more than peak throughput for agent tools: generate a tiny function,
compile, run once, discard — or retrieve a tool, instantiate, score 20 candidates, discard.
The harness prints every phase against a usefulness band:

```
parse source (syn)                     15.5 µs   realistic agent snippet
compile source → image                 18.0 µs   realistic agent snippet
instantiate runner (cached image)       1.1 µs   hot-loop cell/tool call
warm run_fast — tiny (overhead floor)   0.06 µs   excellent warm primitive
warm run_fast — realistic (score)       0.25 µs   excellent warm primitive
batch run_many_fast — per-call          0.05 µs   excellent warm primitive
cold: compile source + 1 run           18.9 µs   realistic agent snippet
cold: cached image + 1 run              1.0 µs   hot-loop cell/tool call
cold: image bytes + 1 run               1.2 µs   hot-loop cell/tool call
```

Reading it:

- **The per-call overhead floor is ~0.06 µs** (a trivial cell) — reset + trampoline + CPU
  setup. The score's 0.25 µs (single `run_fast`) is mostly actual Z80 emulation, not
  framework cost. `run_fast` is genuinely an inner-loop primitive.
- **`run_many_fast` is ~0.05 µs/call** — a **~5×** drop. For a *straight-line* cell it
  decodes once and replays on a stripped, native-register executor (no per-instruction
  fetch/contention/refresh/flag work); the cycle count is input-independent so it's taken
  from one authentic calibration run, and the results stay differential-checked against the
  authentic interpreter. Anything outside that subset (branches/calls/shifts/`(HL)`) falls
  back transparently. That puts the batch hot path **~4× off native-JIT Wasm** (0.013 µs),
  vs ~18× for the general per-call `run_fast`.
- **A whole disposable tool — instantiate + run — is ~1 µs.** That's the "million tiny
  tools" number: retrieve by manifest, instantiate cheaply, run deterministically, discard.
- **The cell image is a 71-byte cartridge.** `CellProgram::to_bytes()` serializes code +
  symbols + policy with no syn; `from_bytes()` reloads + runs in **~1.2 µs — 16× cheaper
  than compiling the 235-byte source**. Cache it by hash, ship it, index it.

Against the usefulness thresholds (`<1 µs` warm primitive, `1–10 µs` hot-loop call,
`10–100 µs` realistic snippet, `100–500 µs` cold tool), every phase here lands in a useful
band. The claim isn't "faster than Wasm" — it's **smaller to start, cheaper to throw away,
easier to bound and inspect**: a microsecond-scale safe tool capsule.
