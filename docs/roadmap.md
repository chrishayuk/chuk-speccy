# chuk-speccy — Roadmap

Single source of truth for what's built and what's next. The design is split
across eight specs ([README index](./README.md)); this tracks delivery against them.

**Status:** the **emulator core is feature-complete (M0–M8)** — a cycle-accurate,
ZEXALL-clean 48K Spectrum. On top of it, now **built**: the MCP server + autonomy
plane, a World-of-Spectrum game library, real-time `.tzx` loading, a disassembler,
the `ED FE` trap ABI, the Spectrum-native chatbot, and a native Rust game SDK
(Snake), and the `rustz80` compiler with a **full Snake written in the dialect** — compiled to Z80, run on the CPU, drawing to real screen RAM (differential-tested), a `.tap` emitter, and **the dial closed**: one `impl Game` source compiles under rustc (speccy-sdk) **and** rustz80 (a bootable tape that runs on the real ROM). The dialect has since grown into a **bounded data-structure language**: generics + const-generics, struct arrays / fixed-capacity pools (`Entities<Cell, const N>`), and a `u32` 32-bit xorshift RNG — all monomorphized, no heap, deterministic.
That language now also runs headless as **`rustz80-cell`** (B3) — a *deterministic agent microVM*: compile-once/run-many on a flat-RAM Z80, cycle-budgeted, capability-gated, with typed inputs + state read-back and a structured report. It defaults to a **Cell80** backend (B4) — a Z80 *superset* where `*`/`/`/`%` and `[v; N]` fills lower to `ED FE` host traps (native, no software runtime), while the authentic `Spectrum48` target keeps real-Z80 output. Benchmarked vs Wasmtime + Python (`cell-bench`): a realistic snippet warm-runs in **~0.24 µs** (**~0.05 µs batched**, via a decode-once fast executor) — ~18×/~4× off native-JIT Wasm but ~1070× smaller code (47 B vs 50 KB) and ~5× lower cold setup.
Plus **bit-exact `serialize_full` reset** (the RL gate), surfaced through PyO3 + MCP,
and the crates published (`chuk-speccy-*` libs, `speccy`/`rustz80` CLIs). Headline
next: the **authoring plane** ([spec 08](./08-speccy-kit-authoring-plane-spec.md)) —
*one typed source → three artifacts* (host build · pure `.tap` · agent env), bridged
by a compiler-emitted symbol map; and the **agent-microVM** track (typed I/O + an MCP
adapter over the cell). Then, in parallel: extra frontends (WASM), `rustz80` Stage 2
peephole (const-fold + strength-reduction already landed), and the accuracy tail (128K/AY).

---

## Architecture at a glance

```
  frontend heads ──┐
   terminal (TUI)   │
   native window    ├─▶ display crate ─▶ Spectrum (spectrum crate) ─▶ Z80 (z80 crate)
   (web / MCP …)    │     theme+filter      ULA · memory · ports        pure CPU,
                    ┘     pipeline          keyboard · tape · audio      no_std
```

Two foundations everything rests on: the **`Bus`/clock boundary** (CPU owns no
memory and no clock) and the **`Machine` observation surface** (the core emits raw
indexed framebuffers / registers / bytes and owns no presentation). Every head and
theme is a thin consumer of those; adding one is zero core change.

---

## Completed — core emulator (M0–M8)

| # | Milestone | Delivered | Verified by |
|---|---|---|---|
| **M0** | Workspace + `Bus` trait + `FlatBus` | `z80` (no_std) / `spectrum` / `display` / `frontend` / `z80-tests` crates | builds, smoke tests |
| **M1** | Z80 documented opcodes | base + CB + ED (block ops) + DD/FD/DDCB via X/Y/Z/P/Q decode; full documented flags | 26-case harness; **ZEXDOC 67/67** |
| **M2** | Undocumented set | MEMPTR/WZ, XF/YF, SCF/CCF **Q-quirk**, IXH/IXL, DDCB register-copy | **ZEXALL 67/67**, 0 CRC errors |
| **M3** | Memory map + ROM + INT | 48K map, system-ROM load, maskable interrupt (IM 0/1/2, HALT wake), per-frame `/INT`, `screen_text` | boots real ROM to **`© 1982`** prompt |
| **M4** | ULA video + keyboard | indexed framebuffer (raw obs), standalone **`display`** crate (themes: palette remap / duotone ramp + scanline effect), 8×5 matrix + host-key table | typing `PRINT 6*7` → `42` |
| **M5** | Snapshot loading | `.sna` load **+ save** (checkpoint primitive), `.z80` v1/v2/v3 RLE; `read_/write_memory` | runs **Manic Miner** (real `.z80`) |
| **M6** | Beeper audio | ULA records port-0xFE bit-4 edges, box-filters to host samples (`enable_/drain_audio`); `cpal` ring-buffer in the window head | Manic Miner **title tune** oscillates |
| **M7** | Contention | precomputed `[u8;69888]` stall table on bottom-16K accesses + M1 fetch; ZEXALL still clean; runtime `contention_enabled` toggle for A/B timing | contended vs clean T-state delta |
| **M8** | `.tap` tape | block parser + ROM `LD-BYTES` trap (`0x0556`) fast-load; both heads accept `.tap` | auto-running `BORDER` tape via real ROM |

### Heads (spec 05) shipped alongside
- **Terminal (TUI):** live 50 Hz loop, truecolor **quadrant** block glyphs (2×2 px/char, exact per-cell colour), aspect-correct fractional sampling (queries `CSI 16 t`), opt-in sextant, ASCII fallback for pipes. Themes: `authentic`/`dark`/`light`/`terminal`/`amber`/`gameboy`.
- **Native window (`speccy-gui`, winit + softbuffer):** pixel-perfect 256×192, aspect-correct + letterboxed, real key up/down, cpal sound with **audio-driven frame pacing** (emulation refills the ring to ~3 frames, so it tracks the real-time audio clock instead of the jittery video refresh — no underrun, stable beeper pitch). A real app shell with **native menus** (muda): a *Machine* menu (Save/Load Snapshot via native file dialogs, Reset), a *View* menu / F11 / the macOS green button toggle **full screen** at runtime (any display), and an *Audio* menu switches the **output device live** (e.g. an AirPlay/TV speaker when projecting). Accepts a **game title** (fetched from World of Spectrum) as well as a file.

### Test inventory
- `z80-tests`: 32 unit + ZEX harness (`run_zex`, CP/M BDOS trap) + a disassembler
  suite (golden per family, all-opcode/all-prefix fuzz, CPU length cross-check).
- `spectrum`: unit tests for contention, beeper, `.sna` round-trip, `.tap` trap,
  recording, the real-time tape engine (TAP/TZX pulse encodings + EAR state),
  `disassemble`, the host-trap ABI (`FnTable` mul16, math, unknown-id carry,
  NOP-without-host), and `deserialize_full` garbage-rejection; ROM-backed:
  `boots_to_copyright`, `types_basic_and_evaluates`, `title_music_makes_sound`,
  `tap_loads_and_autoruns_basic`, `chat_terminal_round_trip`, and
  `serialize_full_is_bit_exact` (restored machine evolves identically for 300 frames).
- `display`: theme/effect/border. `wos`: matching/encoding + a network fetch (ignored).
- `speccy-sdk`: screen-interleave + tile/attr + input-edge units; ROM-backed Snake
  render + 600-frame long-run.
- `chuk-mcp-spectrum` (Python): surface split, autonomy, record-with-audio, rewind,
  disassemble, host-trap ABI + guard, the `CHAT_*` protocol, and a WoS search/load.
- All warning-/clippy-clean. ROM-backed tests gated behind `SPECTRUM_ROM` /
  `SPECTRUM_GAME` env (ROMs gitignored under `testroms/`); network tests `#[ignore]`.
- Diagnostics: `spectrum --example audiodiag` reports the beeper's dominant pitch
  (contention on vs off; finding: negligible effect — the toggle is an A/B aid).

---

## Next — layers on top

### A. MCP server (spec 02) — **built**
The core loads, runs, observes, is driven, and records — every tool is a thin
wrapper. Lives in `../zxspec_py` (PyO3) + `../chuk-mcp-spectrum` (server, on
`chuk-mcp-server`). The tool catalog and recording were first built flat, then
restructured into the agent/admin two-endpoint model — see **A2**.
- [x] `zxspec_py` PyO3 `Machine` over the `spectrum` crate (maturin wheel, abi3-py311):
  registers/memory, screen (rgba/indexed/text), step/run/run_until, snapshots
  (`.sna`/`.z80`), tape, keyboard (`press`/`type_text`), audio, and **session
  recording** (frames captured at the `run_frame` chokepoint in the core).
- [x] **Recording → MP4** (H.264 + AAC) with beeper sound, encoded host-side
  (imageio/ffmpeg), downloadable.
- [x] **Game library** — search **World of Spectrum** (ZXInfo API) and download +
  unzip a loadable `.tap`/`.z80`/`.sna`. Shared Rust **`wos`** crate, so it works
  on the **CLI** (`speccy-gui <rom> "Jet Set Willy"`) *and* the MCP (admin
  `search_games`/`load_game`). 48K-build preference; `.tzx`/custom-loader games
  load in **real time** (see the accuracy tail), so the Dizzy series etc. work.
  The `speccy-library` bin headlessly verifies a set of classics in one command.
