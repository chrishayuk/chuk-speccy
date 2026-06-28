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
- [x] **L0** toolchain — **built.** `speccy-new` (scaffold a dual-compile game from a
  template) → `speccy-compile` (source → `.tap` + `.sym.toml`) → `speccy-run` (source/`.tap`
  → boot on the real ROM → an animated **GIF** of it running, headless, in one command) →
  `speccy-asset` (PNG → Spectrum `.scr` + colour-clash report). `speccy-run` reuses the
  headless `spectrum` machine + `display::gif` from the SDK behind the `compile` feature, so
  the windowed frontend stays `rustz80`-free; `speccy-gui` still runs a `.tap` in a window.
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

### B2. The compiler + the cell — moved to **cell80**
The `rustz80` restricted-Rust → Z80 compiler, the cell micro-VM (`.cell` cartridges, typed
I/O, index/search, warm host, CLI, MCP), and the `z80` CPU core now live in their own repo,
[**cell80**](https://github.com/chrishayuk/cell80) (history preserved). chuk-speccy depends
on it: `spectrum`/`wos`/`zxspec_py` on cell80's `z80`, and `speccy-sdk` (the `compile`
feature) on cell80's `rustz80`. The cell roadmap (compiler stages, Cell80 ABI, the agent-tool
substrate, the standard cell library) lives in `cell80/docs/roadmap.md`. The cross-component
proof — *rustz80 output boots on the real Spectrum* — stays here as
`speccy-sdk/tests/tap_boot.rs` + the `dial`/`benchmark` ROM tests.

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

