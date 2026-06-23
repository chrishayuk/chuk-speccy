# chuk-speccy — Roadmap

Single source of truth for what's built and what's next. The design is split
across eight specs ([README index](./README.md)); this tracks delivery against them.

**Status:** the **emulator core is feature-complete (M0–M8)** — a cycle-accurate,
ZEXALL-clean 48K Spectrum. On top of it, now **built**: the MCP server + autonomy
plane, a World-of-Spectrum game library, real-time `.tzx` loading, a disassembler,
the `ED FE` trap ABI, the Spectrum-native chatbot, and a native Rust game SDK
(Snake), and the `rustz80` compiler with a **full Snake written in the dialect** — compiled to Z80, run on the CPU, drawing to real screen RAM (differential-tested), a `.tap` emitter, and **the dial closed**: one `impl Game` source compiles under rustc (speccy-sdk) **and** rustz80 (a bootable tape that runs on the real ROM).
Plus **bit-exact `serialize_full` reset** (the RL gate), surfaced through PyO3 + MCP,
and the crates published (`chuk-speccy-*` libs, `speccy`/`rustz80` CLIs). Headline
next: the **authoring plane** ([spec 08](./08-speccy-kit-authoring-plane-spec.md)) —
*one typed source → three artifacts* (host build · pure `.tap` · agent env), bridged
by a compiler-emitted symbol map. First move: **prove the seam on Snake**. Then, in
parallel: extra frontends (WASM), `rustz80` Stage 2 (optional), and the accuracy tail
(128K/AY).

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

### B2. `rustz80` — restricted Rust → Z80 compiler (spec 07) — **Stage 0 built**
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
- [ ] **Stage 2+**: peephole + const-fold/strength-reduce; recognise `impl Game`
  (same source host + pure); generics via monomorphization; optional MIR frontend.
  Inline-asm / eDSL escape hatch for hot loops.

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
  Snake that compiles **pure** as one source — blocked on `rustz80` features
  (generics for `Entities`, **array struct fields**, tuples, `loop`); `Fx8_8` lands
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
- [ ] `StopReason` lives in `z80` but is a run-loop concept it never produces → move to
  `spectrum`.
- [ ] App/SDK artifacts in the core: `sdk.rs`'s hand-assembled `CHAT_TERMINAL` + the chat
  dispatcher/`ChatState` in `host.rs` → move to the SDK/app layer; keep only the generic
  trap machinery (`HostCalls`/`FnTable`/`HostCtx`/`math_traps`).
- [ ] Tighten `z80` module visibility (`alu`/`decode` → `pub(crate)`); drop dead
  `Memory::ram_mut`.
- [ ] Harden: `Cur::take` bounds, `tape_trap` `wrapping_sub`, factor the thrice-repeated
  `.tap` block-framing parser, fix the duplicated `screen_indexed` doc.
- [ ] Tests: the `.z80` v2/v3 page loader + RLE `decompress_z80` are untested.

### H5. `rustz80` code quality
- [ ] Error UX (the "ergonomic rejection" feature, spec 08 §1.5): replace
  `Result<_, String>` + `{syn:?}` debug-dumps with span-carrying errors; turn
  `panic!`/`expect` on undefined call targets / scratch overflow into `Err`; **reject
  recursion**; add tests for the rejections.
- [ ] Split `lower.rs` (940 lines) into submodules; collapse the read/store + block
  duplicate pairs.

### H6. `speccy-env`
- [ ] `StateView::u16` silently returns 0 for unknown fields (against spec 08 §2's own
  thesis) → `Option`/assert. `view()` ignores `count` so **array fields aren't
  reconstructed** — add typed array reconstruction. Replace the fragile hand-rolled TOML
  parser when `SymbolMap` is unified (H1).

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
