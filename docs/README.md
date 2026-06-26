# chuk-speccy — Design Docs

A headless, deterministic ZX Spectrum emulator in Rust, exposed to LLMs / scripts
/ RL loops as an MCP server — and a developer kit + agent showpiece on top. The
Rust core stays format-agnostic; every consumer (MCP server, RL env, SDK, chat
terminal) is a thin re-skin over one `Machine` and one trap ABI.

## Specs

| Doc | What it covers |
|---|---|
| [01 — Core Emulator Spec](./01-core-emulator-spec.md) | Z80 core, ULA video/timing/contention, memory map, I/O, snapshots, test strategy, milestones. Cycle-accurate 48K first; 128K-ready architecture. |
| [02 — MCP Server Layer Spec](./02-mcp-server-layer-spec.md) | PyO3 boundary, headless/stepped execution model, the `Machine` surface, MCP tool catalog (incl. the `screenshot` PNG tool), and the free `chuk-rl-env` corollary. |
| [03 — SDK / Developer Kit Spec](./03-sdk-spec.md) | The fidelity dial (pure vs hybrid), z88dk front-end, L0–L3 layers, and the `ED FE` **trap ABI** that lets Z80 apps escape to the host. |
| [04 — Spectrum-Native Chat / Agent](./04-spectrum-native-chat-spec.md) | L3 showpiece: a 32-column chatbot/agent where the Z80 is a dumb terminal and `chuk-llm` + `chuk-tool-processor` + MCP servers do the thinking. Two decoupled clocks + a typed-event poll. |
| [05 — Frontends & Display Pipeline](./05-frontends-display-spec.md) | Multi-head (desktop/web/TUI/MCP) over one core, and the shared theme + filter pipeline (palette remap / duotone ramp / effect chain). One `DisplayConfig`, every head. |
| [06 — Roles, Sessions & Autonomy](./06-roles-autonomy-spec.md) | Admin vs agent capability tiers, implicit per-session machines, and the autonomy plane (always-on recording, snapshot timeline, reaping) — agents as pure consumers. |
| 07 — Rust → Z80 Compiler | `rustz80` (**built**, now in [cell80](https://github.com/chrishayuk/cell80)): a restricted Rust dialect that's *also real Rust*, compiled to a bootable `.tap` via `syn` + own IR/codegen + a micro-runtime. One source, both compilers — a dialect Snake boots on the real ROM. The spec + cell micro-VM live in the cell80 repo. |
| [08 — Speccy Kit: the Authoring Plane](./08-speccy-kit-authoring-plane-spec.md) | The synthesis on top of the closed dial: **one typed source → three artifacts** (host build · pure `.tap` · agent env), bridged by a compiler-emitted **symbol map**. Pins the kit (L1), assets (L0), the typed env surface, the two MCP planes, and the sequencing (prove the seam on Snake first). |
| [Getting Started](./getting-started.md) | **Start here** — install, supply a ROM, run a game, write one in Rust, drive it over MCP. |
| [Roadmap](./roadmap.md) | **Delivery tracker** — what's built (core M0–M8 + the layers) and what's next (Game-trait prelude, frontends, RL env, accuracy tail). |

## Implementation status

The **emulator core is feature-complete (M0–M8)** — a cycle-accurate, **ZEXALL-clean
(67/67)** 48K Spectrum: Z80, ULA video + contention, keyboard, beeper, `.sna`/`.z80`
snapshots, and `.tap` tape — with two heads (terminal + native pixel window) over
one themeable `display` pipeline, plus a **World of Spectrum** game fetcher and an
MCP server. **On top of the core, also built:** the MCP server + autonomy plane
(spec 02/06), a **native Rust game SDK** (spec 03, [`speccy-sdk`](../speccy-sdk/README.md)),
a **Spectrum-native chatbot** (spec 04), and (via the **`rustz80`** compiler, now in
[cell80](https://github.com/chrishayuk/cell80)) a restricted-Rust→Z80 game-compile flow
whose output **boots on the real ROM** (a dialect Snake runs). Try it:

```
# A local file…
cargo run --release --bin speccy-gui -- testroms/48.rom testroms/manic.z80   # pixel-perfect + sound
# …or fetch a game by name from World of Spectrum:
cargo run --release --bin speccy-gui -- testroms/48.rom "Skool Daze"          # add `fullscreen` to project it
cargo run --release --bin speccy     -- testroms/48.rom testroms/manic.z80 terminal   # themed TUI
# …or write a game in Rust and boot it:
cargo run -p chuk-speccy-sdk --features compile --bin speccy-compile -- speccy-sdk/samples/snake.rs -o snake.tap
cargo run --release --bin speccy-gui -- testroms/48.rom snake.tap
```

Or install from **crates.io** (v0.1.0): `cargo install speccy` (player + tools),
`cargo install rustz80` (the compiler), `cargo add chuk-speccy-spectrum` to build on
the core. Prebuilt binaries for macOS/Windows/Linux are on the
[latest release](https://github.com/chrishayuk/chuk-speccy/releases/latest).

New here? Start with **[Getting Started](./getting-started.md)**. The MCP server
(search/load games, drive + record sessions) lives in
[`../chuk-mcp-spectrum`](../chuk-mcp-spectrum/README.md).

The milestone-by-milestone breakdown, test inventory, and remaining tracks (MCP
server, SDK, chatbot showpiece, web/WASM head, RL env, accuracy long tail) live in
the **[Roadmap](./roadmap.md)**.

## One-line orientation

- **Core dependency arrow:** `frontend → spectrum → z80`. The `z80` crate never
  knows what a Spectrum is.
- **The Bus trait** is the Rust-specific design call: the CPU owns no memory and
  no clock; all timing lives in the bus.
- **Determinism is the feature, not a side-effect.** Headless + stepped means a
  snapshot is a timeline branch — which is what makes the same core serve both the
  MCP server and an RL environment.

## Build order at a glance

Core M0–M8 is done (workspace → Z80 → memory/ROM → ULA/keyboard → snapshots →
beeper → contention → tape). The layers on top — MCP server, SDK, chatbot, and the
`rustz80` compiler — are built over the finished core; remaining tracks (extra
frontends, RL env, accuracy tail) attach the same way. Full sequencing in the
**[Roadmap](./roadmap.md)**.
