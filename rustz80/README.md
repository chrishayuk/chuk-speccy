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
| Types | `u16` (default) and `u8` (wraps at 256). `as u8` truncates, `as u16`/`as usize` widen. |
| Arithmetic | `+ - * / %`, `wrapping_add/sub/mul`. `*`/`/`/`%` use the appended micro-runtime. |
| Bitwise | `\|` `&` `^`. |
| Control flow | `if`/`else if`/`else`, `while`, `for` over integer ranges (`a..b` / `a..=b`, `for _ in`), `loop` / `break` / `continue`, early `return`; comparison conditions (`< <= > >= == !=`). |
| Arrays | `let a = [0u16; N];` / `[e0, e1, …]`; `a[i]`, `a[i] = v`. Index with `i as usize`. `[u8; N]` are byte-packed-per-slot with byte load/store. |
| Structs | `struct P { x: u16, y: u16 }` + literals + `p.x` read/write. Scalar fields only. |
| Enums + match | `enum Dir { Up = 1, … }` (explicit discriminants or `0,1,2,…`); `match` on integers/variants with `_`. Plus `bool` (`true`/`false`). |
| Functions + methods | Free fns and `impl T { fn m(&mut self, …) }` — up to 3 args in `HL`/`DE`/`BC`, result in `HL`; `self.field` through the receiver. |
| Raw I/O | `poke(addr, val)` / `peek(addr)` (memory) and `inport(port)` (I/O ports, e.g. the keyboard at `0xFE`). |

Out of scope (use `rustc`-only host code, or wait for later stages): recursion
(needs stack frames — Stage 4), references / `&mut` params, `>3` params, slices,
`String`/`Vec`/`alloc`, floats, traits/generics, closures, tuples, nested struct
*fields* (scalar and `[u16; N]` fields work). Anything unsupported is a **clear
compile error** — that error is the "this is host-only" budget detector.

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

- **Frontend** (`lower.rs`): `syn::parse_str` → accepted subset → typed IR
  (`ir.rs`). Unsupported nodes become errors.
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
  through `rustz80` on the emulator and asserts they agree.
- `tests/snake.rs` — the whole dialect at once: a Snake checked against a Rust replica
  (state checksum + screen bitmap).
- `tests/examples.rs` — locks each `samples/showcase/` program's result (the demos in
  `examples/` run the same sources against a rustc oracle).
- `tests/tap.rs` — `.tap` structure, and ROM-gated boot/animation of `samples/snake.rs`.
