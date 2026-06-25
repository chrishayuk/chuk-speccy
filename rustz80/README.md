# `rustz80` — a restricted Rust → Z80 compiler

Write a game in a **subset of Rust that is also real Rust**, and compile it to Z80
machine code that boots on a real ZX Spectrum — no C, no external toolchain. The
same `.rs` runs two ways:

- under **`rustc`** (`cargo run`) — host execution, fast iteration, a real debugger;
- through **`rustz80`** — Z80 you can package as a `.tap` and boot on the ROM.

The two are kept honest by **differential testing**: every feature is run both ways
and the results must match (see [`tests/`](./tests)). Design rationale lives in
[spec 07](../docs/07-rust-z80-compiler-spec.md).

Not an LLVM backend and not real `core`: a `syn` frontend → a small typed IR → naive
Z80 codegen (`HL` accumulator, `DE` secondary, a fixed RAM "register file"), plus a
hand-written mul/div micro-runtime.

`rustz80` is **generic** — it knows nothing about games or any SDK. The game layer
(`impl Game`, the dialect prelude, the symbol map, the `speccy-compile` CLI) lives in
[`chuk-speccy-sdk`](../speccy-sdk) behind its `compile` feature, built on this crate's
generic API (`lower_program` with a caller-supplied `PreludeConfig`, `codegen_loop`,
`to_tap`).

## Quick start

```bash
# Compile a dialect program to a bootable tape (entry: a no-arg `fn main`):
cargo run -p chuk-speccy-sdk --features compile --bin speccy-compile -- rustz80/samples/snake.rs -o snake.tap

# Boot it on the emulator (needs a 48K ROM at testroms/48.rom):
cargo run --release --bin speccy-gui -- testroms/48.rom snake.tap
```

`speccy-compile <input.rs> [-o out.tap] [--entry main] [--name GAME]`. Samples live
in [`samples/`](./samples) (`snake.rs`, `pixels.rs`).

## The dialect

Supported today (all differential-tested):

| Feature | Notes |
|---|---|
| Types | `u16` (default) and `u8` (wraps at 256). `as u8` truncates, `as u16`/`as usize` widen. `u32` (two slots, computed in `HL:DE`) for `^ & \|` + constant shifts + `as u16`/`as u8` — enough for a 32-bit xorshift RNG. |
| Arithmetic | `+ - * / %`, `wrapping_add/sub/mul`. `*`/`/`/`%` use the appended micro-runtime. (16-bit; `u32` arithmetic beyond bitwise/shift is not done yet.) |
| Bitwise | `\|` `&` `^`, and `<<` / `>>` by a **constant** amount (`u16` and `u32`). |
| Control flow | `if`/`else if`/`else`, `while`, `for` over integer ranges (`a..b` / `a..=b`, `for _ in`), `loop` / `break` / `continue`, early `return`; comparison conditions (`< <= > >= == !=`). |
| Arrays | `let a = [0u16; N];` / `[e0, e1, …]`; `a[i]`, `a[i] = v`. Index with `i as usize`. `[u8; N]` are byte-packed-per-slot. Arrays of structs `let a = [Cell { … }; N]` — element field access `a[i].x` (read/write) + whole-element assign `a[i] = Cell { … }`. |
| Structs | `struct P { x: u16, y: u16 }` + literals + `p.x` read/write. Scalar, `[u16; N]`, tuple (`pos: (u16, u16)`, `p.pos.0`), and array-of-structs (`cells: [Cell; N]`, `p.cells[i].x`, `p.cells[i] = Cell { … }`) fields. |
| Enums + match | `enum Dir { Up = 1, … }` (explicit discriminants or `0,1,2,…`); `match` on integers/variants with `_`. Plus `bool` (`true`/`false`). |
| Functions + methods | Free fns and `impl T { fn m(&mut self, …) }` — up to 3 args in `HL`/`DE`/`BC`, result in `HL`; `self.field` through the receiver. |
| Generics | Generic *free functions* (`fn max<T: Ord>(…)`, `fn buf<const N: usize>()`), monomorphized per call — a type argument (turbofish or inferred) sets the instance's width, a const argument (turbofish) sizes arrays and substitutes as a value. Generic *structs* + methods (`struct Pair<T>`): type args erased to 16-bit. **Const-generic structs** (`struct Buf<const N: usize> { data: [u16; N], … }`) are monomorphized per `N` — a per-instance layout + methods (`Buf$8::push`), `N` inferred at the struct literal from the array field's length. The field may itself be an array of structs — **`Entities<Cell, const N> { data: [Cell; N], … }`**, the fixed-capacity entity pool. |
| Tuples | Multiple return values: `fn divmod(…) -> (u16, u16)` (in `HL`/`DE`/`BC`) destructured with `let (q, r) = …` — a tuple literal or a call. |
| Raw I/O | `poke(addr, val)` / `peek(addr)` (memory) and `inport(port)` (I/O ports, e.g. the keyboard at `0xFE`). |