- [x] **1 · Prove the seam** (the headline) — *done: a pure Snake boots on the real ROM.*
  A subset-clean **Snake `impl Game` compiles from one source under both rustc (host)
  and rustz80 (pure)** — `speccy-sdk/samples/snake_game.rs`, wired into `tests/dial.rs`.
  On the real 48K ROM it **boots from tape, draws, animates, and its typed state reads
  back off the tape** (`len`, `food_x`, … via the emitted `.sym.toml`) —
  `snake_game_boots_animates_and_reads_back`. It uses the new subset-clean SDK
  primitives: the `fill_cell`/`clear_cell` by-value cell draw and a `u16` xorshift RNG
  (the body is parallel `[u16; 32]` arrays since struct fields are 16-bit slots). It's a
  **real game** — wall + self-collision game over, restart on Fire (auto-restart keeps a
  no-input run animating), a **numeric score** (3×5 pixel-font digits via `frame.pixel`,
  since font-by-address is gated), food that re-rolls off the body, and **constant-speed
  incremental drawing** (each move
  redraws only the head + vacated tail, never the whole body). What it omits is gated on **cell80**
  compiler features, not this repo: a text HUD (font/string-by-address needs a
  `&CONST → addr` data section) and `Entities<Cell>`/`u32` state (16-bit slots only); the
  richer `chuk-speccy-games` Snake keeps those host-side. *Original note retained below for
  the bridge details.*
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
  are done.** *What's left is SDK-side, not compiler-side* (see Stage 4i). Done since:
  the **subset-clean ranged RNG** — `Rng::below_mask(mask)` (`next_u32() & mask`; u32 has
  `&` but no `%`), with `Snake::spawn` drawing from a power-of-two and rejecting out-of-range
  ✓; the **value-args drawing path** — `Frame::fill_cell(cx, cy, ink)` / `clear_cell`, a
  data-free solid-cell sprite with the colour passed **by value** (3 args, fits the 3-register
  convention), routed through `PreludeConfig` to `__frame_fill_cell`/`__frame_clear_cell` and
  proven to compile pure (`compile::tests::solid_cell_draw_compiles_pure`); the demo Snake's
  body/food now draw through it ✓. *Remaining for a fully-pure Snake:* a dialect **`Rng`** in
  the prelude (so a pure game can call the RNG methods, not just the host one), and real
  **tile-bitmap / font-text by address** (`Frame::tile`/`text` of a `&Tile`/`&str`) — which
  needs a `&CONST → addr` data section in the compiler, now **cell80's** roadmap, not this
  repo's. `Fx8_8` lands with the kit, not here.
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
  + `NoOp`/`Random`/`Scripted` + `Recording`/`Replay` + `run_episode`. The `reach` sample (a reward-bearing
  input game) gives a working **agentability benchmark** — `speccy-env/tests/benchmark.rs`
  shows `no-op 0 < random 0 < scripted 17` on real hardware. *(Found + fixed a real
  bug doing this: the dialect Down/`A` key read a bad port — QAOP `A` never worked.)*
  **The multi-game table has its second game:** `speccy-env/tests/snake_bench.rs` makes the
  pure `snake_game` agentable — a host twin scores `len` growth and a reverse-aware homing
  agent (head → food, read *only* off the symbol map: `bx[0]`/`food_x`/`dir`) grows the snake
  while random/no-op never eat (`no-op 0 = random 0 < homing 9` on real hardware). Two games,
  same `SpectrumEnv` + agent loop — the agentability story generalises past the toy task.
  **Deterministic replay proven:** `Recording`/`Replay` agents + `replay_reproduces_the_homing_episode`
  record the homing episode and replay the keys — bit-exact `reset` + the same actions reproduce the
  reward exactly (the RL-safe `reset` / replayable-repro cornerstone, end to end through an agent).
  *Remaining:* memory-probe / vision-LLM agents + more games on the table.
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
  **Split by what the dial allows:** the *composable pure* kit — `Sprite`/`TileMap`/`Hud`
  as game-state **struct fields**, tile/font bitmaps, a text HUD — is **gated on cell80
  frontend features** (nested struct fields, `[u8; N]` arrays, persisted `u32`/`i16`,
  `&CONST → addr` const-data; the precise list is in [`cell80/docs/roadmap.md`](https://github.com/chrishayuk/cell80)
  under *"features the chuk-speccy authoring-plane kit needs"*). So Stage 2 proceeds
  **host-side now** — `chuk-speccy-assets` (asset/colour-clash tooling) and a host kit —
  and the *pure* kit lands as cell80 ships those features. Pure games stay inside the
  envelope until then (solid-cell sprites via `fill_cell`, `u16` RNG, `[u16; N]` pools).
  *Done since:* **`chuk-speccy-assets`** — `convert(rgb, 256, 192)` reduces each 8×8 cell to
  two colours (ink/paper + shared bright, min-error over the 16 authentic colours from
  `display::AUTHENTIC` — no duplicate palette), emits a drop-in **6912-byte `.scr`**, and
  reports every **attribute clash** (a cell whose source wanted >2 colours). Surfaced as the
  **`speccy-asset`** CLI (`PNG → .scr` + printed clash report). The colour-clash report — the
  cheap demo-magnet — is built; tile/tracker→`const` and the *pure* tile-draw payoff wait on
  cell80's `&CONST → addr`.
  *Also done — L0 scaffolding:* **`speccy new <name> [--template blank|snake]`** (the
  `speccy-new` bin) emits a starter that already crosses the dial. A `speccy_sdk::templates`
  module exposes the proven `samples/blank.rs` / `snake_game.rs` as templates, renamed to the
  game's state struct; **`tests/dial.rs` holds every template's host+pure guarantee** (a
  scaffolded game is dual-compilable by construction).
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
- [x] **De-dup sweep over the Stage-2 work** (a self-review): the ZX interleave formula
  is now `display::screen_byte_index` (one source — the SDK `Frame` and `speccy-assets`
  share it, no copies); `speccy-env`/`speccy-run` boot via `load_media(TAP)` + warmup
  instead of re-rolling `BOOT_FRAMES`/`load_tap`/`autoload`; and a ROM test
  (`dial::fill_cell_host_and_pure_agree`) pins the host `Frame::fill_cell` and the dialect
  prelude `__frame_fill_cell` to the same attribute encoding (the host↔pure drift guard).
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

### H5. `rustz80` code quality — moved to **cell80**
Tracked in [`cell80/docs/roadmap.md`](https://github.com/chrishayuk/cell80) now that the
compiler lives there.

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

**Cell80 has since split off into its own repo** ([chrishayuk/cell80](https://github.com/chrishayuk/cell80),
published 0.2.0) and now follows **its own roadmap** (`cell80/docs/roadmap.md`). The agent-tool
substrate arc — B6: freeze the ABI → `.cell` cartridges → CLI → typed schemas → MCP → a tool index →
cell graphs, toward *"millions of tiny executable tools agents discover and run without loading their
schemas"* — lives **there**, not here. chuk-speccy depends on cell80 from crates.io but does **not**
drive it.

**This repo's focus is squarely E, the authoring plane — the SDK side of the house.** The immediate
next move is SDK-side, not compiler-side: finish *prove-the-seam* with a **pure-compiling Snake**.
Two gaps remain, both in `speccy-sdk`, neither in the (now-generic) compiler:
1. a power-of-two `Rng::below` (mask `& (n-1)`, not `% n`) so RNG-driven placement is subset-clean on
   the authentic `Spectrum48` target (which has no `/`/`%`);
2. a by-address tile/text drawing path — `Frame::tile(&Tile)`/`text(&str)` take references; route
   value/by-address variants through the generic `PreludeConfig`, exactly as `Frame::pixel` is wired.
Then, in sequence: **2 · the kit (L1+L0)** → **3 · vertical slice** (`speccy new maze`) →
**4 · authoring studio (LAST)**.
