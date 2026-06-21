# `rustz80` — A Restricted Rust → Z80 Compiler (spec 07)

The pure-Z80 backend of the SDK dial ([spec 03](./03-sdk-spec.md)): author a game
in **imperative Rust** and compile it to a real `.tap`, no C and no external
toolchain. "Rust→Z80 compiler" spans a 100× effort range; this is deliberately the
small, finishable end — a compiler for a **restricted Rust dialect that is also
real Rust**, with a `syn` frontend, our own IR + Z80 codegen, and a hand-written
micro-runtime.

> **Status: planned.** This is the largest and riskiest single component in the
> project, and explicitly *escapable* — if it stalls, the host-Rust SDK (spec 03)
> and z88dk still ship games, so it is not a bet-the-project move.

---

## 0. The two traps to avoid

1. **An LLVM Z80 backend.** Vertical learning curve, perpetual fork maintenance,
   register-poverty codegen. The cautionary tale: building Rust's
   `compiler_builtins` for Z80 peaked at ~169 GB of RAM. Don't.
2. **Compiling real `core`/`std`.** Where the RAM blow-up and endless edge cases
   live. Instead: `#![no_std]`, no `alloc`, and a **~200-line hand-written Z80
   micro-runtime** for the few ops the subset needs (mul, div, shifts, memcpy).
   This one decision is the difference between impossible and solo-buildable.

What gets built: parse real Rust with `syn`, accept a bounded subset, lower to a
small typed IR, emit Z80, link the micro-runtime, write a `.tap`.

---

## 1. The prize: one source, both compilers

Because the accepted subset is valid Rust, the *same* `.rs` compiles two ways:

- `cargo check` / `cargo run` (**rustc**) → host execution, fast iteration, real
  debugger, and the option to reach host power (LLM/physics) — at which point it
  no longer compiles under `rustz80`, which is the **budget detector** working.
- `rustz80 build` → a pure `.tap` that runs on a real Spectrum.

This **upgrades the fidelity dial**: imperative Rust now spans it. The declarative
`GameDef`/host-composite SDK ([spec 03](./03-sdk-spec.md)) is no longer the *only*
portable path — the dial becomes "which compiler do I run," and the subset
boundary is literally the 1982-budget / 2026-capability line.

---

## 2. The dialect

**In** (maps to a register-poor 8-bit machine): `u8 i8 u16 i16` (and `u32/i32` via
runtime, expensive), `bool`, `char` as `u8`; `struct`, C-like / small-payload
tagged `enum`, tuples, fixed arrays `[T; N]`; `fn` with a defined ABI (§4),
`if/else`, `match`, `while`, `for` over ranges / array iter (→ counted loops),
`loop`/`break`/`continue`; `&T`/`&mut T` as 16-bit pointers; `static`/`static mut`
for state; `wrapping_*` / explicit-overflow ops.

**Out** (initially): heap / `alloc` / `Vec`/`Box`/`String`; `f32`/`f64`; trait
objects / `dyn` / vtables / capturing closures (non-capturing `fn` pointers OK);
most of `core` (a tiny supported prelude instead); generics (add via
monomorphization later, §7). Anything outside the dialect is a compile error with
a clear "not supported on Z80" message — which doubles as the host-only signal.

---

## 3. Architecture

```
  .rs ──syn──▶ AST ──lower──▶ IR (3-address, typed) ──codegen──▶ Z80 asm
                │                                          │
          type resolve                              peephole opt
          (lean on rustc, §5)                             │
                                          assemble (reuse z80 encode tables)
                                                          │
                                       link micro-runtime + SDK blit routines
                                                          │
                                                    write .tap
```

- **Frontend `syn`** — real syntax for free; source keeps rust-analyzer / rustfmt.
- **IR** — small typed three-address; keep it boring (the difficulty is codegen).
- **Codegen** — *not* graph-colouring allocation (the Z80's irregular register set
  punishes it). Use the proven small-Z80-compiler model: **`HL` accumulator, `DE`
  secondary, `A` for byte ALU, a fixed RAM scratch region as the virtual register
  file** (spill there). Emit straightforwardly, then a **peephole pass** — where
  most of the quality comes from on this target.
- **Assembler** — reuse the `z80` crate's encode tables; the **disassembler is the
  codegen debugger**.
- **Linker / appmake** — `.tap` *writing* (small; `.tap` reading already exists).

