# chuk-speccy

[![CI](https://github.com/chrishayuk/chuk-speccy/actions/workflows/ci.yml/badge.svg)](https://github.com/chrishayuk/chuk-speccy/actions/workflows/ci.yml)
&nbsp;License: MIT OR Apache-2.0

**A deterministic ZX Spectrum you can play, let agents drive, and compile Rust
games for.** Underneath is a cycle-accurate, **ZEXALL-clean** 48K emulator in Rust;
the point is what sits on top — an agent-controllable, bit-exact-reproducible *game
lab*. The core is a pure, deterministic, headless `Machine`; everything else is a
thin consumer of it (`frontend → spectrum → z80`; the `z80` crate never knows what
a Spectrum is).

![Snake, written in the rustz80 dialect, compiled to Z80 and running on the emulator](docs/assets/demo.gif)

*Snake — written in restricted Rust, compiled to Z80 by [`rustz80`](./rustz80/README.md),
booting on the emulator. The GIF was rendered headless by `speccy-gif` (no capture
tool): `speccy-compile snake.rs → speccy-gif → demo.gif`.*

## Three things you can do

**1 — Play Spectrum games.** Pixel-perfect + sound, fetched live from World of
Spectrum (or a local file), themes, fullscreen.
```bash
cargo run --release --bin speccy-gui -- testroms/48.rom "Jet Set Willy"
```

**2 — Let an AI agent observe and drive one.** The [MCP server](./chuk-mcp-spectrum/README.md)
exposes the machine over two endpoints (a small *agent* surface — screenshot, keys,
frame-step, memory — and a full *admin* surface), with **bit-exact checkpoints** and
a rewindable timeline. Records every session to MP4. That makes the Spectrum a
controlled, reproducible agent/RL environment, not just nostalgiaware.

**3 — Write a game in Rust and boot it.** The same `.rs` runs under `rustc` (host,
for debugging) *and* compiles **pure** to a real tape via [`rustz80`](./rustz80/README.md).
```bash
cargo run -p rustz80 --bin speccy-compile -- rustz80/samples/move.rs -o move.tap
cargo run --release --bin speccy-gui -- testroms/48.rom move.tap   # then press Q/A/O/P
```

**Don't want to build?** Grab a prebuilt `speccy-gui` (+ the CLIs) for macOS / Windows
/ Linux from the [latest release](https://github.com/chrishayuk/chuk-speccy/releases/latest).
Either way you supply a 48K system ROM at `testroms/48.rom` (gitignored — see
[Getting Started](./docs/getting-started.md)).

## Quick start

> New here? The **[Getting Started guide](./docs/getting-started.md)** walks through
> install → run a game → write your own → drive it over MCP.

You need a 48K system ROM at `testroms/48.rom` (gitignored — supply your own).

```bash
# Play a local tape/snapshot in the native window (pixel-perfect + sound):
cargo run --release --bin speccy-gui -- testroms/48.rom testroms/manic.z80

# …or just name a game — it's fetched live from World of Spectrum:
cargo run --release --bin speccy-gui -- testroms/48.rom "Skool Daze"
cargo run --release --bin speccy-gui -- testroms/48.rom "Jet Set Willy" fullscreen

# Themed terminal (TUI) head:
cargo run --release --bin speccy -- testroms/48.rom testroms/manic.z80 terminal

# A native Rust game (speccy-sdk) running on the substrate, and the chatbot:
cargo run --release --bin speccy-gui -- testroms/48.rom snake
cargo run --release --bin speccy-gui -- testroms/48.rom chat
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
core loads `.tap`/`.z80`/`.sna` instantly and `.tzx` in real time (signal-level,
so turbo/custom loaders like the Dizzy series work).

## MCP server

[`chuk-mcp-spectrum`](./chuk-mcp-spectrum/README.md) exposes the machine over MCP
as **two endpoints** — a tiny *agent* surface (observe + drive your session) and a
full *admin* surface (lifecycle, pokes, recording, snapshot timeline, and
`search_games`/`load_game` from World of Spectrum). The autonomy plane records
every session to MP4 and checkpoints a rewindable snapshot timeline. The Rust core
is exposed to Python via the [`zxspec_py`](./zxspec_py) PyO3 binding.

## Write a game in Rust, boot it on the Spectrum (`rustz80`)

[`rustz80`](./rustz80) is a small **Rust → Z80 compiler** for a restricted dialect
that is *also real Rust*: the same `.rs` runs under `rustc` (host, for debugging)
**and** compiles to Z80 that runs on the real machine — the two kept honest by
differential testing on the emulator. It supports `u8`/`u16`, arrays, `struct`,
`enum`/`match`, functions, `*`/`/`/`%`, bitwise ops, and `poke`/`peek` raw-memory
intrinsics — enough to write a game.

```bash
# Compile the dialect Snake to a bootable tape, then run it on the real ROM:
cargo run -p rustz80 --bin speccy-compile -- rustz80/samples/snake.rs -o snake.tap
cargo run --release --bin speccy-gui -- testroms/48.rom snake.tap
```

Any dialect `.rs` with a no-arg `fn main()` compiles the same way (the autoloader
`CLEAR`s, `LOAD`s the code, and `RANDOMIZE USR`s it). See
[spec 07](./docs/07-rust-z80-compiler-spec.md) and `rustz80/samples/`.

## Workspace layout

| Crate | What |
|---|---|
| [`z80`](./z80) | Pure `no_std` Z80 CPU. Owns no memory and no clock — all timing lives behind the `Bus` trait. |
| [`spectrum`](./spectrum) | The 48K machine: ULA video/contention, memory, keyboard, beeper, tape, snapshots, session recording. |
| [`display`](./display) | Theme + effect pipeline (palette remap / duotone ramp / scanlines). One `DisplayConfig`, every head. |
| [`frontend`](./frontend) | `speccy` (TUI), `speccy-gui` (native window), `speccy-library` (headless check). |
| [`wos`](./wos) | World of Spectrum search + download (ZXInfo API), shared by the CLI and MCP. |
| [`speccy-sdk`](./speccy-sdk) | Native Rust game SDK — write `Game::update`, run on the substrate over the trap ABI (Snake demo). |
| [`rustz80`](./rustz80) | Restricted Rust → Z80 compiler (`syn` frontend, own IR/codegen, mul/div micro-runtime) + `speccy-compile` (`.rs` → bootable `.tap`). |
| [`zxspec_py`](./zxspec_py) | PyO3 binding exposing the core to Python (maturin). |
| [`chuk-mcp-spectrum`](./chuk-mcp-spectrum) | The MCP server (two endpoints + autonomy plane). |

## Status & design

The emulator core is feature-complete (M0–M8). On top of it: the MCP server +
autonomy plane, a World-of-Spectrum game library, a disassembler, the `ED FE` trap
ABI, a Spectrum-native chatbot, a native Rust game SDK, and the **`rustz80`**
compiler (write Rust → boot it on a real Spectrum). What's built and what's next is
tracked in the **[roadmap](./docs/roadmap.md)**; the design is split across seven
specs indexed in **[docs/](./docs/README.md)**.

## Build & test

```bash
cargo test --workspace            # Rust core + heads
cargo test -p wos -- --ignored    # network-gated World of Spectrum fetch
```

## License

Dual-licensed under either [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE),
at your option.

## A note on ROMs and games

The 48K **system ROM is not included** — you supply your own (`testroms/48.rom`,
gitignored). The optional World-of-Spectrum fetcher downloads game binaries by
title for **personal and research use**; those games remain the property of their
respective rights holders. This project ships no game binaries and takes no
position on the copyright status of anything you choose to load into it.
