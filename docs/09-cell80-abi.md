# Cell80 ABI — v1

The frozen contract every `rustz80-cell` consumer (CLI, MCP server, tool index, future
`.cell` cartridge) relies on. **`ABI_VERSION = 1`** (`rustz80::cell::ABI_VERSION`, also the
`"abi"` field of the JSON report). Bump only on a breaking change to anything below.

The cell is a **flat-RAM Z80** — no ROM, no ULA, no I/O ports by default, no syscalls —
behind `--features cell`. Determinism is the whole point: same program + same inputs ⇒
identical result, identical cycle count, identical touched-set (asserted by
`runner_reuse_is_deterministic`).

## Memory map

| region | address | purpose |
|---|---|---|
| trampoline | `0x7000` | argument loader + `CALL entry` + `HALT` (written per run) |
| code (`ORG`) | `0x8000` | the compiled program |
| scratch / locals | `0x9000` | the "virtual register file": local `i` at `0x9000 + i*2` |
| typed state (convention) | `0xB000` (`STATE_BASE`) | where `StateCell` lays a state struct |
| stack | `0xFFF0` (`SP_TOP`), grows down | |

64 KiB flat. A program may read/write anywhere it has the capability for; the runner zeroes
only the bytes a run *touched* before the next run.

## Calling convention

- **Entry**: an exported `fn` (default resolution: `run`, then `main`; or an explicit name,
  e.g. `State::run`).
- **Arguments**: up to three `u16`, passed in `HL`, `DE`, `BC` (in that order). A method
  entry (`State::run(&mut self)`) takes the state base in `HL`.
- **Return**: the result is in **`HL`**; a `-> (u16, u16, u16)` tuple fills `HL`, `DE`, `BC`.
  The report exposes all three as `regs[0..3]` (`result == regs[0]`).

## Typed I/O

State lives as a `struct` in memory (by convention at `STATE_BASE = 0xB000`).
`rustz80::struct_layout(src, "State")` returns each field's slot `offset` and `slots`; a
scalar field is one 2-byte slot at `base + offset*2` (`u8` in the low byte). Callers:

- **inputs**: write typed values before the run (`Runner::run_with_inputs`, CLI
  `--set addr:ty=val`), applied after the reset and cleaned before the next run;
- **outputs**: read typed values from post-run memory (`Runner::read_named` / `peek_u8/16/32`,
  CLI `--read name@addr:ty`);
- **by name**: `StateCell::bind(src, "State", entry)` does the name↔address mapping —
  `set("x", v)` → `run` → `get("score")`.

Field-state through this map is **differentially verified against rustc**
(`struct_field_state_matches_host`), not just against expected literals.

## Cycle budget & the `cycles` caveat

Each run is bounded by a T-state `budget` (the deterministic liveness guard); exceeding it
halts with `cycle_budget`. The reported **`cycles` is a deterministic *relative* cost
metric, not authentic Z80 time**: in Cell mode `*`/`/`/`%` and `[v; N]` fills are `ED FE`
host traps serviced natively and charged a flat ~4 T-states each, *not* their
software-routine cost. So `cycles` is correct for liveness and replay, but **must not** be
used as a hardware-fidelity figure or an RL reward — that would reward pushing work into
traps that read as "free." (The authentic `Spectrum48` target keeps the real software
routines; only the `Cell` target traps.)

## Halt status

| `halt` | meaning |
|---|---|
| `returned` | the entry returned cleanly |
| `halted` (+ `halt_code`) | the program called `halt(code)` (an `ED FE` trap) |
| `cycle_budget` | the T-state budget was reached first |
| `memory_limit` | the `max_touched` ceiling was reached |

## Capability model

`CellConfig` gates the raw intrinsics and caps resources; **`default()` = `sandboxed()`**:

| field | `sandboxed()` (default) | `permissive()` |
|---|---|---|
| `allow_raw_memory` (`poke`/`peek`) | `false` | `true` |
| `allow_ports` (`inport`) | `false` | `true` |
| `max_code_bytes` | `Some(4096)` | `None` |
| `max_touched` | `Some(4096)` | `None` |

A program using a denied intrinsic fails to compile under that policy. The policy travels
with a compiled `CellProgram` (and its serialized image).

## Report JSON (v1)

`Report::to_json()` emits, in order:

```json
{"abi":1,"entry":"run","entry_addr":32768,"result":42,"regs":[42,0,0],
 "cycles":67,"budget":2000000,"halt":"returned","code_bytes":47,"functions":1,
 "symbols":{"run":32768},"memory_touched":[[36864,36867]],"reads":{}}
```

- `abi` — schema version (this document).
- `entry` / `entry_addr` — the function run, and its address.
- `result` / `regs` — `HL`, and `[HL, DE, BC]`.
- `cycles` / `budget` — see the caveat above.
- `halt` — one of the statuses above; `halt_code` is present only for `halted`.
- `code_bytes` / `functions` — compiled size and function count.
- `symbols` — name → address, sorted by address.
- `memory_touched` — contiguous written ranges, `[start, end_inclusive]`.
- `reads` — named typed values requested via `--read` (else empty).

## Image format (cartridge seed)

`CellProgram::to_bytes()` / `from_bytes()` serialize a compact, self-contained image (magic
`CZ80`, version, code, symbols, policy) with no `syn` — the seed of the future `.cell`
cartridge. A named, versioned, manifest-bearing artifact is the gate for the standalone
spin-out (see roadmap B5/B6).