Out of scope (use `rustc`-only host code, or wait for later stages): recursion
(needs stack frames — Stage 4), references / `&mut` params, `>3` params, slices,
`String`/`Vec`/`alloc`, floats, traits, `u32` *arithmetic* (`+ - * /`) and `u32`
params/returns (bitwise/shift `u32` works), variable shift amounts, closures, nested
struct *fields*. Anything unsupported is a **clear compile error** — that error is the
"this is host-only" budget detector.

## A whole program

```rust
// The canonical ZX screen-address math + a pixel plotter, in the dialect.
fn addr_of(x: u16, y: u16) -> u16 {
    16384u16 + (y / 64u16) * 2048u16 + (y % 8u16) * 256u16
        + ((y / 8u16) % 8u16) * 32u16 + x / 8u16
}
fn mask_of(x: u16) -> u16 {
    let masks = [128u8, 64u8, 32u8, 16u8, 8u8, 4u8, 2u8, 1u8];
    masks[(x % 8u16) as usize] as u16
}
fn main() {
    let a = addr_of(0u16, 0u16);
    poke(a, peek(a) | mask_of(0u16)); // light the top-left pixel
}
```

`samples/snake.rs` is a complete game (body in arrays, `match` steering, draw via
`poke`/`peek`) — the worked example end to end.

## Examples — run the language

Runnable demos in [`examples/`](./examples) each compile a dialect program (in
[`samples/showcase/`](./samples/showcase)), run it on the real `z80` CPU, print the
result, and check it against the same algorithm in plain rustc:

```bash
cargo run -p rustz80 --example sorting        # insertion sort  (arrays, break, for)
cargo run -p rustz80 --example sieve          # primes < 100    (byte arrays, nested loops)
cargo run -p rustz80 --example rpn_vm         # a bytecode VM   (arrays + match dispatch)
cargo run -p rustz80 --example state_machine  # vending machine (struct + enum + methods)
cargo run -p rustz80 --example rng            # 16-bit LCG      (wrapping_mul, ^)
cargo run -p rustz80 --example numerics       # gcd / isqrt / fib (while, return, loop)
cargo run -p rustz80 --example generics       # one generic source → 6 monomorphic instances
cargo run -p rustz80 --example const_generics # const-param array sizes (triangle$4, triangle$8)
cargo run -p rustz80 --example stack          # const-generic fixed-cap stack (Stack$4, Stack$8)
cargo run -p rustz80 --example points         # array of structs [Cell; N], a[i].x access
cargo run -p rustz80 --example pool           # fixed-cap entity pool (struct field [Cell; N])
cargo run -p rustz80 --example entities       # Entities<Cell, const N> — two instances ($4, $8)
cargo run -p rustz80 --example rng32          # 32-bit xorshift RNG (u32 in the HL:DE pair)
cargo run -p rustz80 --example structs        # generic struct + methods + a tuple field
cargo run -p rustz80 --example tuples         # multiple return values (HL/DE/BC)
cargo run -p rustz80 --example report         # per-function code-size report (instances + runtime)
cargo run -p rustz80 --example bitmap         # draw to screen RAM, printed as ASCII art
```

The `bitmap` demo prints what it drew straight from the framebuffer:

```
########
##......
#.#.....
#..#....
#...#...
#....#..
#.....#.
#......#
```

`tests/examples.rs` locks every showcase result, so a codegen regression fails
`cargo test` even without running the demos.

## Run headless — `rustz80-cell`

