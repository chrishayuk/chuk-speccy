# chuk-speccy — Roadmap

Single source of truth for what's built and what's next. The design is split
across six specs ([README index](./README.md)); this tracks delivery against them.

**Status:** the **emulator core is feature-complete (M0–M8)** — a cycle-accurate,
ZEXALL-clean 48K Spectrum with two heads over one themeable display pipeline. The
remaining work is the *layers on top*: MCP server, SDK, chatbot showpiece, extra
frontends, and the accuracy long tail.

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
- **Native window (`speccy-gui`, minifb):** pixel-perfect 256×192, integer-scaled, real key up/down, cpal sound with **audio-driven frame pacing** (emulation refills the ring to ~3 frames, so it tracks the real-time audio clock instead of the jittery video refresh — no underrun, stable beeper pitch). Accepts a **game title** (fetched from World of Spectrum) as well as a file, and a `fullscreen` mode (borderless, screen-filling, on-top) for a second display / AirPlay mirroring.

### Test inventory
- `z80-tests`: 32 unit + ZEX harness (`run_zex`, CP/M BDOS trap).
- `spectrum`: 6 unit (incl. contention, beeper, `.sna` round-trip, `.tap` trap) + 4 ROM-backed (`boots_to_copyright`, `types_basic_and_evaluates`, `title_music_makes_sound`, `tap_loads_and_autoruns_basic`).
- `display`: 4 unit (theme/effect/border).
- All warning-clean. ROM-backed tests gated behind `SPECTRUM_ROM` / `SPECTRUM_GAME` env (ROMs gitignored under `testroms/`).
- Diagnostics: `spectrum --example audiodiag` reports the beeper's dominant pitch per window (contention on vs off). Finding: contention has negligible effect on beeper pitch; the toggle stays as an A/B aid, not a fix.

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
  are reported as needing real-time tape loading (see the accuracy tail). The
  `speccy-library` bin headlessly verifies a set of classics in one command.
- [ ] `set_display(preset)` — expose the `display` crate themes so an agent can re-theme + screenshot.
- [ ] `disassemble` / `trace` / breakpoints (need a disassembler in the core first).
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
- [ ] **L0** toolchain: one-command source → `.tap` → run-in-emulator; PNG→Spectrum asset pipeline.
- [ ] **L1** framework over z88dk (sprites clash-aware + mono, tilemap, input, beeper SFX, fixed-point, RNG).
- [ ] **L2** trap ABI: the `ED 70 <id>` host syscall + dispatch table, behind a pure/hybrid build flag. (The `z80` ED decoder already NOPs undefined slots — the degrade-on-real-hardware property is in place.)
- [ ] **L3** showpiece: one app calling an MCP server through a trap.

### C. Spectrum-native chatbot / agent (spec 04)
- [ ] `CHAT_BEGIN`/`CHAT_POLL` trap ABI + per-session event queue.
- [ ] Host `run_chat` over `chuk-llm` + `chuk-tool-processor`; `speccy()` ASCII sanitiser + 32-col system prompt.
- [ ] Z80 terminal: input line, custom colour-by-event print, `PRINT_FIFO` teletype drain + beeper, UDG spinner.
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
- [ ] Real-time tape edge loading + `.tzx` (currently trap-load `.tap` only).
- [ ] 128K model: memory paging + AY-3-8912 sound (memory layer is written bank-ready).

---

## Suggested order

```
core M0–M8 ✓ ──▶ A. MCP server ──▶ E. RL env (free re-skin)
                      │
                      └──▶ B. SDK (L0→L2) ──▶ C. chatbot (L3 showpiece)
   D. frontends (WASM / shaders / streamed)  ── parallel, any time
   Later. accuracy tail                       ── parallel, as desired
```

The honest through-line (from the specs): everything is downstream of a Z80 core
you trust — which now passes ZEXALL — so the build order is core → MCP → SDK/chat,
with frontends and the accuracy tail as independent parallel tracks.
