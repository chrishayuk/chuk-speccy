# ZX Spectrum Emulator — Developer Kit / SDK Design

Companion to the [core](./01-core-emulator-spec.md) + [MCP](./02-mcp-server-layer-spec.md)
specs. Question on the table: an SDK so people build apps/games for the machine
**without hand-writing Z80**. The design hinges on one decision, so this doc leads
with that.

---

## 0. The one decision: the fidelity dial

Every SDK choice flows from where you set this:

- **Pure mode** — everything compiles to Z80 and runs on a *real* Spectrum (or any
  standard emulator). Maximum authenticity, maximum constraint (no hardware
  multiply, 3.5 MHz, colour clash, 48K).
- **Hybrid / HLE mode** — the app is still a Z80 program, but it can *escape to the
  host* through a trap ABI (§4) for heavy lifting: fast math, asset decode,
  sound, even calls into your MCP/CHUK servers. Maximum developer velocity; runs
  only on **your** emulator.

These aren't either/or — the good design is **one API with a fidelity dial**: the
same `mul16(a,b)` is a slow Z80 routine in pure mode and a one-cycle host trap in
hybrid mode, chosen at build time. But you should decide *which is the default*
and whether "runs on real hardware" is a goal you're willing to keep, because it
constrains the trap layer hard.

My read, given your stack: **default hybrid, keep pure buildable.** The hybrid
trap layer is the part that's distinctively yours and reuses machinery you already
have (it's the virtual-expert interception pattern, §4). Pure mode is mostly a
solved problem you should *stand on*, not rebuild (§2).

---

## 1. What an "SDK" actually has to provide

Independent of mode, four things:

1. **Toolchain / runner** — one command: source → loadable artifact (`.tap`/`.sna`)
   → launched in the emulator, with a fast edit-build-run loop. This is the
   single biggest DX win and it's *all yours* because you own the emulator.
2. **Asset pipeline** — modern PNG/aseprite art → Spectrum bitmap+attribute
   format; tilemaps; level data; beeper/AY music. Host-side Rust converters.
   Pure-win regardless of mode; this is where a lot of the real friction lives.
3. **Runtime / framework** — sprites, tilemap, text, input, sound, RNG, fixed-point
   math. The "game engine" library the author actually calls.
4. **Std-lib gaps** — the Z80 has no multiply/divide; strings and 16-bit math are
   painful. Someone has to provide these.

Items 1 and 2 you build no matter what. Item 3/4's *implementation* is what the
fidelity dial swaps out.

---

## 2. Authoring front-end: don't write a Z80 compiler

Be honest about prior art before building anything: **z88dk** (C for Z80, mature,
Spectrum targets, `appmake` produces `.tap`) and **Boriel's ZX BASIC** (a fast
compiled BASIC dialect) already solve "high-level language → Z80." Re-doing a C
backend is months of work for a worse result. Two sane front-end choices:

| Path | What it is | Cost | Fit |
|---|---|---|---|
| **Stand on z88dk / ZX BASIC** | Your SDK = libraries + toolchain glue over an existing compiler | Low | Fastest to a real game |
| **Rust eDSL assembler** | A crate emitting Z80 with a typed builder + labels/reloc; grow expression compilation later | Medium–High | Most "ours", workspace-native, but you're partly building a compiler |

Recommendation: **start on z88dk (C)** for the pure-Z80 front end — it gives you a
real language, a calling convention, and an existing community of code — and add a
**Rust eDSL** later only if you want the authoring experience fully inside your
workspace. The eDSL is a fun project but it's a *second* project; don't let it
gate the SDK.

The distinctive value you build yourself is **not** the compiler — it's the
runner, the asset pipeline, the trap ABI, and the framework library.

---

## 3. Layered architecture

```
  L3  CHUK/MCP bridge        ← Spectrum app calls your MCP servers via traps (§5)
  L2  trap ABI / HLE         ← host-escape syscalls (the distinctive layer, §4)
  L1  framework + std-lib    ← sprites, tilemap, input, sound, math, RNG
  L0  toolchain + assets     ← one-command build→run, PNG→Speccy pipeline
 ─────────────────────────────────────────────────────────────────────
  front-end: z88dk C  (and/or Rust eDSL)
  target:    your emulator   (pure artifacts also run on real hardware)
```

L0 and L1 are a normal retro game SDK. L2/L3 are the parts that only exist because
you own the emulator and the CHUK stack.

---

## 4. The trap ABI (the distinctive layer) — **built**

This is the same trick as your virtual-expert architecture — intercept execution,
route to an external implementation — applied to the Z80. The Spectrum program
hits a magic instruction; the emulator dispatches to a registered host function;
results come back in registers/memory.

**Trap mechanism.** The reserved opcode is **`ED FE`** (`HOSTCALL`, [`z80::TRAP_OP`]).
It's genuinely undefined on a real Z80 *and* on the ZX Spectrum Next's extended
ED set — so a hybrid binary on real hardware degrades to a clean NOP rather than
mis-executing. The **syscall id is in `A`** (not a trailing byte): a lone `ED FE`
NOPs cleanly, so a stray id can't be mis-decoded, and a binary can probe with
`HOST_PRESENT` (§6) and fall back at runtime.

```
  ld a, <id>
  defb $ED, $FE   ; HOSTCALL
```