Compile and **run** a program on a flat-RAM Z80 — no ROM, no ULA, no I/O, no syscalls —
and get back a structured report: the result (`HL`), T-states spent against a budget,
code size, the symbol layout, the memory it touched, and whether it returned or hit the
budget. Deterministic, bounded, side-effect-free — a *micro-VM* you can hand a snippet
and measure (it's behind the `cell` feature, which pulls in the CPU):

```bash
cargo run -p rustz80 --features cell --bin rustz80-cell -- run samples/showcase/rng32.rs
```
```
entry      run @ 0x8000
result     11509 (0x2cf5)
cycles     16215 / 2000000 T-states
halt       returned
code       471 bytes, 1 functions
symbols    run@0x8000
memory     0x9000-0x9007 (8B), 0xffea-0xffef (6B)
```

`--entry NAME` picks the function (default `run`, else `main`), `--args a,b,c` passes
`u16`s in `HL`/`DE`/`BC` (decimal or `0x..`), `--cycles N` sets the budget, and `--json`
emits one machine-readable line:

```bash
cargo run -p rustz80 --features cell --bin rustz80-cell -- run samples/showcase/entities.rs --json
# {"entry":"run","result":2530,"cycles":14742,"halt":"returned","code_bytes":812,
#  "functions":6,"symbols":{"run":32768,"Entities$8::add":33158,...},"memory_touched":[[36864,36939],...]}
```

The runner is library API ([`rustz80::cell::run`] → a `Report`); the binary is a thin
shim. An infinite loop stops at the budget and reports `BUDGET EXCEEDED` rather than
hanging.

**Compile once, run many.** [`rustz80::cell::Runner`] owns a single bus and, between
runs, resets only the bytes the previous run wrote — so a warm run pays for the
computation, not a fresh allocation:

```rust
let mut cell = rustz80::cell::Runner::compile(src)?;
let a = cell.run(None, &[2, 3], budget)?;   // run with HL=2, DE=3
let b = cell.run(None, &[9, 4], budget)?;   // again — bus reset, no realloc
```

Benchmarked (`cargo bench -p rustz80 --features cell --bench cell`, Apple Silicon): a
trivial cell warm-runs in **~0.3 µs**, realistic snippets (`rng32`, `entities`) in
**~10–15 µs** after their one-time compile, and heavy compute loops emulate the Z80 at
**hundreds of × real-hardware speed**. Reuse cut the small-cell run cost ~60× vs a cold
one-shot (which was dominated by the 64 KiB bus allocation, not CPU work).

**Typed inputs + results + state.** A cell takes typed **inputs** (`run_with_inputs`, or
CLI `--set addr:ty=val`), returns all three result registers (`regs` = `[HL, DE, BC]`, so
a `-> (u16, u16, u16)` tuple reads back fully), and — because the bus stays live after a
run — exposes typed **state** read-back (`peek_u8/u16/u32`, `read_named`, CLI
`--read name@addr:ty`). `rustz80::struct_layout(src, "State")` gives each field's slot
offset, so a caller works in **field names, not raw addresses** — place a `State` struct
at a known base, set its fields, run a method on it, read its fields:

```rust
struct State { x: u16, y: u16, score: u16 }
impl State { fn run(&mut self) -> u16 { self.score = self.x + self.y * 10u16; self.score } }
```
```bash
rustz80-cell run state.rs --entry State::run --args 0xb000 \
    --set '0xb000:u16=3,0xb002:u16=4' --read 'score@0xb004:u16' --json
# … "result":43,"reads":{"score":43}
```

That closes the agent loop: **set typed inputs → run the cell → read typed output/state →
iterate** — source-shaped state, no Python/Docker/Wasm weight.

**Safe by default.** A `CellConfig` gates the intrinsics and caps resources: `poke`/`peek`
(raw memory) and `inport` (ports) are **capability-gated, off by default** for untrusted
cells, with `max_code_bytes` / `max_touched` ceilings on top of the deterministic cycle
budget. The CLI runs **sandboxed** unless you opt in (`--allow-raw-memory`,
`--allow-ports`, `--max-code-bytes N`, `--max-touched N`); `Runner::compile` stays
permissive for trusted/game code, or pass `CellConfig::sandboxed()` to
`Runner::compile_with_config`. A run reports *why* it stopped (`halt`: returned /
cycle-budget / memory-limit), so a model-generated program that misbehaves is rejected or
bounded, never a hang.

## The dial: one `impl Game`, two compilers

The headline. Write an ordinary [`speccy-sdk`](../speccy-sdk/README.md) `Game` and the
*same file* compiles **both** ways:

- **`rustc`** (host): a normal `impl Game for T { fn update(&mut self, …) }` — debug it.
- **`rustz80`**: `speccy-compile` detects the `impl Game`, routes `frame.*`/`input.*`
  to a **dialect prelude** (`Frame::pixel`/`clear` → screen pokes), lays the game
  state out as a zero-initialised global, and generates a frame loop
  (`EI; HALT; DI; CALL update` — interrupts on only for the 50 Hz sync, off during
  `update`). The output boots on the real ROM.

```bash
cargo run -p chuk-speccy-sdk --features compile --bin speccy-compile -- rustz80/samples/bounce.rs -o bounce.tap
cargo run --release --bin speccy-gui -- testroms/48.rom bounce.tap
```

`samples/bounce.rs` (self-playing) and `samples/move.rs` (**playable** — cursor keys
or QAOP move a blob) are exactly this; `tests/dial.rs` compiles each under rustc
*and* rustz80 and boots them, proving the dial. The pure prelude covers
`Frame::clear`/`pixel` and **real `Input::held`** (keyboard read via the `inport`
intrinsic, mapped like the SDK). Games stay in the dialect subset (fixed state, no
`Vec`/`String`).

```bash
cargo run -p chuk-speccy-sdk --features compile --bin speccy-compile -- rustz80/samples/move.rs -o move.tap
cargo run --release --bin speccy-gui -- testroms/48.rom move.tap   # then press 5/6/7/8 or Q/A/O/P
```

## How it works

- **Frontend** (`lower/`): `syn::parse_str` → accepted subset → typed IR (`ir.rs`).
  Unsupported nodes become errors. Split by concern: `vars` (the register file),
  `layout` (struct/enum layout + parse helpers), `prelude` (handle routing),
  `generics` (monomorphization), `expr`, and `stmt`; `mod.rs` owns the `Ctx` and the
  function-level orchestration.
- **Codegen** (`codegen.rs`): IR → Z80. Locals (incl. params) live in a per-function
  scratch region; expressions evaluate via `HL` + the stack; `*`/`/`/`%` `CALL` an
  appended `__mul16`/`__divmod16`.
- **Library API**: `compile_program(src) -> Program { code, symbols }`,
  `compile_fn(src) -> Vec<u8>`, `to_tap(code, org, entry, name)`,
  `compile_to_tap(src, entry, name)`. Code is laid out from `ORG = 0x8000`.
- **Tape boot**: `compile_to_tap` emits a `DI; CALL entry; EI; RET` trampoline at
  `ORG` and a BASIC autoloader (`CLEAR; LOAD "" CODE; RANDOMIZE USR`). The `DI` is
  load-bearing: the ROM's interrupt routine clobbers `BC`/`DE` (keyboard scan),
  which the codegen keeps live — so games run with interrupts off.

## Tests

```bash
cargo test -p rustz80                                   # differential + tap structure
SPECTRUM_ROM="$PWD/testroms/48.rom" \
  cargo test -p rustz80 -- --ignored                    # boot on the real ROM
```

- `tests/diff.rs` — the oracle: each `check!` runs one Rust block under `rustc` and
  through `rustz80` on the emulator and asserts they agree; plus multi-`fn` programs
  for generics, tuples, structs/methods, and control flow.
- `tests/snake.rs` — the whole dialect at once: a Snake checked against a Rust replica
  (state checksum + screen bitmap).
- `tests/examples.rs` — locks each `samples/showcase/` program's result (the demos in
  `examples/` run the same sources against a rustc oracle).
- `tests/coverage.rs` — the error/rejection arms, prelude routing, the frame-loop
  generator, and array-struct fields through `self` — the paths the above don't reach.
- `tests/tap.rs` — `.tap` structure, and ROM-gated boot/animation of `samples/snake.rs`.

Coverage (`cargo llvm-cov -p rustz80 --all-features -- --include-ignored`): **97% of
lines, 95% of regions**, every source file ≥ 90% on both.
