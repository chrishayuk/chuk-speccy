# ZX Spectrum Emulator — Rust Implementation Spec

Target: a cycle-accurate **48K** Spectrum first, with the architecture left open
for 128K (paging + AY) as a later phase. The Z80 core is written from scratch.

---

## 1. Scope & non-goals

**In scope (phase 1):**
- Hand-written Z80 core, full documented + undocumented opcode set, correct flags
  (including XF/YF, MEMPTR-derived bits, and the SCF/CCF Q quirk).
- 48K memory map, ULA video, beeper audio, keyboard matrix.
- `.sna` and `.z80` snapshot loading.
- Cycle-accurate timing *capable* architecture; contention can land as a milestone
  rather than a precondition.

**Deferred:**
- 128K paging, AY-3-8912, +2/+3 disk, Interface 1/microdrive.
- `.tzx` tape loading (do `.tap` first; tape is its own rabbit hole).

**Explicit non-goal:** don't chase demoscene-perfect floating bus on day one. Get
games running, then tighten timing against tests.

---

## 2. Crate layout

A workspace keeps the CPU reusable and `no_std`-friendly, and keeps platform I/O
out of the core.

```
zxspec/
├── z80/          # pure CPU: registers, decode, Bus trait. no_std, no deps.
│   └── src/
│       ├── lib.rs
│       ├── cpu.rs        # struct Cpu, step()
│       ├── decode.rs     # X/Y/Z/P/Q dispatch
│       ├── flags.rs      # flag table, SZ53P precompute
│       └── bus.rs        # trait Bus
├── spectrum/     # the machine: memory, ULA, contention, ports
│   └── src/
│       ├── lib.rs
│       ├── ula.rs        # video gen, port 0xFE, contention clock
│       ├── memory.rs     # 16K ROM + 48K RAM, Bus impl
│       ├── keyboard.rs   # 8x5 matrix
│       └── snapshot.rs   # .sna / .z80
├── frontend/     # binaries: window, terminal, library check
│   └── src/
│       ├── main.rs           # speccy: themed terminal (TUI) head
│       └── bin/gui.rs        # speccy-gui: native window (winit + softbuffer + cpal)
│       └── bin/library.rs    # speccy-library: headless "do these games work?" check
├── wos/          # World of Spectrum search + download (used by the CLI + MCP)
└── z80-tests/    # SingleStepTests JSON harness + ZEX runner
```

The dependency arrow is strictly `frontend → spectrum → z80`. The `z80` crate
never knows what a Spectrum is.

---

## 3. The Bus boundary (the Rust-specific design call)

The borrow-checker trap in emulators is the CPU wanting `&mut` to memory while the
memory subsystem (ULA) wants to advance its own clock during that same access. The
clean resolution: **the CPU owns no memory and no clock.** It borrows a `&mut impl
Bus` for the duration of `step()`, and *all timing lives in the bus*.

```rust
pub trait Bus {
    // Data path. Each call is one Z80 memory/IO machine cycle.
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    fn input(&mut self, port: u16) -> u8;
    fn output(&mut self, port: u16, val: u8);

    // Timing path. The CPU reports *when* it touches the bus so the ULA
    // can inject contention and advance the frame clock.
    fn contend(&mut self, addr: u16, cycles: u32); // pre-access stall
    fn tick(&mut self, cycles: u32);               // pure internal cycles
}
```

Why this shape:
- **Single source of truth for T-states** is the ULA inside the bus, because the
  ULA is the thing that needs the frame position to decide both contention *and*
  what pixel to draw. The CPU never holds the master clock.
- `contend()` is separated from `read()` because the Z80 contends *before* the
  access at known points in each M-cycle, and the stall amount depends on the
  frame position the bus already knows.
- The trait is small enough that a `FlatBus` (RAM only, no contention) is a
  20-line test double — which is exactly what you run the JSON opcode tests
  against.

For full cycle accuracy each opcode calls `contend`/`read`/`tick` in the right
order and counts (e.g. `LD A,(HL)` = opcode fetch 4T, then a contended read 3T).
A non-accurate first pass can have `read`/`write` internally do `tick(3)` and make
`contend` a no-op; you upgrade in place later without touching the CPU.

---

## 4. Z80 core

### 4.1 Register file

Represent as explicit `u8`s with pair accessors rather than a `[u8;26]` blob —
the codegen is identical and the field access reads better.

```rust
#[derive(Default)]
pub struct Regs {
    pub a: u8, pub f: u8,
    pub b: u8, pub c: u8,
    pub d: u8, pub e: u8,
    pub h: u8, pub l: u8,
    // shadow set
    pub a_: u8, pub f_: u8,
    pub b_: u8, pub c_: u8,
    pub d_: u8, pub e_: u8,
    pub h_: u8, pub l_: u8,
    pub ix: u16, pub iy: u16,
    pub sp: u16, pub pc: u16,
    pub i: u8, pub r: u8,
    pub wz: u16,   // MEMPTR — needed for BIT n,(HL) flag bits & some XF/YF
}

impl Regs {
    #[inline] pub fn bc(&self) -> u16 { (self.b as u16) << 8 | self.c as u16 }
    #[inline] pub fn set_bc(&mut self, v: u16) { self.b = (v >> 8) as u8; self.c = v as u8; }
    // de, hl, af likewise
}
```

