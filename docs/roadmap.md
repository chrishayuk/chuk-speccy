# chuk-speccy — Roadmap

Single source of truth for what's built and what's next. The design is split
across seven specs ([README index](./README.md)); this tracks delivery against them.

**Status:** the **emulator core is feature-complete (M0–M8)** — a cycle-accurate,
ZEXALL-clean 48K Spectrum. On top of it, now **built**: the MCP server + autonomy
plane, a World-of-Spectrum game library, real-time `.tzx` loading, a disassembler,
the `ED FE` trap ABI, the Spectrum-native chatbot, and a native Rust game SDK
(Snake), and the `rustz80` compiler at **Stage 1** (calls, mul/div, arrays, structs — differential-tested).
Remaining: `rustz80` Stage 1d (enum/match/u8 → Snake), extra frontends, the RL env, and the accuracy tail.

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
  `disassemble`, and the host-trap ABI (`FnTable` mul16, math, unknown-id carry,
  NOP-without-host); ROM-backed: `boots_to_copyright`, `types_basic_and_evaluates`,
  `title_music_makes_sound`, `tap_loads_and_autoruns_basic`, `chat_terminal_round_trip`.
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
- [ ] Decision to lock: native `serialize_full()`/`deserialize_full()` for exact RL/debugger reset fidelity (vs lossy `.sna`/`.z80`). See [MCP spec §10](./02-mcp-server-layer-spec.md#10-open-decision-pyo3-boundary).

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
- [ ] **Stage 1d** — `enum`/`match` + real `u8`, then **compile Snake** and run it
  on real hardware. (Recursion needs stack frames — Stage 4.)
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

### E. RL environment (specs 02 §8 / 03 §7)
- [ ] `chuk-rl-env` `SpectrumEnv` re-skin: snapshot = reset, `run_frames` = step, `read_memory`/screen = obs/reward, `save_snapshot` tree = MCTS rollouts.

---

## Later — accuracy long tail (optional deep end)

Deliberately deferred; affects timing-precise demos, not games.
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

## Suggested order

```
core M0–M8 ✓ ──▶ A. MCP server ✓ ──▶ E. RL env (free re-skin)
                      │
                      └──▶ B. SDK ✓ (trap ABI + host-composite SDK) ──▶ C. chatbot ✓
                                  │
                                  └──▶ B2. rustz80 compiler (pure-.tap dial) ── Stage 1 (calls/mul/div/arrays/structs) ✓; the big, escapable bet
   D. frontends (WASM / shaders / streamed)        ── parallel, any time
   Later. accuracy tail (real-time .tzx ✓ done)    ── parallel, as desired
```

The honest through-line (from the specs): everything is downstream of a Z80 core
you trust — which passes ZEXALL — so the build order was core → MCP → SDK/chat,
all now built. What's left divides into the **escapable big bet** (`rustz80`, B2 —
imperative Rust to a pure `.tap`) and **independent parallel tracks** (frontends,
RL env, the accuracy tail). Nothing else depends on the compiler; it's pure upside.
