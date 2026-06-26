# Getting Started

`chuk-speccy` is a cycle-accurate 48K ZX Spectrum emulator in Rust, with native /
terminal heads, a World-of-Spectrum game fetcher, an MCP server, a native Rust game
SDK, and a Rust → Z80 compiler. This page takes you from a clone to running and
writing games.

## 1. Prerequisites

- A recent **Rust** toolchain (`rustup`, stable). Build everything with `cargo`.
- A **48K system ROM** at `testroms/48.rom`. It's gitignored — supply your own
  (any standard 48K ROM, 16 KB). Without it, the pure-Rust unit tests still run;
  only the ROM-backed integration tests and the GUI need it.
- macOS / Linux / Windows. Audio + the native window use `cpal` / `winit`.

**Fastest path (no clone):** install from crates.io —
```bash
cargo install speccy        # player + tools: speccy-gui, speccy, speccy-library, speccy-gif
# the rustz80 compiler + cell micro-VM are in cell80: cargo install rustz80
```
— or grab a prebuilt binary for your OS from the
[latest release](https://github.com/chrishayuk/chuk-speccy/releases/latest). You
still supply the ROM. The commands below use `cargo run` so they work whether you
installed the binaries or are building from a clone:

```bash
git clone <repo> && cd chuk-speccy
cargo build --workspace
cargo test --workspace          # ~90 unit/integration tests, no ROM needed
```

## 2. Run a game

The native window (pixel-perfect, sound, menus):

```bash
# A local tape/snapshot:
cargo run --release --bin speccy-gui -- testroms/48.rom testroms/manic.z80

# …or just name it — fetched live from World of Spectrum:
cargo run --release --bin speccy-gui -- testroms/48.rom "Skool Daze"
cargo run --release --bin speccy-gui -- testroms/48.rom "Jet Set Willy" fullscreen
```

`speccy-gui <48.rom> [game] [theme] [scaleN] [fullscreen] [audiodev=NAME] [audiolist]`
— `game` is a local `.tap`/`.sna`/`.z80` or a title to fetch; `theme` is one of
`authentic`/`dark`/`light`/`terminal`/`amber`/`gameboy`. Toggle full screen with the
green button / View menu / F11; the Audio menu switches output device live.

A themed terminal head (truecolor block glyphs):

```bash
cargo run --release --bin speccy -- testroms/48.rom testroms/manic.z80 terminal
```

Check the bundled game library (fetch + load + run a curated set):

```bash
cargo run --release --bin speccy-library -- testroms/48.rom
```

## 3. Write a game

Two ways, the two ends of the **fidelity dial**:

**(a) Native Rust SDK — `speccy-sdk`** (host power, instant iteration). Implement
`Game::update(&Input, &mut Frame)`; your Rust runs once per frame and draws into the
machine. See the **[`speccy-sdk` README](../speccy-sdk/README.md)**.

```bash
cargo run --release --bin speccy-gui -- testroms/48.rom snake   # the SDK demo
```

**(b) Restricted-Rust → Z80 — `rustz80`** (pure; boots on real hardware). Write a
subset of Rust that's *also real Rust*, compile it to a bootable `.tap`. See the
**[`rustz80` README](https://github.com/chrishayuk/cell80)**.

```bash
cargo run -p chuk-speccy-sdk --features compile --bin speccy-compile -- speccy-sdk/samples/snake.rs -o snake.tap
cargo run --release --bin speccy-gui -- testroms/48.rom snake.tap
```

The same dialect `.rs` also compiles under `rustc` for host debugging — the two are
kept honest by differential testing on the emulator.

## 4. Drive it from code / LLMs

- **Python**: [`zxspec_py`](../zxspec_py) is a PyO3 binding exposing the `Machine`
  (registers, memory, screen, step/run, snapshots, keyboard, audio, recording).
- **MCP**: [`chuk-mcp-spectrum`](../chuk-mcp-spectrum/README.md) serves the machine
  over MCP as two endpoints (a small *agent* surface + a full *admin* surface) with
  an autonomy plane (always-on recording → MP4, a rewindable snapshot timeline).

Because the core is headless and deterministic, a snapshot is a timeline branch —
the same machine serves the MCP server and an RL environment.

## 5. Tests & quality

```bash
cargo test --workspace                                   # core + heads (no ROM)
SPECTRUM_ROM="$PWD/testroms/48.rom" \
SPECTRUM_GAME="$PWD/testroms/manic.z80" \
  cargo test --workspace -- --ignored                    # ROM-backed integration
cargo clippy --workspace --all-targets                   # lint (clean)
cargo test -p wos -- --ignored                           # network-gated WoS fetch
```

ROM-backed tests are gated behind `SPECTRUM_ROM` (and `SPECTRUM_GAME` for the few
that load a game); network tests are `#[ignore]`.

## Where next

- **[Design docs index](./README.md)** — the seven specs (core, MCP, SDK, chat,
  frontends, autonomy, compiler).
- **[Roadmap](./roadmap.md)** — what's built and what's next.
- **Crate READMEs**: [`speccy-sdk`](../speccy-sdk/README.md),
  [`rustz80`](https://github.com/chrishayuk/cell80),
  [`chuk-mcp-spectrum`](../chuk-mcp-spectrum/README.md), [`wos`](../wos/README.md).
