# chuk-speccy

A cycle-accurate, **ZEXALL-clean** 48K ZX Spectrum emulator in Rust — with a
native window, a themed terminal head, a **World of Spectrum** game fetcher, and
an **MCP server** so LLMs / scripts / RL loops can drive and record the machine.

The core is a pure, deterministic, headless `Machine`; everything else is a thin
consumer of it. Dependency arrow: `frontend → spectrum → z80` (the `z80` crate
never knows what a Spectrum is).

## Quick start

You need a 48K system ROM at `testroms/48.rom` (gitignored — supply your own).

```bash
# Play a local tape/snapshot in the native window (pixel-perfect + sound):
cargo run --release --bin speccy-gui -- testroms/48.rom testroms/manic.z80

# …or just name a game — it's fetched live from World of Spectrum:
cargo run --release --bin speccy-gui -- testroms/48.rom "Skool Daze"
cargo run --release --bin speccy-gui -- testroms/48.rom "Jet Set Willy" fullscreen

# Themed terminal (TUI) head:
cargo run --release --bin speccy -- testroms/48.rom testroms/manic.z80 terminal
```

### `speccy-gui` options

`speccy-gui <48.rom> [game] [theme] [scaleN] [fullscreen] [audiodev=NAME] [audiolist]`

- **game** — a local `.tap`/`.sna`/`.z80`, or a title to fetch (e.g. `"Renegade"`).
- **theme** — `authentic` (default) · `dark` · `light` · `terminal` · `amber` · `gameboy`.
- **scaleN** — pixel zoom, e.g. `scale3` (default 3); ignored in fullscreen.
- **fullscreen** — start full screen; also toggle anytime via the green button,
  View ▸ Enter Full Screen, or F11. The **Audio** menu switches output device live
  (e.g. an AirPlay/TV speaker when projecting) — `audiolist` prints the choices.

### Check the game library

```bash
cargo run --release --bin speccy-library -- testroms/48.rom        # a curated set of classics
cargo run --release --bin speccy-library -- testroms/48.rom "Chaos" "Green Beret"
```

Fetches each title, loads it, runs a few seconds, and reports which render. The
core loads `.tap`/`.z80`/`.sna`; `.tzx`/custom-loader games (e.g. the Dizzy
series) are reported as needing real-time tape loading (a future item).

## MCP server

[`chuk-mcp-spectrum`](./chuk-mcp-spectrum/README.md) exposes the machine over MCP
as **two endpoints** — a tiny *agent* surface (observe + drive your session) and a
full *admin* surface (lifecycle, pokes, recording, snapshot timeline, and
`search_games`/`load_game` from World of Spectrum). The autonomy plane records
every session to MP4 and checkpoints a rewindable snapshot timeline. The Rust core
is exposed to Python via the [`zxspec_py`](./zxspec_py) PyO3 binding.

## Workspace layout

| Crate | What |
|---|---|
| [`z80`](./z80) | Pure `no_std` Z80 CPU. Owns no memory and no clock — all timing lives behind the `Bus` trait. |
| [`spectrum`](./spectrum) | The 48K machine: ULA video/contention, memory, keyboard, beeper, tape, snapshots, session recording. |
| [`display`](./display) | Theme + effect pipeline (palette remap / duotone ramp / scanlines). One `DisplayConfig`, every head. |
| [`frontend`](./frontend) | `speccy` (TUI), `speccy-gui` (native window), `speccy-library` (headless check). |
| [`wos`](./wos) | World of Spectrum search + download (ZXInfo API), shared by the CLI and MCP. |
| [`zxspec_py`](./zxspec_py) | PyO3 binding exposing the core to Python (maturin). |
| [`chuk-mcp-spectrum`](./chuk-mcp-spectrum) | The MCP server (two endpoints + autonomy plane). |

## Status & design

The emulator core is feature-complete (M0–M8). What's built and what's next is
tracked in the **[roadmap](./docs/roadmap.md)**; the design is split across six
specs indexed in **[docs/](./docs/README.md)**.

## Build & test

```bash
cargo test --workspace            # Rust core + heads
cargo test -p wos -- --ignored    # network-gated World of Spectrum fetch
```