---

## 4. Calling convention (owned, documented)

Args: first `u8` in `A` / first `u16` in `HL`, second in `DE`, third in `BC`, rest
on the stack. Return: `u8` in `A`, `u16` in `HL`. Clobbers: caller-saved
`AF/HL/DE/BC`; callee preserves `IX/IY` (`IX` as frame pointer for locals/spills).
`self`: pointer in `HL` (methods = functions with a `*mut Self` first arg). Owning
the ABI is the point — tune it for the codegen.

---

## 5. Let rustc be the type/borrow checker

The biggest scope cut: **do not reimplement borrow checking, and lean on rustc for
type checking.** Because the source is real Rust, `cargo check` is the correctness
gate — types, lifetimes, borrowck, exhaustiveness, all free. `rustz80` then needs
only enough type resolution to choose instructions, and trusts well-typed input.
So it is a **Rust *backend* for a subset**, not a from-scratch compiler — the line
between one-person-sized and not. *(Later option: consume rustc MIR instead of
`syn` for zero divergence, at the cost of MIR complexity. Start with `syn`.)*

---

## 6. The micro-runtime (the tiny `compiler_builtins`)

Hand-written Z80, only what the dialect needs (~200 lines): `mul8`/`mul16`,
`div16`/`mod16`, multi-bit `shl`/`shr`, optional `u32` ops, `memcpy`/`memset` /
struct copy, and the ABI / prologue-epilogue helpers. Linked into every binary.
Codegen **strength-reduces** `* const` / `/ const` into shifts+adds and skips the
runtime for common cases.

---

## 7. Optimization — correct first, then peephole

1. Correct, naive codegen (ugly but right; validate via §8).
2. **Peephole** — redundant `LD` removal, `INC`/`DEC` folding, load-store
   collapse. Highest ROI on Z80.
3. Const-fold + strength-reduce (`*`/`/` by constants → shifts/adds).
4. Dead-code elimination, simple copy propagation.
5. Later: monomorphized generics, basic inlining, smarter scratch allocation.

Honest expectation: for a long while the output is **worse than hand-asm and
likely behind mature SDCC** on tight loops. That is fine — the win is *writing
games in Rust*; the inline-asm / eDSL escape hatch (§9) covers the few hot loops.

---

## 8. Testing — the emulator is the oracle

Differential testing is the spine, and it falls out of the dual-compilation
property:

```
for each test program (valid in the dialect):
    A) rustc → run on host → record (screen hash, state) over N frames
    B) rustz80 → load .tap on chuk-speccy (headless) → run N frames → record
    assert A_trace == B_trace
```

Divergence pinpoints a codegen bug. Per-feature unit tests (compile a snippet, run
on the emulator, assert registers/RAM) sit underneath. **Three pieces of the stack
already exist**: the Z80 encode tables, the disassembler (codegen debugger), and
the emulator (oracle) — a real head start over anyone starting a Z80 compiler.

---

## 9. Escape hatch

An `asm!`-like macro / eDSL emitting into the same object, so a game is mostly
restricted Rust with a couple of hand-tuned hot routines — like real embedded Rust.

---

## 10. Staging (value early, escapable)

| Stage | Build | Milestone |
|---|---|---|
| **0** | `syn` → IR → naive codegen for `u8/u16`, `fn`, `if/while`, arithmetic, array/static access; `.tap` writer | differential-test a "move a sprite" routine |
| **1** | `match`, `struct`/`enum`, the ABI, the mul/div micro-runtime | **compile Snake**, run on real hardware |
| **2** | peephole + const-fold + strength-reduce | output not-terrible |
| **3** | recognise the `Game` trait as the entry point; SDK prelude | the *same* `impl Game` compiles host (rustc) **and** pure (rustz80) |
| **4** | generics via monomorphization, inlining, smarter scratch | bigger games, better code |
| **5** *(opt)* | MIR frontend | zero divergence from real Rust semantics |

---

## 11. Where it lands

A new crate, **`rustz80`**, depending on the `z80` crate (encode tables) and
`syn`/`proc-macro2` (frontend). It reuses three things already owned — the **Z80
encode/decode tables**, the **disassembler** (codegen debugger), and the
**emulator** (test oracle) — which is the head start that makes the
restricted-dialect + `syn` + micro-runtime design the version that is actually
finishable.