- [ ] `set_display(preset)` — expose the `display` crate themes so an agent can re-theme + screenshot.
- [x] **Disassembler** — `z80::disassemble` (a pure read-only mirror of the
  decoder: prefixes, `(IX+d)`, DDCB, ED block ops + undocumented slots; absolute
  JR/DJNZ targets). Surfaced as `Spectrum::disassemble`, `zxspec_py`, and the MCP
  `disassemble` tool (agent + admin). Tested by golden + all-opcode fuzz + a CPU
  length cross-check.
- [ ] `trace` / breakpoints (`StopReason::Breakpoint` already exists in the core).
- [x] **Bit-exact `serialize_full()`/`deserialize_full()`** (was the open decision in
  [MCP spec §10](./02-mcp-server-layer-spec.md#10-open-decision-pyo3-boundary)) —
  captures *everything* that affects execution (CPU incl. `wz`/`q`/`q_prev`/`iff`/
  `im`/`ei_pending`/`halted`, full 48K RAM, ULA frame phase + border + beeper/audio
  carry, keyboard matrix + EAR); ROM and the contention table are constants, host/
  recording are runtime. ROM-gated test proves a restored machine evolves
  **bit-for-bit identically for 300 frames**. This is the precondition for the RL
  env (E): reset is now a non-source-of-variance. (Next: surface it through PyO3/MCP.)

### A2. Roles & autonomy (spec 06) — **built** (on `chuk-mcp-server`)
Rebuilt the MCP layer on `chuk-mcp-server` (pydantic-native): **two endpoints**
over one shared `Supervisor`.
- [x] **Two endpoints** — `agent` (8 tools, observe + drive, implicit session) and `admin` (20 tools, everything). Small agent surface = little context.
- [x] **Implicit session** via `get_session_id()`; agent tools take no `machine_id`, admin tools take explicit `session_id` across all sessions.
- [x] **Autonomy plane** (`Supervisor`): provision-per-session, **record-by-default** → MP4 (H.264 + AAC) with snapshot-cadence checkpoints (`restore_snapshot` to rewind), idle reaping. All env-configurable.
- [x] **Artifacts → VFS** when an artifact store is configured (downloadable), local-file + base64 fallback. `read_only_hint`/`destructive_hint` on every tool.
- [ ] **Event-based snapshots** (watch a score/lives address) in addition to time-based.
- [ ] **Wall-clock cadence** for the real-time path.
- [ ] **Cross-process live control** (proxy) — today separate processes share metadata/artifacts via the framework's multi-server store; co-host (`serve.py`) for shared live machines.

### B. SDK / developer kit (spec 03)
- [x] **Native Rust game SDK** (`speccy-sdk`, host-composite backend) — author a
  game as one `Game::update(&Input, &mut Frame)`; the Z80 is an ~11-byte frame
  pump (`di/im1/ei/halt/HOSTCALL 0x60/jr`) and all logic + rendering is host Rust.
  `Frame` rasterises 1-bit pixels + attrs into the interleaved 6912-byte screen;
  `Input` reads the matrix via the trap; the ROM font drives `text`. **First light:
  Snake** — `speccy-gui <rom> snake` (also a headless render + long-run test).
  Composes with every head/MCP/recording for free. This is the **host-composite**
  backend of the fidelity dial; the **pure-`.tap`** backend is the `rustz80`
  compiler (**B2**), and a z80-native blitter backend is a later option.
- [ ] **L0** toolchain: one-command source → `.tap` → run-in-emulator; PNG→Spectrum asset pipeline.
- [ ] **L1** framework over z88dk (sprites clash-aware + mono, tilemap, input, beeper SFX, fixed-point, RNG).
- [x] **L2** trap ABI — `ED FE` (`HOSTCALL`, id in `A`) → defaulted `Bus::host_trap`
  → `spectrum::host` registry (`HostCalls`/`HostCtx`/`FnTable`) → PyO3 bridge
  (`register_host_dispatcher`, with a liveness-guarded `TrapCtx`). NOP on bare
  hardware (the fidelity dial), `HOST_PRESENT` probe, disassembles as `HOSTCALL`.
  Tested in Rust (`FnTable` mul16) and Python (round-trip + guard + both ways).
- [x] **L2 math handlers** — `spectrum::host::math_traps()`: `0x10 MUL16`,
  `0x11 DIVMOD16` (carry on ÷0). Composable via `FnTable::with_fallback`, so Rust
  math + a Python chat handler share one dispatcher (`register_host_dispatcher(cb,
  with_math=True)` / `install_math_traps`).
- [ ] **L3** showpiece: one app calling an MCP server through a trap.

### B2. `rustz80` — restricted Rust → Z80 compiler (spec 07) — **bounded data structures built (Stages 0–4h)**
The pure-`.tap` backend of the SDK dial: author a game in **imperative Rust** and
compile it to a real Spectrum binary — no C, no external toolchain. A compiler for
a restricted Rust dialect that is *also real Rust*, so the **same source compiles
under rustc (host, fast iteration) and under `rustz80` (pure)**. That upgrades the
dial: imperative Rust now spans it, and the subset boundary *is* the 1982-budget /
2026-capability line (reach for an LLM/host physics and it won't compile pure).
The largest, riskiest component — and **escapable** (the host-Rust SDK + z88dk
still ship games if it stalls). The decisions that keep it solo-sized are realised:
- [x] **Stage 0 (proof of life)** — `rustz80::compile_fn(src)`: `syn` frontend →
  own typed IR → naive Z80 codegen for `u16` locals, `+`/`-`, `if/else`, `while`,
  comparison conditions. `HL` accumulator, `DE` secondary, a RAM scratch "register
  file", label-patched jumps. Unsupported nodes (e.g. `f32`) are a clear compile
  error — the host-only signal.
- [x] **`syn` frontend + own IR + own codegen** — *not* an LLVM backend, *not* real
  `core` (sidesteps the `compiler_builtins` 169 GB trap; mul/div micro-runtime is Stage 1).
- [x] **rustc is the type/borrow checker** — the subset is real Rust, so this is a
  Rust *backend* for a subset, not a from-scratch compiler.
- [x] **Emulator is the oracle** — differential testing is the spine: each test block
  runs under rustc (host `fn`) *and* through `rustz80` onto our Z80, asserting they
  agree (one source, both compilers; `7*6` via repeated addition already matches).
- [x] **Stage 1a** — the **calling convention** (params in `HL`/`DE`/`BC`, multi-`fn`
  programs via `compile_program`, calls, per-function scratch regions) and a
  **mul/div/rem micro-runtime** (`__mul16` shift-add, `__divmod16` restoring),
  each differential-tested (`add`/`sq`/3-arg compose, sum-of-squares, `1000%7`).
- [x] **Stage 1b (arrays)** — fixed arrays `[T; N]`: `[v; N]` / `[e0, …]` init and
  element read/write with runtime indices (element-address arithmetic + indirect
  `u16` load/store); `as` casts are no-ops (all 16-bit), so `a[i as usize]` is valid
  host Rust. Differential-tested incl. an in-place reverse.
- [x] **Stage 1c (structs)** — `struct` defs + literals + scalar field read/write;
  every field has a constant offset, so `s.field` lowers to a plain slot (zero
  codegen change). Composes with functions. Differential-tested (`Point` mutate →
  1308; `area(a.x,a.y)+area(b.x,b.y)`). Array/nested fields are a clear error for now.
- [x] **Stage 1d (enum/match)** — C-like `enum`s (variant = integer constant) and
  `match` lowered to an if-chain over a scrutinee temp (literal + variant patterns
  + `_`). Lowering-only, no codegen change. Differential-tested (enum match → 200;
  literal/enum-param match → 162).
- [x] **Stage 1e (byte arrays)** — `[u8; N]`: byte load (zero-extend) / store,
  element width inferred from the `u8` literal suffix; `x as u8` truncates to the
  low byte. Differential-tested (`300 as u8 = 44`, fill/sum). (u8 and u16 arrays
  share 2-byte slots; only the access width differs.)
- [x] **Stage 1f (scalar u8)** — `u8` type tracking (literals/params/casts/vars),
  masked arithmetic (wrap at 256), `wrapping_add/sub/mul`, and `x as u8` truncation.
  Differential-tested (`200+100→44`, `10−20→246`, `20*20→144`, a u8 loop counter).
- [x] **Stage 3a (raw memory + bitwise)** — `poke`/`peek` raw-memory intrinsics
  (their host defs are prelude-only and skipped) + bitwise `|`/`&`/`^` + a discard
  `Eval` for void calls. A `plot(x, y)` written *in the dialect* (div/mod screen
  math + a mask table) now compiles to real **screen-RAM pokes** — verified by
  running it on the CPU and comparing the bitmap against the canonical ZX address
  formula computed independently.
- [x] **dialect Snake (the payoff)** — a real Snake written in the dialect (body in
  `bx`/`by` arrays, `match` steering with wrap, tail-erase + head-draw via
  `set_px`/`clr_px` over `poke`/`peek`), compiled by `compile_program`, run on the
  CPU, and **differential-tested** against a Rust replica — final-state checksum
  *and* the screen bitmap (`0x4000..0x5800`) match byte-for-byte across 0..64 steps;
  exactly the 6-cell body stays lit. The whole dialect exercised at once.
- [x] **`.tap` emitter + `speccy-compile` CLI** — wrap compiled code in a BASIC
  autoloader (`10 CLEAR org-1: LOAD "" CODE: RANDOMIZE USR entry`) so a dialect
  `.rs` becomes a **bootable tape** (`speccy-compile game.rs -o game.tap`, then load
  in `speccy-gui`). Proven end-to-end: a dialect program loaded via the **real 48K
  ROM** tape loader auto-runs and executes (sentinel + screen poke verified; ROM-
  gated test). So a rustz80 game now boots on the actual machine — the dial closed
  through to hardware.
- [x] **Stage 3b.1 (methods + references)** — `impl T { fn m(&mut self, …) }`,
  `self.field` (indirect through the receiver pointer), and `obj.m(args)` lowering to
  `T::m(&obj, …)` (`self` as a leading pointer arg; method names mangled `T::m`).
  Differential-tested. The machinery `impl Game` needs.
- [x] **Stage 3b.2 (the dial, closed)** — `compile_game` recognises `impl Game for T`,
  routes `Frame`/`Input` methods to a **dialect prelude** (`__frame_pixel`/`__frame_clear`
  over `poke`/`peek`), and generates a frame-loop entry (`EI; HALT; DI; CALL update` —
  interrupts on only for the 50 Hz sync, off during `update`). `samples/bounce.rs`
  compiles **both** under rustc (a `speccy-sdk` `Game`) and rustz80 (a bootable tape);
  the dial test (`tests/dial.rs`) compiles it both ways and boots it on the real ROM.
  Also: `inport` intrinsic, explicit enum discriminants, `bool` literals, bool-expression conditions, `use` skipped. `samples/move.rs` is playable (keys move a blob). (Recursion needs stack frames — Stage 4.)
- [x] **Stage 3c (bounded control flow)** — `for` over integer ranges (`a..b` / `a..=b`,
  `for _ in`, variable bounds evaluated once), `loop`, `break`, `continue`, and early
  `return`. Lowering desugars `for` to a counted loop whose induction step *is* the
  `continue` target (so `continue` advances, not spins); codegen gains a loop-label
  stack (`break`→exit, `continue`→step/cond) and a per-function epilogue label for
  `return`. `break`/`continue` outside a loop, `break <value>`, and labels are clean
  rejections. Differential-tested (`for`/inclusive/nested/array-index, `loop`+`break`,
  `while`/`for`+`continue`, `loop`+`return` rejection-sampling). The dialect
  `samples/snake.rs` is rewritten on `for`/`loop` and still boots + animates on the
  real ROM. (Closes the `loop`/`for` blocker for the pure-Snake seam.)
- [x] **Stage 4a (generic functions)** — real generic free fns (`fn max<T: Ord +
  Copy>(…)`) monomorphized per call: the type arg comes from a turbofish or is
  inferred from the matching argument's width, and the instance's params are declared
  at that concrete width, so the body lowers like any function (u8 instances mask, u16
  don't). Generic-calls-generic instantiates transitively off a worklist (`clamp` →
  `max`/`min`). Lowering-only — instances are extra named functions (`max$u16`). A
  runnable `examples/generics` shows one source → six instances. **Generic structs +
  methods** (`struct Pair<T>` / `impl<T> Pair<T>`) too — type arguments erased to 16-bit
  (one shared layout, like any struct's fields), so no per-instance struct codegen.
  (Const generics + struct-element arrays — what `Entities<T, N>` *also* needs — remain
  pending.) Also: `lower.rs` split into a `lower/` module tree.
- [x] **Stage 4b (tuples → multiple return values)** — `fn divmod(a, b) -> (u16, u16)
  { (a / b, a % b) }` returns its tuple in `HL`/`DE`/`BC` (up to three), destructured
  at the call site with `let (q, r) = …` (a tuple literal or a call). Lowering-only
  for the destructure (`Stmt::AssignTuple` distributes the result registers into
  slots); codegen gains a tiny multi-value `gen_return`. Differential-tested (divmod,
  swap-`minmax`, a 3-tuple) + a runnable `examples/tuples`. (Closes the tuples blocker
  for the pure-Snake seam.)
- [x] **Stage 4c (const generics on functions)** — `fn buf<const N: usize>()`
  monomorphized per `::<N>`: a const argument (turbofish — it can't be inferred) sizes
  a local `[u16; N]` array and substitutes as a plain value (loop bounds, comparisons).
  Reuses the `Mono` worklist (a generic param is now type *or* const; an instance key
  carries widths and values, e.g. `triangle$4`). Differential-tested + a runnable
  `examples/const_generics`.
- [x] **Stage 4d (const-generic structs)** — `struct Buf<const N: usize> { data: [u16;
  N], len: u16 }` with `impl<const N: usize> Buf<N> { … }`, monomorphized per `N`: each
  instance gets a **per-instance layout** (the `[u16; N]` field sized by `N`, registered
  on demand in `Mono::struct_instances`) and **per-instance methods** (`Buf$8::push`,
  lowered with `self` typed as `Buf$8` and `N` substituted). `N` is inferred at the
  struct literal from the array field's length; array fields can now be initialised in a
  literal (`[v; N]` / `[e0, …]`). Differential-tested with a capacity-bounded
  `Stack<N>` + a runnable `examples/stack` (instances `Stack$4`/`Stack$8`). rustz80 stays
  ≥90% line/region per file.
- [x] **Stage 4e (struct-element arrays)** — `let a = [Cell { … }; N]`: array elements
  are now multi-slot, so element access computes an address `&a + index*stride
  (+ field_off)` via three general IR nodes (`MulConst`/`LoadAt`/`StoreAt`; a power-of-
  two stride shifts, else `__mul16`). `a[i].x` read/write and whole-element `a[i] = Cell
  { … }`; array fields can now also be initialised in a struct literal. Differential-
  tested (`[Cell; 4]` filled at runtime indices, a field overwrite, runtime-index reads)
  + a runnable `examples/points`.
- [x] **Stage 4f (struct-field struct arrays)** — a struct *field* that is an array of
  structs: `struct Body { cells: [Cell; N], len: u16 }` with `impl Body { … }`.
  `field_target` now carries the field's element struct; a unified `array_base` yields
  the field's byte base (`self_ptr + off` through the receiver, or the slot address by
  value), so `self.cells[i].x` (read/write) and `self.cells[i] = Cell { … }` work, and a
  `[Cell; N]` field is initialised in the struct literal (`[Cell { … }; N]` / `[c0, …]`).
  Differential-tested with a `Body`/pool (`push` + `checksum` through `self`, plus
  by-value `b.cells[0].x`) + a runnable `examples/pool` — the **`Entities<Cell, N>` shape
  for a non-generic capacity**.
- [x] **Stage 4g (the `Entities<Cell, const N>` combo)** — a *const-generic* struct
  whose field is an array of structs (`data: [Cell; N]`). The only change needed: thread
  the regular struct layouts into `instantiate_struct`, so a const-generic instance's
  `[Cell; N]` field sizes correctly (`N` from the const map, `Cell` from the layouts);
  A3b's `array_base`/`field_target` already work on the per-instance layout. Methods
  bound on `N`, store whole elements (`self.data[i] = Cell { … }`), and read element
  fields. Differential-tested (`add`/`checksum`, `N`-bounded, `N` inferred from the
  literal) + a runnable `examples/entities` (two instances, `Entities$4`/`Entities$8`).
  **The fixed-capacity entity pool now compiles + runs** — the last *structural* blocker
  for a pure Snake.
- [x] **Stage 4h (`u32` — the RNG core + shifts)** — a `u32` is a two-slot value computed
  in the `HL:DE` pair by a dedicated `gen_expr32`; new IR (`Lit32`/`Var32`/`Bin32`/
  `Shift32`/`Trunc32` + `Assign32`). Supports `^ & |`, constant `<<` / `>>` (incl. across
  the word boundary), and `as u16`/`as u8` truncation — and adds `<<` / `>>` for `u16`
  too (previously rejected). Differential-tested with a real **xorshift32** step (`x ^= x
  << 13; x ^= x >> 17; x ^= x << 5`) + bitwise/truncate + `u16` shifts; a runnable
  `examples/rng32`. *(Deferred: `u32` `+ - * /` / `%` — needed by `Rng::below(n) = next %
  n`; `u32` params/returns; variable shift amounts.)*
- [ ] **Stage 4i (pure Snake finish)** — the only blockers left are *non-structural*,
  and mostly **SDK-side, not compiler-side**:
  - `Rng::below` needs `u32` `%` — or just a power-of-two `below` mask in the dialect `Rng`.
  - **`Frame::tile`/`text` are an SDK concern, not a `rustz80` one.** Prelude routing is
    already generic (`PreludeConfig`: `(handle, method) → fn`); the SDK supplies the
    dialect prelude fns + routes (e.g. `__frame_pixel`), and `Frame::pixel`/`clear` work
    exactly this way. Adding `tile`/`text` is the same SDK pattern — *if* their args are
    expressible. `pixel(x,y,on)` passes values (fine); `tile(&Tile)`/`text(&str)` pass a
    **reference/string**, which the handle convention (≤3 value args, receiver dropped)
    can't carry. Resolve SDK-side with value args — pass the tile **data by address**
    (`__frame_tile(addr, cx, cy)` reads 8 bytes + pokes; tile bytes live as a `const`),
    `text` likewise from `(addr, len, cx, cy)`. The one *general* (non-game) compiler
    feature that would help: lower a `&str`/`const [u8; N]` literal to a const data blob +
    its address — cleaner than full references, and reusable beyond games.
- [ ] **Stage 2+**: peephole + const-fold/strength-reduce; recognise `impl Game`
  (same source host + pure); generics via monomorphization; optional MIR frontend.
  Inline-asm / eDSL escape hatch for hot loops.

### B3. `rustz80-cell` — a deterministic agent microVM — **runner + typed read-back built**
The const-generics + struct-element-array work has quietly changed `rustz80`'s
category: it is no longer just a "Rust-shaped Z80 game compiler" but a **bounded,
Rust-shaped data-structure compiler for a tiny deterministic VM** — fixed-capacity
generic containers (`Stack<N>`), struct arrays / object pools (`Entities`-shape), typed
compound state, and a symbol-map-visible layout. No heap, no OS, no syscalls — just
bounded memory and deterministic execution. That is the minimum language layer for a
**microVM agents can safely program against**: *restricted Rust in → monomorphized
bounded Z80 out → deterministic execution → typed symbols/state out.*
- [x] **A non-Spectrum headless runner** — `rustz80-cell run scratch.rs [--entry run]
  [--cycles N] [--args a,b,c] [--json]` over a flat-RAM `z80::Bus` (no ROM, no ULA, no
  I/O), returning a structured `Report`: `result` (`HL`), `cycles` used vs. budget,
  `code_bytes` + function count, the symbol map, `memory_touched` (coalesced write
  ranges), and `halt` (returned vs. budget-exceeded). Lives in `rustz80::cell` (library
  API + a thin `rustz80-cell` bin) behind the `cell` feature so the compiler stays
  dependency-free; an infinite loop stops at the budget instead of hanging. A "safe
  executable thought bubble" for agents: deterministic, bounded, inspectable, no side
  effects. Tested (`tests/cell.rs`).
- [x] **Benchmarked + compile-once/run-many** — `benches/cell.rs` measures per-cell
  latency + emulated throughput. Baseline showed the per-run floor was a fresh 64 KiB
  bus allocation, not CPU work; `Runner::compile(src)` → `runner.run(…)` now owns one
  bus and between runs **resets only the bytes the last run wrote** (an O(touched) reset
  via a distinct-write list). Result (Apple Silicon): a trivial warm run dropped
  ~20 µs → **~0.3 µs** (≈60×), realistic snippets (`rng32`/`entities`) ~30–35 µs →
  **~11–15 µs**; heavy loops emulate at 300–600× real-hardware speed. Reuse is
  bit-deterministic (same args → same result, T-states, and memory diff). The warm
  run cost is now the computation, not the setup.
- [x] **Cycle/byte budgets as first-class output** — `cycles` (from the `tick` clock)
  and `code_bytes`/`functions` (from `Program::size_report`) are in every `Report`, so
  an agent sees the cost of what it wrote.
- [x] **Reads back results + typed state** — the `Report` carries the symbol map (name →
  address, incl. instances + runtime), `memory_touched`, **all three result registers**
  (`regs` = `[HL, DE, BC]`, so a tuple return reads back fully), and — since the bus
  stays live after a run — **typed named state** decoded from memory: `Runner::peek_u8/
  u16/u32` and `read_named(&[(name, addr, Ty)])`, surfaced on the CLI as
  `--read name@addr:ty,...`. The `(name, addr, ty)` layout is the caller's, so it composes
  with the B2/E state-struct symbol map (`score@0xb000:u16` → `score=4`). This closes the
  agent loop: write code → run the cell → read typed output/state → iterate. *(Next: a
  convenience that derives the read layout straight from a state struct's emitted
  `.sym` map, so fields are named automatically.)*

**Product shape — three layers, one core.** `rustz80-cell` is *a safe executable
scratchpad for agents*: compile a tiny Rust-shaped program into a bounded Z80 cell, run
it deterministically under a cycle budget, return typed results + cost + memory effects +
(later) a trace. Keep it layered — **don't make MCP the core**:

- `rustz80-cell-core` — the library API (today: `rustz80::cell`; later its own crate),
  embeddable with no CLI/MCP assumptions. Used by the CLI, MCP, benches, the SDK, tests.
- `rustz80-cell` — the native CLI / local scratchpad.
- `chuk-mcp-cell` — a thin MCP adapter exposing cells to agents.

The first niche isn't replacing Wasm/Python; it's replacing *"let the agent run arbitrary
code to check something small"* with *"let the agent run tiny bounded code in a
deterministic cell."* More inspectable than Wasm (source-shaped typed state, not linear
memory), lighter than a container, constrained enough that models generate it reliably.

**Positioning — keep the claim narrow.** *Not* "faster than Wasm" — Wasm JITs to native
and wins decisively on real compute. The claim is: **for tiny agent-generated programs,
the cell is a smaller, more inspectable, deterministic sandbox** — *a thought bubble, not
a runtime.* Latency only needs to be cheap enough to call in a loop (it is: realistic
snippets warm-run in single-to-low-tens of µs); the differentiators are determinism,
source-shaped typed state read-back, capability gating, a cycle budget, and a sandbox
surface you can hold in your head (64K, no WASI/imports). The honest proof is the cross-runtime **comparison benchmark** (`cell-bench/`): native
Rust (floor) · Wasmtime warm · `rustz80-cell` warm · Python subprocess, scoring 1000
candidates. Measured (Apple Silicon): warm per-call **native 0.001 µs · Wasm 0.013 µs ·
cell 0.24 µs (`run_fast`) / 0.05 µs (`run_many_fast`, decode-once) · Python ~37 µs**; cold
setup **Wasm 3.0 ms · cell 0.59 ms (≈1 µs from a cached image) · Python ~35 ms**; code size
**cell 47 B vs Wasm 50 KB**. So Wasm wins warm compute (~18× per-call, ~4× on the batch hot
path), but the cell sets up ~5× faster, is ~1070× smaller, and runs **~4–20 M evals/s** —
well inside "call it in a loop." The niche holds: *not faster than Wasm — smaller,
lower-setup, more inspectable, deterministic, for the tiny-snippet class.*

**Phased plan** (✓ = done; → = next):

- [x] **P1 · Warm execution** — compile-once/run-many `Runner`, O(touched) reset (above).
- [~] **P2 · Run modes** — `Runner::run_fast` (just regs + cycles + halt, **no per-call
  allocations** — no symbol-map clone / size report / memory-diff coalesce) splits the hot
  path from `run`'s rich `Report`. `run_many_fast(entry, &arg_sets, budget)` is the batch
  hot path: it resolves the entry once and, for a **straight-line** cell, **decodes it once
  and replays on a stripped native-register executor** (no per-instruction
  fetch/contention/refresh/flag work) — the cycle count is input-independent so it comes
  from one authentic calibration run, and results stay differential-checked against the
  authentic interpreter; non-straight-line cells fall back transparently. Lifecycle bench
  (cell-bench): per-call overhead floor **~0.06 µs**, single `run_fast` score **0.25 µs**,
  `run_many_fast` **~0.05 µs** (~5×, ~4× off native-JIT Wasm). *Next:* lazy flags +
  conditional jumps so looping cells fast-path too; `run_trace` for the debug tier.
- [x] **P3 · Full register capture** — `regs = [HL, DE, BC]`; tuple returns read back.
- [x] **P4 · Typed I/O** — typed *read-back* (`read_named`/`--read`) **and typed inputs**
  (`Runner::run_with_inputs`, CLI `--set addr:ty=val`, written after the reset + cleaned
  next run). `rustz80::struct_layout(src, name)` exposes each field's slot offset (the ABI
  primitive), so a caller resolves **field names → addresses** (`base + offset*2`): place a
  `State` struct at a base, set its fields, run `State::run(&mut self)`, read its fields —
  the full named loop, differential-tested. *(Next: a one-call convenience that does the
  name↔addr resolution for a named state struct; `memory_diff` values, not just ranges.)*
- [~] **P5 · Native CLI + compiled artifact** — the **compile/instantiate split** landed:
  `CellProgram::compile(src)` (the parse-dominated cold cost — `cell-bench` shows cold
  setup is ~90% syn parse, ≈16 µs; bus alloc is amortized-free) is now separate from
  `Runner::new(&program)`, which instantiates a fresh machine in **~1.2 µs** (no re-parse,
  ~16× cheaper; vs Wasm's ~3 ms JIT, ~2500×). So a cached `CellProgram` makes re-running a
  known snippet's cold setup effectively vanish. **The image format landed:**
  `CellProgram::to_bytes()` / `from_bytes()` is a compact self-contained cartridge (code +
  symbols + policy, no syn — **71 bytes** for the score cell) that reloads + runs in
  ~1.2 µs (16× under compiling the source) — cache by hash, ship, index. And a **`CellPool`**
  recycles the 64 KiB bus across cells of any program, so a *disposable* cell (acquire +
  run + release) costs **~0.38 µs** instead of ~1.06 µs cold — the "spawn N short-lived
  cells" path. *Next:* surface it on the CLI — `compile` (source → `.cell`), `exec`
  (precompiled image), `inspect` (symbols/size/helpers) — and stamp the artifact with a
  source hash + compiler/ABI version.
- [ ] **P6 · MCP server** — `chuk-mcp-cell` over the core: `cell.compile`, `cell.run`,
  `cell.compile_and_run`, `cell.inspect`; then cached-runner sessions
  (`compile → cell_id`, `run_cell(cell_id, args)`) for warm-path agent performance.
- [x] **P7 · Safety / capabilities** — a `CellConfig` with **capability-gated
  `poke`/`peek` (raw memory) + `inport` (ports), off by default** (a `syn`-visitor scan
  rejects them at compile unless allowed), plus `max_code_bytes` (compile-time) and
  `max_touched` (run-time abort) ceilings on top of the deterministic cycle budget. The
  CLI is **sandboxed by default** (`--allow-raw-memory`/`--allow-ports`/`--max-code-bytes`/
  `--max-touched` to opt in); `Runner::compile` stays permissive for trusted/game code,
  `compile_with_config(src, CellConfig::sandboxed())` for untrusted. `Report.halt` now
  says *why* a run stopped (returned / cycle-budget / memory-limit). Tested. *(Next: a
  wall-clock timeout and a monomorphization cap — compile-time blow-up guards.)*
- [~] **P8 · Cell-specific codegen wins** (overlaps Stage 2) — landed the multiply/divide
  ones (they benefit games too): **`× constant` is shift-and-add** (any constant, not just
  powers of two — `__mul16` gone for constant multipliers), **`/ 2ⁿ` / `% 2ⁿ`** are
  shift/mask (no `__divmod16`), and **literal-only ops const-fold**. Result: `mul_loop`
  (`×3`) dropped 12.8M → 2.5M T-states (warm 9.9 ms → 1.1 ms, ~8.8×); `entities` warm
  11.5 → 6.7 µs. And **`__mul16` is now multiplier-terminated** (early-exit) — a `var*var`
  with small operands finishes in a few iterations, not a fixed 16, so a mul-using snippet
  (`x*x + y*y`) roughly **halved** its per-call (cell-bench `run_fast` 1.9 → 1.0 µs).
  `__divmod16` gained a **`dividend < divisor` fast path** (quotient 0, remainder =
  dividend — returns at once instead of 16 iterations; common for `% n` of in-range
  values). Also a **compile double-parse fix** (the cap scan shares the AST) ~halved
  compile time. Differential-tested (incl. `__mul16` across multiplier widths and
  `__divmod16` across a<b / a>b / a==b / 0 / wide) + asserts the runtimes aren't appended
  for constants. **`x * x` (a variable squared) now loads the operand once** and fans it
  to both registers instead of evaluating + reloading twice — score `x*x + y*y + x*3`
  dropped 385 → 327 T-states (−15%), 53 → 47 code bytes (helps games too). Differential-
  tested (`square_same_var` across widths + overflow). A disassembly/perf-debug of the
  score showed the remaining warm cost was **interpreter dispatch + fixed per-call
  overhead, not codegen** — which is exactly what the **decode-once fast executor** (P2)
  then attacked, taking the batch hot path from ~0.19 → ~0.05 µs/call by skipping the
  authentic CPU's per-instruction fetch/contention/refresh/flag work. So codegen micro-opts
  still shrink games + cycle counts, while the engine swap took the wall-clock win.
  *(Next: register-fitting locals out of slots — a real allocator, high risk for modest
  gain; or supersede in cell mode via host-native intrinsics, see B4.)*
- [ ] **P9 · Direct-IR cell mode** (later) — let advanced callers feed IR/JSON straight
  to codegen, bypassing the Rust parser (model-generated tools). Rust source stays the
  default — it's human-readable, testable, debuggable.
- [~] **P10–11 · Benchmark families + matrix** — have synthetic + 2 real rows; add
  agent-shaped classes (scalar, state-transition, scoring, bounded search, data-structure,
  generated-code stress, trace mode) × {cold, warm, warm-batch-10k, fast/report/trace},
  and publish — proving usability, not raw compute.

### B4. Cell80 — a Z80 *superset* for the cell — **dual-target + intrinsics (mul/div · fill · halt) built**
The cell keeps hitting Z80's limits (software mul/div, no block ops, no typed I/O, return
via the calling convention). Rather than make *authentic* Z80 do everything, treat Z80 as
the **base** and define a small **superset for cell mode** — two backends off the one
frontend/IR:
```
                 ┌─ target: spectrum  → authentic Z80 / .tap (CALL __mul16, …)
rustz80 frontend ┤
   (typed IR)    └─ target: cell      → Z80 + host intrinsics (the microVM)
```
A `Target` capability picks the lowering; **real games stay real** (no non-Z80 bytes in
spectrum output), agent cells get the fast/ergonomic chip. Keep it *tiny and bounded* —
deterministic, sandboxed, easy to emulate; **never** a general OS (no fs/net/threads/heap/
syscalls — host tools live *outside* the cell; the cell computes, the agent decides).

**The mechanism is already here.** The CPU has a reserved host-trap, `ED FE` (`TRAP_OP`,
`Bus::host_trap(&mut regs)`), today a no-op on a bare bus. So a cell intrinsic is just
`ED FE <id>` (id in `A`, operands in regs) that the cell's bus services natively — clean,
disassemblable, and a no-op on real hardware (so it can't sneak into a real game silently;
spectrum-mode codegen simply never emits it). Per-op lowering table:

| op | spectrum48 | cell |
|---|---|---|
| `u16 * u16` | `CALL __mul16` | `ED FE` MUL16 (host-native) |
| `u16 / %`   | `CALL __divmod16` | `ED FE` DIVMOD16 |
| memcpy/clr/fill | emit loop | `ED FE` MEM* |
| typed input/output | symbol memory | `ED FE` READ/WRITE region |
| halt + tuple return | trampoline `HALT` | `ED FE` HALT (clean halt code + regs) |

**Incremental route** (smallest, safest first):
- [x] **1 · `Target` capability flag** — `codegen::Target { Spectrum48, Cell }` threaded
  through codegen; `compile_program`/`.tap`/games default Spectrum (authentic), the cell
  defaults **Cell**. No behaviour change for Spectrum output (real Z80 still real).
- [x] **2 · Trapped `mul`/`div`** — Cell mode lowers non-constant `*`/`/`/`%` to the
  `ED FE` host trap (`0x10` MUL16 / `0x11` DIVMOD16, matching `spectrum::host::math_traps`);
  `CellBus::host_trap` does native `u16` arithmetic. No `__mul16`/`__divmod16` appended in
  cell mode (code shrank 69 → 53 B). **Result: the score's per-call (`run_fast`) dropped
  1.03 → 0.24 µs (~4.3×, ~4M evals/s) — ~18× off native-JIT Wasm.** Differential: a Cell
  program with `a*b + a/b + a%b` matches rustc and appends no runtime, while the Spectrum
  compile still does. (Authentic Spectrum keeps the software routines — and their
  early-exit/fast-path wins.)
- [x] **3 · Block memory op (fill)** — `[v; N]` array init is now one `Stmt::Fill` (value
  evaluated once) instead of N unrolled stores. Spectrum: a first-slot store + `LDIR`
  (compact + fast — a games win too). Cell: an `ED FE` FILL16 trap (`0x20`: `BC` slots of
  `DE` at `HL`), serviced host-native (writes are still tracked, so the next run resets
  them). A `[0u16; 64]` cell dropped from ~450 B of stores to **28 B**. Differential-tested
  (word const/zero/runtime + `u8`) — every element is one 2-byte slot, so the fill is
  slot-stride. *(memcpy awaits an element-copy construct.)*
- [~] **4 · Typed I/O regions** — `StateCell::bind(src, "State", entry)` lays a state struct
  at a fixed `STATE_BASE` and exposes **typed I/O by field name**: `set("x", 10)` →
  `run(budget)` → `get("score")`, resolving names to addresses via the struct layout (the
  program is `impl State { fn run(&mut self) … }`); `fields()` lists them, reuse is
  leak-clean. The JSON↔state surface the MCP adapter (P6) needs. The structured
  `{halt, result, cycles}` is already the `Report`.
- [x] **4b · `ED FE HALT`** — a `halt(code)` dialect intrinsic (Cell80; a no-op `ED FE` on
  real hardware) stops the run early with a `u16` status code, surfaced as
  `Halt::Halted(code)` (+ `halt_code` in the JSON report). The run loop breaks right after
  the trap — letting a cell signal found/not-found/error-N or bail on an assertion (the
  XHALT_OK/XHALT_ERR contract). *(Next: multi-slot field get/set on `StateCell`.)*
- [ ] **5 · (optional) trace markers + seeded RNG** — debug tier only; RNG seeded + reported
  to keep replay deterministic.
- [ ] **6 · Formalise the *Cell80* ABI** — Z80-compatible deterministic VM: 64K flat RAM,
  no ports by default, cycle budget, the `ED FE` extension space, standard I/O regions +
  halt/report ABI, optional typed-symbol metadata. A small public spec.
- [ ] **7 · (only then) real extension opcodes** if the trap dispatch ever shows up hot.
Crate shape stays layered: shared frontend/IR; `rustz80` authentic + `rustz80-cell`
virtual chip; `speccy-sdk` authoring + a future `cell-sdk`. *Pitch:* "Cell80 — a tiny
deterministic virtual chip for agent-generated programs; restricted Rust in, falls back to
real Z80 for Spectrum output, structured execution reports out."

### B5. Cell80 spin-out — extract the cell into a standalone runtime (staged)
The cell concept has outgrown "a module in a Spectrum repo." These are now **two audiences**:
`chuk-speccy` = *"build & run Spectrum games, let agents play them"*; Cell80 = *"run tiny
deterministic sandboxed programs as agent tools"* — and the second is much bigger than the
Spectrum. Buried in a retro-emulator repo, it undersells; a standalone repo lets it be
positioned as **"Cell80 — microsecond-scale safe executable tool capsules for agents"**
(the "millions of tiny tools, not millions of tool schemas" story). **But don't split early**
— the API (traps, typed I/O, manifest, CLI, MCP, inter-cell, tool index) is still moving;
a premature split creates dependency friction. Do it **staged**:

- [x] **Phase 1 · clean internal boundary** — the cell path must **not** depend on the
  Spectrum emulator (ULA, video, audio, keyboard, TAP, SDK `Frame`/`Input`). **Verified
  met:** `rustz80`'s only deps are `syn` + `z80` (the generic CPU) — no `spectrum`/SDK; the
  `Frame`/`Input` references are dialect *type-name* recognition, not a crate dependency.
  The test *("copy `rustz80` + the cell into a new repo without the emulator?")* passes
  today. The `cell/` + `codegen/` module split sharpened it further.
- [ ] **Phase 2 · settle the identity** — name the product **Cell80** (Z80-derived, a cell
  runtime, not just Spectrum). Future crates: `cell80-core` (VM/runner/reports/fast path),
  `cell80-compiler` (frontend/IR/Cell80 backend), `cell80-cartridge` (`.cell` manifest +
  format), `cell80-cli`, `cell80-mcp`. Start coarse (`cell80` + `cell80-mcp`), split later.
- [ ] **Phase 3 · extract once the `.cell` cartridge lands** — wait for a minimal cartridge
  format so the new repo has a crisp object: `source.rs → .cell → run/inspect/bench/MCP`.
  Without it the repo reads as "a module from chuk-speccy"; with it, "a new executable
  artifact format for tiny agent tools." (The image format `to_bytes`/`from_bytes` is the
  seed of `.cell`; a named, versioned, manifest-bearing artifact is the gate.)

**The compiler is the shared part.** Short term: it stays in `chuk-speccy`, cell as a
subcrate. Long term: extract compiler/core to `cell80`, and `chuk-speccy` depends on it for
the Spectrum target. **What moves out:** flat-RAM runner, Cell80 traps, manifests/cartridges,
typed I/O ABI, MCP cell tools, the tool registry/inter-cell graph. **What stays:** ULA/
video/audio/keyboard, TAP/TZX as Spectrum media, ROM integration, 48K timing, the game SDK
`Frame`/`Input`, `SpectrumEnv`, agents *playing* games.

**Break-it-out decision rule** (all true): ① cell API stable enough for external users · ②
`.cell` artifact/manifest exists · ③ CLI compiles/runs/inspects independently · ④ no
Spectrum-emulator dependency in cell mode · ⑤ README explains the value in 30 s · ⑥ MCP
server is plausible as a separate adapter. **Risk = fragmentation:** keep the two
mutually reinforcing, not competing — Cell80 README: *"began as the compute-cell layer of
chuk-speccy; still targets authentic Z80/Spectrum where needed"*; `chuk-speccy`: *"uses
Cell80/rustz80 for compiled game logic and agent-cell execution."* Order: **internal
boundary → rename → `.cell` → CLI polish → extract.**

### B6. Cell80 as an agent-tool substrate — the product sequence
The cell roadmap now shifts from *"prove the cell can exist"* (done — it runs, it's fast,
safe, tiny, deterministic) to **"make cells a usable agent/tool substrate."** North star:

> **Agents discover, inspect, compose, and run *millions of tiny executable tools* without
> loading their schemas into context. Each tool is a self-describing `.cell` cartridge —
> bounded, deterministic, fast to start, cheap to run, safe by default.**

Ordered sequence (consolidates B3/B4/B5; ✓ done · ~ partial · ☐ next):

1. ✓ **Freeze the cell ABI (v1)** — `ABI_VERSION = 1`; `Report` JSON leads with `"abi":1`
   (schema locked by test). Full contract in
   [`docs/09-cell80-abi.md`](./09-cell80-abi.md): memory map, calling convention
   (`HL`/`DE`/`BC`), typed I/O, cycle budget + the **`cycles` trap-cost caveat** (loud
   comment: not authentic Z80 time, never an RL reward), halt statuses, capability model,
   JSON schema, image format. **B3 seam now closed against the host oracle:**
   `struct_field_state_matches_host` runs a struct program through the cell, reads *every*
   field via `struct_layout`, and asserts equality with the same logic under rustc — the
   field-state differential `diff.rs` only did for `HL` before.
2. ~ **`.cell` cartridge format** — **landed:** `Cartridge` = a `Manifest` (id · summary ·
   tags · entry · source-hash · compiler + ABI version) wrapping the `CellProgram` image
   (`CELL` magic, `to_bytes`/`from_bytes`); CLI `compile <file.rs> -o <file.cell>` +
   `inspect <file.cell> [--json]`. A *named, versioned, manifest-bearing* artifact —
   portable tool objects (compile once → ship → inspect → run). *(Next: the typed entry
   I/O schema in the manifest — step 4; optional embedded tests.)*
3. ~ **Native CLI** — `compile`(→`.cell`) ✓ · `run`(source) ✓ · `inspect`(`.cell`) ✓.
   *Next:* `exec`(`.cell`, the runtime/registry loop, vs `run`-source the dev loop) ·
   `bench` · `verify` · `trace`.
4. ~ **Typed schema from structs** — emit `{input:{…}, output:{…}}` JSON from `Input`/`Output`
   struct defs so callers use **named JSON, not raw addresses** (`StateCell` already does the
   runtime name↔addr mapping; this auto-derives the schema). The agent-friendliness unlock.
5. ✓ **Batch API `run_many_fast`** — one cell, many inputs → many outputs (decode-once fast
   path, ~0.05 µs/call). *(Next: CLI `exec --batch candidates.json`.)*
6. ☐ **MCP server (P6)** — start small (`cell_compile`/`inspect`/`run`/`exec`/`bench`), then
   cached sessions (`cell_load → id`, `cell_run_cached(id, input)`, `cell_unload`) to keep
   the warm-run edge; later `cell_search`/`trace`/`verify`/`graph_run`.
7. ☐ **Tool manifest + local index/search** — the bridge to "millions of tools without
   millions of schemas": each cell carries a compact manifest (id/summary/tags/io/limits/
   caps); `cell index add *.cell` + `cell search "…"` returns *summaries*, and the model
   loads only the selected tool's schema — not the whole library.
