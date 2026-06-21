# ZX Spectrum Emulator — MCP Server Layer

Companion to the [core spec](./01-core-emulator-spec.md). This exposes the
headless emulator as an MCP server via `chuk-mcp-server`, turning a 1982 Z80
machine into a tool an LLM (or a script, or an RL loop) can drive and introspect.

---

## 1. Why this is more than a toy

Three concrete framings, in increasing order of how much they justify the build:

1. **LLM-in-the-loop play.** `screenshot → reason → press_keys → run_frames`. A
   closed perception–action loop on real hardware behaviour. The screenshot tool
   returns an actual PNG, so the model *sees* the screen.
2. **LLM-as-Z80-debugger.** Breakpoint, step, inspect registers/memory,
   disassemble. The model drives a debugger over a deterministic CPU — a clean
   harness for "explain what this routine does / why it crashes."
3. **Deterministic, checkpointable substrate.** Because the emulator is headless
   and stepped (§3), `save_snapshot`/`load_snapshot` are a *timeline branch*: run
   inputs, checkpoint, try a different branch, roll back. That is exactly the
   shape of an RL environment — see §8, this falls out as a `chuk-rl-env` env
   almost for free, and it shares the same core with the MCP server.

The non-obvious payoff: **the MCP server and an RL environment are the same object
under two framings.** Snapshot = state reset, `run_frames` = step,
`screenshot`/`read_screen_text` = observation, `press_keys` = action,
`read_memory` = reward shaping. Build the headless core once; wear both hats.

---

## 2. Architecture — PyO3 boundary

`chuk-mcp-server` is Python; the emulator core is Rust. Bind them in-process with
PyO3/maturin rather than running a subprocess + wire protocol — no IPC, no
serialisation tax on `read_memory`/`screenshot`, and the emulator is naturally a
library anyway.

```
┌─────────────────────────────────────────────┐
│  chuk-mcp-spectrum  (Python, chuk-mcp-server) │
│   @tool screenshot / run_frames / press_keys… │
│   session registry: {machine_id -> Machine}   │
└───────────────┬───────────────────────────────┘
                │  PyO3 (maturin-built extension)
┌───────────────▼───────────────────────────────┐
│  zxspec_py   (#[pyclass] Machine)              │
│   thin wrapper over the `spectrum` crate       │
└───────────────┬───────────────────────────────┘
                │  plain Rust
        ┌───────▼────────┐   ┌──────────────┐
        │  spectrum crate │──▶│  z80 crate   │
        └─────────────────┘   └──────────────┘
```

The Rust core stays format-agnostic: it hands back **raw RGBA bytes**, **raw
register values**, **raw memory**. Anything MCP-shaped (PNG encoding, base64,
content blocks) happens in the thin Python layer. The `z80`/`spectrum` crates
never grow an `image` or MCP dependency.

---

## 3. Execution model — headless, stepped, deterministic

No 50 Hz wall-clock thread, no audio device, no window. The machine advances only
when a tool tells it to, in instruction or frame quanta:

- `step(n)` — n instructions.
- `run_frames(n)` — n full frames (each = one `/INT` cycle ≈ 69888 T-states).
- `run_until(pc=…, max_tstates=…)` — run to a target PC, a breakpoint, a HALT, or
  a cycle budget, whichever first.

Every tool call is **reproducible**: identical inputs from identical state yield
identical output, down to the T-state. This is what makes branching/checkpointing
meaningful and what makes it a sane test oracle. (You'll notice the resonance with
frozen-model trajectory capture — same deterministic-substrate game, on a 3.5 MHz
Z80 instead of a transformer. `trace(n)` in §6 is literally a trajectory dump.)

