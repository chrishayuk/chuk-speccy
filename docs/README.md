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
| [03 — SDK / Developer Kit Spec](./03-sdk-spec.md) | The fidelity dial (pure vs hybrid), z88dk front-end, L0–L3 layers, and the `ED 70` **trap ABI** that lets Z80 apps escape to the host. |
| [04 — Spectrum-Native Chat / Agent](./04-spectrum-native-chat-spec.md) | L3 showpiece: a 32-column chatbot/agent where the Z80 is a dumb terminal and `chuk-llm` + `chuk-tool-processor` + MCP servers do the thinking. Two decoupled clocks + a typed-event poll. |
| [05 — Frontends & Display Pipeline](./05-frontends-display-spec.md) | Multi-head (desktop/web/TUI/MCP) over one core, and the shared theme + filter pipeline (palette remap / duotone ramp / effect chain). One `DisplayConfig`, every head. |
| [06 — Roles, Sessions & Autonomy](./06-roles-autonomy-spec.md) | Admin vs agent capability tiers, implicit per-session machines, and the autonomy plane (always-on recording, snapshot timeline, reaping) — agents as pure consumers. |
| [Roadmap](./roadmap.md) | **Delivery tracker** — what's built (core M0–M8) and what's next (MCP, SDK, chatbot, frontends, accuracy tail). |

## Implementation status

The **emulator core is feature-complete (M0–M8)** — a cycle-accurate, **ZEXALL-clean
(67/67)** 48K Spectrum: Z80, ULA video + contention, keyboard, beeper, `.sna`/`.z80`
snapshots, and `.tap` tape — with two heads (terminal + native pixel window) over
one themeable `display` pipeline. Try it:

```
cargo run --release --bin speccy-gui -- testroms/48.rom testroms/manic.z80   # pixel-perfect + sound
cargo run --release --bin speccy     -- testroms/48.rom testroms/manic.z80 terminal   # themed TUI
```

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
beeper → contention → tape). The layers on top — MCP server, SDK, chatbot, extra
frontends — attach over the finished core. Full sequencing in the
**[Roadmap](./roadmap.md)**.