CPU state beyond registers: `iff1, iff2: bool`, `im: u8` (0/1/2),
`halted: bool`, and a one-bit `q` (see flags). Keep the IX/IY prefix handling as a
*pointer to which HL-pair to use* threaded through the addressed-memory opcodes,
rather than duplicating the opcode table three times.

### 4.2 Decode strategy — don't write a 256-arm match

Use the **Dinu X/Y/Z/P/Q decomposition**. Split the opcode byte:

```
   7 6   5 4 3   2 1 0
  [ x ] [  y  ] [  z  ]      p = y >> 1,  q = y & 1
```

This collapses the whole base table into a handful of structured arms. E.g. the
entire `LD r,r'` block is `x==1` (with `(6,6)` = HALT carved out), ALU ops are
`x==2` indexing `[add,adc,sub,sbc,and,xor,or,cp][y]` over `r[z]`, and the
`0xCB`/`0xED`/`0xDD`/`0xFD` prefixes each get their own small decoder reusing the
same `r[]`/`rp[]`/`cc[]` lookup arrays. This is ~5× less code than a flat match
and far less error-prone for the undocumented `DDCB`/`FDCB` corners.

Register-select arrays:
```rust
// z/y index → register; index 6 means (HL) / (IX+d) / (IY+d)
const R:  [Reg8; 8]  = [B, C, D, E, H, L, MEM, A];
const RP: [Reg16; 4] = [BC, DE, HL, SP];
const CC: [Cond; 8]  = [NZ, Z, NC, C, PO, PE, P, M];
```

### 4.3 Flags

Precompute the boring bits. Build a 256-entry `SZ53P` table at init (sign, zero,
the two undocumented bits 5/3 copied from the result, and parity) and a separate
`SZ53` for non-parity ops. ALU carry/half-carry you compute inline.

The accuracy traps, in order of how often they bite:
- **XF/YF (bits 5,3):** copied from the *result* for most ops, but from specific
  operands for `BIT`, `CPI/CPD`, `IN`, and block ops. Get these from a known-good
  core or the test vectors.
- **MEMPTR/WZ:** `BIT n,(HL)` takes bits 5/3 of F from `WZ` high byte, not the
  result. WZ is updated by a specific list of instructions. Implement it the
  moment you start failing `BIT` flag tests.
- **SCF/CCF Q quirk:** YF/XF after SCF/CCF depend on whether the *previous*
  instruction touched F (`Q`). Track `q = (this instruction wrote F)` and feed
  last cycle's `q` into SCF/CCF. This is the last ~1% and only matters for ZEXALL
  / strict tests.

### 4.4 Interrupts

- Maskable INT: the ULA holds `/INT` low for **32 T-states** at frame start.
  Accept it at instruction boundaries when `iff1`. IM 1 → `RST 38h`; IM 2 →
  vector from `(I<<8 | bus_byte)`, where the byte is typically `0xFF` on a 48K.
- HALT executes `NOP`s (advancing R and the clock) until an interrupt; model it as
  staying on the HALT opcode with PC frozen, then `pc+=1` on wake.
- NMI exists but the 48K never asserts it — stub it.
- `EI` delays interrupt enable by one instruction (the classic `EI; RET` window).

---

## 5. The ULA — timing, video, contention

This is where a Spectrum emulator earns its accuracy. Constants for the 48K
(PAL, 3.5 MHz):

| Quantity | Value |
|---|---|
| T-states per frame | 69888 |
| T-states per scanline | 224 |
| Scanlines per frame | 312 |
| Display | 256 × 192, centred in a 320×240-ish border |
| Frame rate | ~50.08 Hz |
| `/INT` asserted | 32 T-states at frame start |

### 5.1 Video memory layout

The infamous non-linear screen address. For pixel row `y` (0–191), the byte
address bits are interleaved thirds:

```
addr = 0x4000
     | (y & 0b11000000) << 5   // which third -> offset bits 11-12
     | (y & 0b00000111) << 8   // pixel row within char -> bits 8-10
     | (y & 0b00111000) << 2   // char row within third -> bits 5-7
     |  x_byte;                // 0..31 -> bits 0-4
```

> Note: the "which third" bits must be shifted `<< 5` (to offset bits 11–12) —
> the third selects a 2 KB block (0x000/0x800/0x1000), not a small offset. (An
> earlier draft of this line dropped the shift; that lands the copyright text
> off-screen.) The 13-bit offset is `Y7 Y6 | Y2 Y1 Y0 | Y5 Y4 Y3 | X4..X0`.

Attributes are linear: `0x5800 + (y/8)*32 + x_byte`, one byte per 8×8 cell:
`FLASH(7) BRIGHT(6) PAPER(5..3) INK(2..0)`. FLASH swaps ink/paper every 16 frames
(a frame counter `& 0x10`).