8. ☐ **CellGraph / inter-cell messaging** — composition; v1 deliberately constrained: static
   graph, bounded mailboxes, fixed message size, deterministic scheduler, **no dynamic spawn,
   no shared memory, every message traced**. Intrinsics `send`/`recv`/`poll`/`yield`
   (planner→scorer→validator→decision; worker-swarm→reducer). The "tiny executable society."
9. ~ **Cell80 extensions** — mul/div/fill/halt traps ✓; next: memcpy/memclr, trace/assert
   traps, typed-I/O traps, message send/recv traps. Keep `Spectrum48` = real Z80, `Cell` =
   Z80 + safe virtual chip — never pollute the Spectrum side.
10. ☐ **Extraction** — see [B5](#b5-cell80-spin-out--extract-the-cell-into-a-standalone-runtime-staged);
    split once `.cell` + CLI + MCP exist and the artifact boundary is crisp.
11. ☐ **Reference demos** — calculator · scorer (1000 candidates) · typed-state (Input/Output
    JSON) · repair benchmark (agent patches a broken gcd/sort) · tool-search (retrieve + run
    from the index) · cell-graph (planner/scorer/validator) · Spectrum-bridge (same compiler
    still emits a `.tap`). *Scorer + tool-search are the key agent-tool proofs.*

**Immediate next 5 PRs:** ① `Report` JSON v1 + ABI docs · ② `.cell` cartridge + `inspect` ·
③ `exec` compiled `.cell` + cached-runner path · ④ typed `Input`/`Output` schema generation
· ⑤ batch CLI (`exec --batch`) — the `run_many_fast` core is already done. Then ⑥ MCP MVP ·
⑦ manifest/index/search · ⑧ CellGraph MVP.

### C. Spectrum-native chatbot / agent (spec 04)
- [x] **`CHAT_*` host protocol + event queue** — over the trap ABI, both host-side:
  Python `chat.py` (`ChatSession`, pluggable responder, optional `llm_responder`
  for chuk-llm) and native Rust `spectrum::host::chat_traps()`. `CHAT_BEGIN`/`POLL`/
  `CANCEL`/`RESET`; reply streamed as teletype events. Tested end-to-end.
- [x] **Z80 terminal — interactive** (`spectrum::sdk::CHAT_TERMINAL`): reads a
  keyboard **input line** (echoed; forces L-mode so letters aren't keywords), and
  on ENTER sends it via `CHAT_BEGIN` then teletypes the reply (in cyan) via
  `CHAT_POLL` + `RST $10`. Live in the GUI: **`speccy-gui <rom> chat`** — type and
  chat. Headless round-trip test; `spectrum --example chat_terminal` is the canned
  one-shot demo.
- [ ] Real chat backend: wire `chuk-llm` into the responder (hook is in place).
- [ ] Terminal polish: colour-by-event beyond ink, `PRINT_FIFO` + beeper click, a
  UDG "thinking" spinner while `CHAT_POLL` returns NONE (matters with a slow LLM).
- Prereq: SDK trap ABI (B/L2) and beeper (✓ M6).

### D. More frontends (spec 05)
- [ ] Web / **WASM** head (`wasm32` + canvas + Web Audio) — core compiles unchanged.
- [ ] Effect chain as GPU shaders (`scanlines` → `crt` preset) in the window head.
- [ ] Web / streamed head (WebSocket framebuffer) for shared/agent sessions.

### E. The authoring plane — one source, three artifacts (spec 08)
**The headline next track**, and the synthesis of B (SDK), B2 (`rustz80`), and the
agent-env layer. The invariant of
[spec 08](./08-speccy-kit-authoring-plane-spec.md): **one typed Rust `impl Game` is
the single source of truth → three artifacts fall out with no retrofit — a host
build, a pure `.tap`, and an agent environment.** Nothing is allowed between the
struct and any artifact. The deterministic core + bit-exact reset are the foundation
already in place; this turns them into a research *product*. Sequenced (§10) so the
dial is never multiplied before it's watched close:

- [~] **1 · Prove the seam** (the headline) — *bridge proven; pure Snake pending.*
  The minimal seam is closed: a typed `score` field round-trips (Rust decl → emitted
  addr → read off the running tape, see below). Done since: `chuk-speccy-sdk` ships
  the subset-clean primitives **`Entities<T, N>`** (fixed-cap vec) and **`Rng`**
  (state-seeded, deterministic), the **demo Snake is off `Vec`/`format!`** (uses
  `Entities`/`Rng`, `Frame::text_u16`) and exposes the env surface. *Remaining:* a
  Snake that compiles **pure** as one source. Done since: generic *functions* + structs
  ✓, const-generic *functions* + structs ✓ (a fixed-cap `Stack<N>` compiles + runs),
  tuples + tuple struct fields ✓, `for`/`loop` ✓, array struct fields ✓, struct-element
  arrays — local, as a struct **field**, and the **`Entities<Cell, const N>` combo** ✓
  (`examples/entities`, instances `Entities$4`/`Entities$8`), `u32` bitwise/shift ✓ (a
  32-bit **xorshift** RNG runs — `examples/rng32`). **All structural compiler blockers
  are done.** *What's left is SDK-side, not compiler-side* (see Stage 4i): a power-of-two
  `Rng::below` (or `u32 %`), and a value-args drawing path — `Frame::tile`/`text` are an
  SDK concern routed through the generic `PreludeConfig` (like `Frame::pixel`), gated only
  by passing tile/string **data by address** rather than `&Tile`/`&str`. `Fx8_8` lands
  with the kit, not here.
- [x] **The symbol map — emitted + round-tripped** (the riskiest bit, *done*).
  `rustz80` emits a full-layout `.sym.toml` (every field a `u16` slot at
  `GAME_STATE + i*2`) via `compile_game_with_symbols`, sidecar'd by `speccy-compile`;
  `rustz80/tests/seam.rs` proves it on the real ROM — a typed `score` field read off
  the running tape **via its emitted address**, climbing as the game runs. The bridge
  exists.
- [x] **The symbol map — the env side** (*done*; the bridge's other half).
  `chuk-speccy-env` parses the `.sym.toml`, reads the typed fields off a running
  tape's RAM into a `StateView`, reconstructs a host `Self` (`FromState`), and runs
  the **same** host `reward`/`done`/`observe` over it — proven end to end in
  `speccy-env/tests/env.rs`. **Supersedes hand-written memory maps for *authored*
  games** (a hand `memory_map.toml` survives only for *found* commercial titles); the
  `env_report` trap becomes an optional fast-path.
- [x] **Widened `Game` trait** — `observe() -> Obs`, `reward(&self, prev: &Self) -> i16`
  (typed — **no string DSL**), `done()`, `reset(seed)`, all with defaults so every
  existing game compiles unchanged. The demo Snake overrides `reward` (score delta) /
  `done` (death) / `reset` (seed). Writing a game *is* writing its env.
- [~] **`chuk-speccy-env`** — the Gym surface + baseline agents + a benchmark exist.
  `SpectrumEnv`: bit-exact `reset` (`serialize_full`/`deserialize_full`), `step_game`
  (hold keys + `run_frames`), `frame_indexed` (pixel obs), `view`/`reconstruct` (typed
  obs via the symbol map), `Transition { obs, reward, done }`. `agents`: `Agent` trait
  + `NoOp`/`Random`/`Scripted` + `run_episode`. The `reach` sample (a reward-bearing
  input game) gives a working **agentability benchmark** — `speccy-env/tests/benchmark.rs`
  shows `no-op 0 < random 0 < scripted 17` on real hardware. *(Found + fixed a real
  bug doing this: the dialect Down/`A` key read a bad port — QAOP `A` never worked.)*
  *Remaining:* memory-probe / vision-LLM / replay agents + a multi-game score table.
  `DaleyThompsonEnv` is the **SOMA B1⊥B2 demonstrator**.
- [~] **Input as one source of truth + demo ROMs** (the build→extract loop, spec 08 §4).
  `Controls` (remappable `Button`↔keys) extracted into the SDK; the demo games moved
  out of the library into **`chuk-speccy-games`** (`snake`/`keytest`/`typing`/`mover`)
  behind a name→installer **registry**, so heads stay game-agnostic
  (`speccy_games::install(spec, name)`). `Frame::new`/`Input::none`/`held_now` exposed
  for testing games. *Next here:* lift the move/erase/draw loop into an `Actor`/`Sprite`
  and a `Hud` — which needs the `rustz80` array-struct-fields extension to go pure.
- [ ] **2 · The kit (L1 + L0)** — `chuk-speccy-game` (subset-clean Sprite/TileMap/
  Scene/Hud/SoundBank; sprites *name* the colour-clash; a dirty-cell engine as the
  **dial canary**) + `chuk-speccy-assets` (PNG/Tiled/tracker → `const`; the
  **colour-clash report** is the cheap demo-magnet). Sound = const data, two players,
  both emitting real port-`0xFE` edges (never "nice generated audio").
- [ ] **3 · Vertical slice** — `speccy new maze --template agent_maze`: splash ·
  tilemap · sprites · beeper SFX · HUD · RNG · typed probes · reward · env · random +
  scripted agents · host run · `.tap` · MP4.
- [ ] **The agentability report** — static analysis over typed reward + the symbol
  map + short rollouts; the **reward-hackability detector** is the research headline
  (possible only because reward is typed, not a DSL).
- [ ] **4 · The authoring studio (LAST)** — `chuk-mcp-speccy-kit`: intent-level tools
  (`add_actor`, `build_assets`, `compile_tap`, `wire_reward`), with **"compiles pure"
  as a security property** (agent-authored games must pass `rustz80` unless an
  escape-hatch trap is whitelisted). Authoring *emits*; the runtime plane
  (`chuk-mcp-spectrum`) *runs*. Templates are cognitive axes (gridworld / runner /
  maze / rhythm / shooter / puzzle) → a **deterministic task factory** that happens to
  express tasks as real Spectrum games.

### F. Reach — distribution & demo
Right now it's compelling to *developers*; these make it usable by, and legible to,
everyone else. Cheap relative to their impact.
- [x] **Top-of-README demo GIF** — the dialect Snake (compiled by `rustz80`) running,
  in `docs/assets/demo.gif`. Rendered headless by **`display::gif`** + the reusable
  **`speccy-gif`** CLI (`speccy-compile snake.rs | speccy-gif`) — no capture tool. A
  richer "search → play → agent takes over → rewind → MP4" clip is a nice follow-up.
- [x] **`v0.1.0` release** (source) — tagged with notes; GitHub description + topics +
  CI all ✓.
- [x] **Prebuilt binaries** — `.github/workflows/release-binaries.yml` builds
  `speccy-gui` + the CLIs on macOS/Windows/Linux and attaches per-OS archives on
  every release (fires on `release: published`; backfilled v0.1.0). No toolchain
  needed to play.
- [x] **Published to crates.io** (v0.1.0) — `cargo install speccy` / `cargo install
  rustz80`; libraries as `chuk-speccy-{z80,spectrum,display,wos,sdk}`
  (`cargo add chuk-speccy-spectrum`). A manual `publish-crates.yml` (dependency-ordered,
  token-gated) ships future versions.
- [ ] **PyPI** — publish `zxspec_py` (maturin) + `chuk-mcp-spectrum` so the Python
  side is `pip install`-able too.
- [ ] Player niceties for the GUI: drag-drop ROM/game, a game-search field, recent
  games, key-remap UI, save/load slots.

### Positioning (honest)
Not the strongest *emulator* in the ecosystem, and not trying to be: **RustZX** is the
more mature 48K/128K player, **z88dk** the more complete Z80 dev kit, **ZX84** the
nearest MCP/browser cousin. The distinctive lane is the **integrated, deterministic
agent lab**: play → drive programmatically → record/replay episodes → inspect machine
state → *build new Spectrum-native research environments in Rust*. The combination
(deterministic core + GUI/TUI + WoS loading + MCP agent/admin + bit-exact
checkpoints + MP4 + PyO3 + native SDK + Rust→Z80 compiler) is the innovation, not any
one piece. Pitch on agent-lab integration, not emulator breadth.

---

## Later — accuracy long tail (optional deep end)

Deliberately deferred — **below the agent-environment layer (E)** in priority, since
it affects timing-precise demos, not games or agent tasks.
- [ ] I/O-port contention (the 4-case ULA/high-byte timing model).
- [ ] Floating-bus reads.
- [ ] Per-T-state / per-scanline video (mid-frame writes → multicolour demos).
- [x] **Real-time tape edge loading + `.tzx`** — `TapeSignal` plays the tape as a
  pulse stream into the EAR line so turbo/custom loaders work (the trap fast-load
  stays for standard `.tap`). TZX common blocks (standard/turbo/tone/pulse/data/
  pause/loops). Proven end-to-end; the Dizzy games load. *(Direct-recording /
  CSW / generalised TZX blocks not yet handled.)*
- [ ] 128K model: memory paging + AY-3-8912 sound (memory layer is written bank-ready).

---

## Hardening — code review & placement remediation (2026-06)

A workspace-wide placement + code review (4 parallel reviewers). Grouped by area,
roughly in priority order. The headline is making `rustz80` a *generic* compiler and
lifting the game layer into the SDK; the rest is layering hygiene + code quality.

### H1. Make `rustz80` generic; lift the game layer into the SDK — **done**
The leak was deeper than `lib.rs` — also in `codegen.rs` and `lower.rs`. All lifted.
- [x] `rustz80` is now generic API only: `compile_program`/`compile_fn`/`compile_to_tap`,
  `lower_program(file, &PreludeConfig)`, `codegen_loop(funcs, org, entry, state_base,
  state_bytes)` (no `GAME_STATE`), `to_tap`, `ORG`. Removed `PRELUDE`, `compile_game*`,
  `has_game`, `find_game_impl`, `struct_layout`, `GAME_STATE`, `codegen_game`, `state_symbols`.
- [x] `lower.rs`: `handle_type`/`lower_prelude_call` now use a caller-supplied
  `PreludeConfig` (the `(handle, method) → fn` map) — the lowerer has no game knowledge.
- [x] Game layer moved into `speccy-sdk::compile` behind a **`compile` feature**
  (`dep:rustz80`, `dep:syn`); the frontend bins stay `rustz80`/`syn`-free (verified via
  `cargo tree`). CI now runs `--all-features`.
- [x] `speccy-compile` bin moved to the SDK (behind `compile`); release workflow updated.
- [x] **Unified `SymbolMap`**: one `speccy-sdk::symbols` type (parse + emit, no `syn`);
  `speccy-env` re-exports it; its duplicate deleted.
- [ ] Split `samples/`: `bounce`/`move`/`reach` (`impl Game`) → SDK/games side;
  `snake`/`pixels` stay generic. *(Deferred — inert `.rs` files referenced by path;
  left in `rustz80/samples/` to bound churn.)*
- [x] Spec 08 reconciled with the placement (symbol-map emission is the SDK's, not
  `rustz80`'s; the removed `demo.rs` reference fixed).

### H2. Presentation out of the emulator core (`display` vs `spectrum`) — **done**
- [x] Deleted `spectrum::ula::PALETTE` (the duplicate); `render_rgba`/`screen_rgba` now
  read `display::AUTHENTIC` (one source of truth). `spectrum` depends on `display`
  (standalone, no emulator deps → clean downward reference).

### H3. Frontend de-duplication
- [x] `Spectrum::load_media(fmt, data)` + `BOOT_FRAMES` + `media_format(name)` in the
  core; all 4 bins now call it (the copy-pasted dispatch deleted, the `.tzx` gap in
  `main.rs`/`gui` fixed). Format strings are now `spectrum::format::{TAP,TZX,SNA,Z80}`
  constants, not magic strings.
- [ ] Block-glyph renderer (`main.rs`) untested + belongs in `display`; `keycode_char`
  drops symbol keys (GUI/TUI input asymmetry); test-card / z80-header knowledge in heads
  belongs near `spectrum`. *(Deferred.)*

### H4. Core layering hygiene (`z80`/`spectrum`)
- [x] `StopReason` moved from `z80` (a run-loop concept it never produces) to `spectrum`.
- [x] Tightened `z80` module visibility (`alu`/`decode`/`flags` → `pub(crate)`); dropped
  dead `Memory::ram_mut`; `tape_trap` uses `wrapping_sub`; fixed the duplicated
  `screen_indexed` doc.
- [ ] App/SDK artifacts in the core: `sdk.rs`'s `CHAT_TERMINAL` + the chat
  dispatcher/`ChatState` in `host.rs` → move to the SDK/app layer; keep only the generic
  trap machinery. *(Deferred — larger.)*
- [ ] `Cur::take` bounds-hardening; factor the thrice-repeated `.tap` block parser;
  `.z80` v2/v3 page loader + RLE `decompress_z80` tests. *(Deferred.)*

### H5. `rustz80` code quality
- [~] Error UX (the "ergonomic rejection" feature, spec 08 §1.5): every unsupported
  node is a clean `Err` and `tests/coverage.rs` now exercises the rejection arms
  (unsupported expr/stmt/pattern/type, arity errors, const/lifetime generics, tuple
  misuse, …). *(Still open: span-carrying errors instead of `{syn:?}` dumps; turn the
  remaining codegen `panic!`/`expect` into `Err`; reject recursion.)*
- [x] Split `lower.rs` (had grown past 1300 lines) into a `lower/` module tree —
  `vars` (register file), `layout` (struct/enum + parse helpers), `prelude` (handle
  routing), `generics` (monomorphization), `expr`, `stmt`, and `mod.rs` (the `Ctx` +
  orchestration); none over ~350 lines. *(Collapsing the read/store + block duplicate
  pairs is still open.)*
- [x] **Test coverage** — `tests/coverage.rs` + the differential/example suites bring
  `rustz80` to **97% line / 95% region** coverage (`cargo llvm-cov`), every source file
  ≥ 90% on both.

### H6. `speccy-env` — **done**
- [x] `StateView::u16` now **panics** on an unknown field (a `FromState` typo —
  spec 08 §2's "silent missing field is the worst bug"); `try_u16` is the lenient
  variant. `view()` reconstructs **all `count` elements** of array fields; `array(name)`
  reads them. (The TOML parser was unified into the SDK in H1.)

### H7. Python (`zxspec_py` / `chuk-mcp-spectrum`)
- [ ] **Supervisor concurrency**: viewer + agent + admin share a non-thread-safe
  `Machine` with unsynchronized mutation → per-session lock (the one real runtime-risk
  finding).
- [ ] Fix `test_restore_snapshot_rewinds` (restores a `serialize_full` blob via
  `load_snapshot("sna")` — passes by luck); tighten `load_sna` to reject over-length blobs.
- [ ] PyO3 error formatting `{e:?}` → Display; refresh MCP README tool counts; gitignore
  `*.egg-info/`.

### H8. Misc / low priority
- [ ] `Fx8_8` is named in spec 08 but not implemented in the SDK.
- [ ] Naming outliers (defensible, low priority): `z80-tests` lacks the `chuk-speccy-`
  prefix + a `[lib] name`; `zxspec_py` diverges from the `speccy` brand stem.

---

## Suggested order

```
core M0–M8 ✓ ─▶ A. MCP server ✓ ─▶┐
                    └─▶ B. SDK ✓ ─▶ C. chatbot ✓
                            └─▶ B2. rustz80 ✓ — dial closed (one impl Game: host + pure)
                                        │
                                        ▼
        E. AUTHORING PLANE (spec 08) ◀── the headline next
        one typed source → host build · pure .tap · agent env, bridged by the
        compiler-emitted symbol map.  Sequence: 1 prove-the-seam (Snake) →
        2 kit (L1+L0) → 3 vertical slice → 4 authoring studio (LAST).
   D. frontends (WASM / shaders / streamed)    ── parallel, any time
   F. reach (demo GIF ✓ / release ✓ / binaries ✓ / player niceties) ── parallel
   Later. accuracy tail (128K/AY, …)           ── below E in priority
```

The honest through-line: everything is downstream of a Z80 core you trust (passes
ZEXALL), so the spine — core → MCP → SDK/chat → `rustz80`, plus bit-exact reset and
published crates — is **built**. The single highest-value next move is **E, the
authoring plane (spec 08)**: turn the built ingredients into *one typed source → three
artifacts* with the compiler emitting the symbol map. Don't build the studio first —
**prove the seam on Snake**, then the kit, then a slice, then the MCP studio last.
Everything else (frontends D, the rest of reach F, the accuracy tail) is independent
and parallel, all below E.

A second strategic arc has since opened up alongside E: **B6, Cell80 as an agent-tool
substrate**. The cell layer (B3/B4) is built and fast; B6 turns it into a *platform* —
freeze the ABI → `.cell` cartridges → CLI → typed schemas → MCP → a tool index → cell
graphs — toward the north star of *"millions of tiny executable tools agents discover and
run without loading their schemas."* It shares the `rustz80` frontend with E but targets a
different (and bigger-than-Spectrum) audience, and is where current momentum sits. Immediate
next: the B6 5-PR sequence (ABI v1 → cartridge → exec/cached → typed schema → batch CLI).