**Sessions.** Each `create_machine` mints a `machine_id` into a server-side
registry; later tools take that id. Multiple independent machines can coexist —
useful for parallel agents, A/B input testing, or RL rollouts. (The exact idiom
for holding that registry is whatever `chuk-mcp-server` prefers for cross-call
state — you know your own framework's session handling better than this doc does.)

---

## 4. The PyO3 surface (`Machine`)

```rust
#[pyclass]
pub struct Machine { spec: Spectrum }      // owns z80 + ula + memory

#[pymethods]
impl Machine {
    #[new]
    fn new(model: &str) -> PyResult<Self>;          // "48k" (later "128k")
    fn reset(&mut self);

    // state I/O
    fn load_snapshot(&mut self, fmt: &str, data: &[u8]) -> PyResult<()>;
    fn save_snapshot(&mut self, fmt: &str) -> PyResult<Vec<u8>>;   // checkpoint

    // execution -> StopReason {Completed, Breakpoint(u16), Halt, Budget}
    fn step(&mut self, count: u32) -> StopReason;
    fn run_frames(&mut self, n: u32) -> StopReason;
    fn run_until(&mut self, pc: Option<u16>, max_t: Option<u64>) -> StopReason;

    // observation
    fn registers(&self) -> RegSnapshot;             // -> dict on the Python side
    fn read_memory(&self, addr: u16, len: u16) -> Vec<u8>;
    fn disassemble(&self, addr: u16, count: u16) -> Vec<(u16, String, Vec<u8>)>;
    fn screen_rgba(&self) -> Vec<u8>;               // 256*192*4, ULA-decoded
    fn screen_text(&self) -> String;                // text-mode cell scrape

    // mutation / debug
    fn write_memory(&mut self, addr: u16, data: &[u8]);
    fn set_register(&mut self, name: &str, value: u32) -> PyResult<()>;
    fn press(&mut self, keys: Vec<String>, frames: u32);
    fn set_breakpoint(&mut self, addr: u16);
    fn clear_breakpoint(&mut self, addr: u16);
    fn trace(&mut self, count: u32) -> Vec<TraceStep>;
}
```

`screen_text()` needs no OCR: in text mode the 32×24 cell grid maps directly
through the known Spectrum character ROM back to ASCII. Near-free, and it's the
cheapest possible observation for an agent reading menus/BASIC.

---

## 5. MCP tool catalog

Thin `@tool` wrappers over the registry + `Machine`. Grouped:

| Group | Tool | Returns | Notes |
|---|---|---|---|
| Session | `create_machine(model="48k")` | `machine_id` | mints a session |
| | `reset(machine_id)` | ok | |
| | `destroy_machine(machine_id)` | ok | free the slot |
| State | `load_snapshot(machine_id, fmt, data_b64\|path)` | ok | sna / z80 |
| | `save_snapshot(machine_id, fmt="z80")` | `data_b64` | **checkpoint/branch** |
| Exec | `step(machine_id, count=1)` | `{stop, pc, op}` | single-step |
| | `run_frames(machine_id, n=1)` | `stop_reason` | the normal "advance" |
| | `run_until(machine_id, pc=…, max_tstates=…)` | `stop_reason` | |
| Observe | `get_registers(machine_id)` | reg dict | full Z80 + WZ/IFF/IM/T |
| | `read_memory(machine_id, addr, len)` | hex | |
| | `disassemble(machine_id, addr, count)` | listing | |
| | `screenshot(machine_id)` | **PNG image content** | ★ the headline tool |
| | `read_screen_text(machine_id)` | text | menu/BASIC scrape |
| Interact | `press_keys(machine_id, keys, frames=2)` | ok | matrix-level |
| | `type_text(machine_id, text)` | ok | sequences keypresses |
| Debug | `set_breakpoint(machine_id, addr)` | ok | |
| | `write_memory(machine_id, addr, data_b64)` | ok | poke / cheats |
| | `set_register(machine_id, reg, value)` | ok | |
| | `trace(machine_id, count)` | trajectory | ★ per-instr state path |

### The screenshot tool (the standout)

```python
@tool
async def screenshot(machine_id: str) -> ImageContent:
    """Render the current Spectrum display as a PNG the model can see."""
    m = registry[machine_id]
    rgba = bytes(m.screen_rgba())                 # 256x192x4 from Rust
    png  = Image.frombytes("RGBA", (256, 192), rgba).resize((512, 384), NEAREST)
    buf  = io.BytesIO(); png.save(buf, "PNG")
    return ImageContent(data=base64.b64encode(buf.getvalue()).decode(),
                        mimeType="image/png")
```

Returning an MCP **image content block** is what closes the loop — the model
literally looks at the screen and decides the next `press_keys`. (Encoding lives
in Python; Rust only ever emits raw RGBA.)

---

## 6. `trace` — the interpretability-flavoured one

`trace(n)` executes n instructions and returns the full path: `[{pc, opcode,
mnemonic, regs_after, tstate}]`. It's a deterministic execution trajectory — a
debugger's "step log," but also the same data shape you'd capture to study
control-flow dependence. Cheap to add (the disassembler + register snapshot
already exist) and disproportionately useful for "explain this routine" prompts.

---

## 7. Transport & packaging

- **STDIO** for a single local client (Claude Desktop–style): one server process,
  one or many machines. Simplest.
- **HTTP/SSE** if you want a hosted emulator multiple clients can attach to, or a
  long-lived machine an agent revisits across sessions.
- Package name fits the stack convention: **`chuk-mcp-spectrum`** (alongside
  `chuk-mcp-physics`, `chuk-mcp-solver`, …). maturin builds the `zxspec_py`
  wheel; the Python package depends on it + `chuk-mcp-server` + Pillow.

---

## 8. The RL-env corollary (free second hat)

Because §3 already gives determinism + snapshot checkpointing, a `chuk-rl-env`
`SpectrumEnv` is a near-trivial re-skin of the same `Machine`:

| RL concept | Spectrum mechanism |
|---|---|
| `reset()` | `load_snapshot(checkpoint)` |
| `step(action)` | `press_keys(action); run_frames(k)` |
| observation | `screen_rgba()` (pixels) or `read_memory(...)` (state vars) |
| reward | derived from `read_memory` (score address) or screen delta |
| terminal | game-over detection via memory/screen |
| branching | `save_snapshot` tree → MCTS-style rollouts |

So the build order is: headless `Machine` → MCP server → (optionally) RL env, with
the two consumers sharing one core. Worth keeping the `Machine` API free of any
MCP- or RL-specific assumptions for exactly this reason.

---

## 9. Where this slots into the core milestones

From the [core spec's milestone table](./01-core-emulator-spec.md#9-build-order-milestones),
the MCP layer attaches after **M5 (`.sna` loader)** — once you can load and run a
program headlessly, every tool above is a thin wrapper. Suggested insertion:

- After **M5**: stand up `zxspec_py` (PyO3) + `create_machine` / `run_frames` /
  `get_registers` / `read_memory` / `screenshot` / `read_screen_text`. That's
  already enough for LLM-in-the-loop play.
- After **M7 (contention)**: add `trace` / `disassemble` / breakpoints for the
  debugger framing (they want correct timing to be trustworthy).
- Anytime after: the `chuk-rl-env` `SpectrumEnv` re-skin.

---

## 10. Open decision (PyO3 boundary)

One thing worth deciding before pinning the boundary: should
`save_snapshot`/`load_snapshot` round-trip the **full** machine state (T-state
position within the frame, ULA flash counter, contention phase) rather than just
the `.z80` format's register+RAM? For a debugger and for RL reset-fidelity you
want the complete internal state, not the lossy on-disk format.

Recommendation: add a native `serialize_full()`/`deserialize_full()` pair
alongside the `.sna`/`.z80` loaders. Cheap to do, and it's what makes a snapshot a
true timeline branch rather than an approximate one.