### 5.2 Rendering model

Simplest correct approach: **render per frame** from the final memory state into
an RGB framebuffer, then blit. This is wrong for mid-frame writes (multicolour
demos) but right for ~all games. Upgrade path is to render per-scanline driven by
the ULA clock, then per-T-state for the deep end.

### 5.3 Contention

Contended region: RAM `0x4000–0x7FFF` (the bottom 16K), only during the 192-line
display area, only on the first 128 T-states of each line. The ULA stalls the CPU
by a delay that depends on the T-state position within an 8-cycle window:

```
delay pattern (offset into 8T window): [6, 5, 4, 3, 2, 1, 0, 0]
```

The cleanest implementation is a **precomputed `[u8; 69888]` contention table**:
index by the current frame T-state, get the stall to add. `contend(addr, n)` then
checks if `addr` is contended and adds `table[tstate]` before ticking. Port
contention (port `0xFE` and the bottom-16K-address rule) follows a related but
separate pattern — table that too.

Ship without contention; add the table when you want the timing tests and
border/multicolour effects to pass. Because it's isolated in `contend()`, this is
a self-contained change.

---

## 6. I/O

**Keyboard** — 8 half-rows of 5 keys, read via port `0xFE` with the *high* byte
selecting the row(s) (active-low). A read returns bits 0–4 = pressed keys for the
selected row(s), bit 5 unused, bit 6 = EAR (tape in), bit 7 high. Maintain an
`[u8; 8]` of row states; host key events flip bits.

**Beeper** — bit 4 of writes to port `0xFE` is the speaker; bit 3 is MIC/tape out;
bits 2–0 are the border colour. Audio is generated by recording the speaker bit's
state against the T-state clock, then resampling that square wave to the host rate
(44.1/48 kHz) once per frame for `cpal`. A simple averaging downsample is plenty;
band-limiting (PolyBLEP) is a nice-to-have later.

---

## 7. Snapshot & tape loading

- **`.sna` (48K):** dead simple — 27-byte header (registers, including a quirk
  where PC is pushed on the stack) + 48K RAM dump. Splat into state, `RETN`-style
  pop of PC. Best first loader.
- **`.z80`:** versioned, optionally RLE-compressed pages; v1 is 48K-only, v2/v3
  add 128K. Handle v1 first.
- **`.tap`:** sequence of blocks with a length prefix. Either emulate the ROM
  loader edge timing for authenticity, or **trap** the ROM load routine (`LD-BYTES`
  at `0x0556`) and inject bytes directly for instant loads. Do the trap version
  first; real-time tape is phase 2.

---

## 8. Test strategy (do this early, not last)

Two independent layers, both worth wiring up before you trust a single game:

1. **Per-opcode JSON tests** — the SingleStepTests / "jsmoo" Z80 set: thousands of
   cases per opcode giving initial state → expected final state *and cycle-by-cycle
   bus activity*. Run these against a `FlatBus` test double. This catches flag and
   undocumented-opcode bugs in isolation, with pinpoint diffs. This is the single
   highest-leverage thing you can build.
2. **ZEXDOC then ZEXALL** — run the real test ROM inside the emulator (needs CP/M
   `BDOS` print stubs, or use the Spectrum-targeted port). ZEXDOC passes without
   the XF/YF/Q minutiae; ZEXALL requires all of it. Passing ZEXALL ≈ "the CPU is
   done."

Order: JSON tests green → ZEXDOC green → games boot → contention table → ZEXALL
green → timing-sensitive demos.

---

## 9. Build order (milestones)

| # | Milestone | You can... |
|---|---|---|
| 0 | Workspace + `Bus` trait + `FlatBus` | run JSON tests against an empty CPU shell |
| 1 | Z80 core, documented opcodes | pass most JSON tests; ZEXDOC partial |
| 2 | Undocumented opcodes + flags + WZ + Q | pass ZEXDOC, then ZEXALL |
| 3 | 48K memory map + ROM + INT timing | boot the ROM to the `(C) 1982` prompt |
| 4 | ULA video (per-frame) + keyboard | type BASIC; see the screen |
| 5 | `.sna` loader | load and run Manic Miner |
| 6 | Beeper + `cpal` | hear it |
| 7 | Contention table | pass timing tests; border effects |
| 8 | `.z80` / `.tap` (trap-load) | load most of the library |

Phases 0–6 get you a real, playable emulator. 7–8 are the accuracy tail.

---

## 10. Decisions to lock before coding

- **48K-only or 128K-ready memory?** Recommend writing `memory.rs` against a
  `Bank`/paging-capable interface from the start (cheap) but only wiring the 48K
  config — saves a refactor when AY/paging arrive.
- **Per-frame vs per-scanline video for phase 1?** Recommend per-frame; the ULA
  clock is already there for contention, so per-scanline is a localised upgrade.
- **Tape: trap-load vs real-time first?** Recommend trap-load; real-time tape
  shares no code with anything else and can wait.