> Implementation (ties to the core): `z80`'s ED decoder special-cases `ED FE` and
> calls a defaulted `Bus::host_trap(&mut regs) -> u32` (extra T-states); all other
> undefined ED slots still NOP. With no host installed it's a NOP, exactly like
> real silicon. The disassembler renders it as `HOSTCALL`.

**Calling convention.** id in `A`; small args in `BC`/`DE`/`HL`; structured args
via an `IX`/`HL`-pointed RAM parameter block. Return in `HL`/`A`; **carry = error**.
The `spectrum` side is a registry off the `Bus`:

```rust
// spectrum::host
pub trait HostCalls: Send { fn dispatch(&mut self, ctx: &mut HostCtx) -> u32; }
// HostCtx gives ctx.id() (= A), regs, ctx.read/write(addr,..), ctx.ok()/fail().
// FnTable maps id -> Rust closure (math/asset/tests); a PyO3 PyDispatcher
// forwards to a Python callable. Install with Machine::set_host_dispatcher.
```

**What you expose through it (hybrid mode):**
- `mul16/div16/sqrt`, fixed-point trig — the math the Z80 is bad at, host-cheap.
- Asset decode: unpack compressed graphics/levels straight into screen/RAM.
- Sound: hand a note/SFX descriptor to a host beeper/AY synth.
- Sprite compositing helpers (with the caveat below).

**Fidelity caveat worth stating up front:** host helpers that *composite into
Spectrum screen RAM* still obey the real display model — colour clash and the
256×192/attribute grid remain, because the pixels still live in `0x4000–0x5AFF`.
You only escape clash if you add a **separate host display layer** (full HLE
framebuffer), which is a hard authenticity break and a different product. Keep
host helpers writing into real screen RAM unless you've consciously chosen to
build a fantasy-console layer.

**The `mode` swap.** In pure builds, each L1 primitive resolves to its Z80
implementation; in hybrid builds, to an `ED FE` trap. Same call site. This is the
fidelity dial made concrete — a build flag, not a code rewrite.

---

## 5. L3 — the genuinely novel bit: Spectrum apps that call MCP

Once L2 exists, a syscall can route to *anything the host can reach* — including
your CHUK MCP servers. That makes possible things that have no precedent on a
1982 machine:

- A text adventure whose parser is an LLM, reached by a trap that ships the input
  line out and reads a structured response back.
- A "physics demo" where the Z80 does presentation and `chuk-mcp-physics` does the
  simulation.
- A maritime/heritage browser on the Speccy backed by `chuk-mcp-maritime-archives`
  / `chuk-mcp-her`.

Mechanically it's `ED 70 <id>` → host handler → MCP client → result marshalled
back into a RAM block. It's gimmicky and it's also a very clean demo of the whole
stack (emulator + trap ABI + CHUK servers) in one artifact. Worth one showpiece —
see [04 — Spectrum-Native Chat / Agent](./04-spectrum-native-chat-spec.md) for the
flagship build of exactly this.

---

## 6. The colour-clash reality (any SDK must confront this)

The Spectrum's defining constraint: 256×192 pixels but colour is per-8×8 cell
(one ink + one paper). A sprite crossing a cell boundary repaints that cell's
colour. An SDK has to *take a position*:

- **Manage it** — masking/alignment helpers, cell-aligned sprite modes, a tile
  system that keeps sprites within attribute boundaries. (Authentic; how real
  games coped.)
- **Hide it** — monochrome-with-fixed-attributes mode, or the host-display escape
  hatch (breaks fidelity).

Recommend providing *both a clash-aware sprite API and a monochrome fast-path*,
and documenting the trade. This is the one piece of domain knowledge a generic
game framework can't paper over.

---

## 7. RL-env corollary (reuse, again)

The trap ABI doubles as a **task-harness hook**: a host trap can report
score/terminal directly to a `chuk-rl-env` `SpectrumEnv`, so authored games come
with reward signals for free, no memory-address sniffing. The SDK author writes
`env_report(score, done)` and the env consumes it. Same core, third hat — see the
[MCP spec §8](./02-mcp-server-layer-spec.md#8-the-rl-env-corollary-free-second-hat).

---

## 8. Recommendation & phasing

1. **L0 first, on z88dk.** One-command `build → .tap → run-in-emulator`, plus a
   PNG→screen/sprite converter. This alone makes the emulator pleasant to develop
   for and is fully authentic.
2. **L1 framework in C** over z88dk — sprites (clash-aware + mono fast-path),
   tilemap, input (Kempston/Sinclair), beeper SFX, fixed-point math, RNG.
3. **L2 trap ABI** — the `ED 70` syscall + host dispatch table + the math/asset
   host helpers, behind a pure/hybrid build flag.
4. **L3 showpiece** — one app that calls an MCP server through a trap, to prove the
   stack.
5. *Optional later:* the Rust eDSL front-end, for workspace-native authoring.

Slots after **M5** of the core spec (loadable artifacts running headlessly) and
benefits from the MCP layer already existing for L3/L5.

---

## 9. What not to build

- A from-scratch C/Z80 compiler — stand on z88dk.
- A clash-escaping host display layer *by default* — that's a different product
  (a fantasy console wearing a Spectrum costume); offer it only as an explicit
  opt-in, eyes open.
- The Rust eDSL as a prerequisite — it's a fun parallel track, not a gate.

The honest summary: ~70% of a "pure" Spectrum SDK already exists in z88dk; your
build-worthy, differentiated surface is the **integrated runner + asset pipeline +
trap ABI + MCP bridge**, with the fidelity dial as the organising idea.
