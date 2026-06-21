# Frontends & Display Pipeline — Multi-Head + Filters Spec

Companion to the [core](./01-core-emulator-spec.md) / [MCP](./02-mcp-server-layer-spec.md)
/ [SDK](./03-sdk-spec.md) / [chatbot](./04-spectrum-native-chat-spec.md) specs. Two
requests, one idea:

1. **Many UIs over one core** — desktop window, terminal (TUI), browser, MCP — all
   driving the same emulator.
2. **Themes & filters** — dark/light mode, green-phosphor terminal, CRT, etc.

Both fall out of a discipline already in place: the core emits **raw observations**
and owns **no presentation**. So heads are thin, and themes live in *one* shared
stage every head reuses. Nothing here touches the `z80` or `spectrum` crates.

---

## 1. The shape

```
   ┌───────────┐  observations  ┌──────────────────┐  surface  ┌──────────┐
   │  Machine  │ ─────────────▶ │  display pipeline │ ────────▶ │   head   │
   │ (core)    │  indexed fb,   │  theme → effects  │  RGBA /   │ window / │
   │           │  border, audio │  (shared crate)   │  cells    │ web /tui │
   │           │ ◀───────────── │                   │ ◀──────── │ /mcp     │
   └───────────┘    inputs      └──────────────────┘  keys      └──────────┘
```

- **Core** is always *stepped* — it never owns a clock or a window (established in
  the MCP spec). "Live vs headless" is a property of the **head**, not the core: a
  desktop head runs `run_frames(1)` at 50 Hz; the MCP head runs it on demand. Same
  Machine, no branching in the core.
- **Display pipeline** is a shared crate: indexed framebuffer → theme map → RGB →
  effect chain → final surface. Every head uses it; configure once, looks the same
  everywhere it *can*.
- **Heads** are thin adapters: pull a surface, push input events mapped to the
  keyboard matrix / Kempston joystick.

The universal head interface is the `Machine` API you already have
(`screen_rgba` / `screen_text` / `border` / audio buffer / `press_keys` /
`run_frames`). Adding a head is wiring, not core work.

---

## 2. The display pipeline (where themes live)

The ULA already produces a logically-*indexed* image: each pixel is ink-or-paper
of a cell, each cell one of 15 Spectrum colours. Keep that intermediate — don't
bake RGB in the core — because the theme stage wants the indices.

```
  indexed (ink/paper, 15 logical colours, + border)
        │
   [ THEME ]  one of two kinds:
        │   • palette remap   : 15 logical colours → 15 RGB  (stays colourful)
        │   • duotone ramp     : luminance → 2-colour ramp    (mono looks)
        ▼
  RGB raster (256×192 + border)
        │
   [ EFFECT CHAIN ]  ordered, optional, head-dependent
        │   scanlines · shadow-mask · phosphor-persist · bloom ·
        │   curvature · vignette · NTSC/PAL artifact · scaler
        ▼
  final surface (head picks resolution / format)
```

### 2.1 Theme taxonomy

| Kind | What it does | Examples |
|---|---|---|
| **Palette remap** | substitutes RGB for the 15 logical colours | `authentic` (real ULA values), `warm`, `inverted`, `gameboy-dmg`, `c64` |
| **Duotone ramp** | collapses to luminance, maps to a 2-colour ramp | `green-phosphor` (P1), `amber` (P3), `lcd-grey`, **`dark`** (dark paper / light ink), **`light`** |

So "dark mode", "light mode" and "terminal view" are all *duotone* themes;
"authentic" and the fantasy-console palettes are *remaps*. One enum, two arms.
(`authentic`'s exact RGB is famously debated — ship a sensible default, expose the
table so it's tweakable.)

### 2.2 Effect taxonomy

Pure raster post-process, composable, GPU-shader on capable heads:
`scanlines`, `shadow_mask` / `aperture_grille`, `phosphor_persist` (temporal blur
— the lovely smear), `bloom`, `curvature` (barrel), `vignette`, `chroma_artifact`
(composite NTSC/PAL fringing), and a `scaler` (nearest / sharp-bilinear / hqx).
"CRT" is just a named preset = `scanlines + shadow_mask + phosphor_persist + bloom
+ curvature + vignette`.

### 2.3 One config, passed to every head

```rust
pub struct DisplayConfig {
    pub theme: Theme,            // Remap(Palette) | Duotone(Ramp)
    pub effects: Vec<Effect>,    // ordered chain; empty = pixel-perfect
    pub integer_scale: bool,
    pub border: BorderMode,      // full | thin | hidden
}
```

Presets (`authentic`, `terminal`, `crt`, `dark`, `light`, `gameboy`) are just
named `DisplayConfig`s. **Expose it over MCP too** — `set_display(machine_id,
preset)` — so an agent can flip the machine into amber-CRT and `screenshot` it.
Themes apply to games, the BASIC prompt, and the chatbot alike.

---

## 3. The heads

| Head | Cadence | Theme stage | Effects | How it renders |
|---|---|---|---|---|
| **Desktop** (winit + softbuffer) ✓ | live 50 Hz | yes | theme stage (raster) | CPU framebuffer, aspect-correct letterboxed blit; `cpal` audio. *(GPU/shader effects are the future wgpu upgrade.)* |
| **Web / WASM** | live 50 Hz | yes | all (WebGL/WebGPU) | core compiled to `wasm32`, `<canvas>`, Web Audio |
| **Web / streamed** | live | server-side | server or client | core stays host-side, framebuffer over WebSocket to a dumb canvas |
| **Terminal (TUI)** | live or stepped | yes | limited (fake scanline only) | truecolor block renderer **or** text scrape |
| **MCP** | stepped | yes (software) | cheap only (scanlines) | `screenshot` runs the pipeline in software → PNG |

Notes that matter:

- **Desktop is a real app shell** (`speccy-gui`): native menus (muda) — full
  screen at runtime via the macOS green button / View ▸ Enter Full Screen / F11
  (any display), and an *Audio* menu to switch the output device live (e.g. an
  AirPlay/TV speaker when projecting). It also takes a **game title** as well as a
  file and fetches it from World of Spectrum (see the [roadmap](./roadmap.md)).
- **Web/WASM is nearly free** — and there's a nice irony: Rust *cannot* target the
  Z80, but it targets `wasm32` beautifully, so the entire core + display pipeline
  drops into the browser client-side with no rewrite. This is the best solo-play
  browser experience and reuses everything.
- **Web/streamed** earns its keep when you want the *host's* CHUK/MCP/agent loop in
  the session, or a shared live view multiple people watch, at the cost of
  bandwidth (stream RGBA deltas or a cheap codec, not full frames).
- **Terminal head, two modes:** (a) *graphics* — render 256×192 into the terminal
  with Unicode half-blocks / sextants and 24-bit ANSI (one cell = a 1×2 or 2×3
  pixel block); genuinely shows the Spectrum screen in a shell. (b) *text scrape* —
  use `screen_text()` for crisp, selectable text; the right choice for BASIC and
  the chatbot. "Terminal view" is therefore *both* a head (TUI) and a theme
  (`green-phosphor` duotone) — they compose: run the chatbot in the TUI head with
  the terminal theme and it's phosphor-on-phosphor, all the way down.
- **TUI can't do curvature/bloom** — it gets the theme stage and at most a dimmed
  alternate-row "scanline." State that limit rather than fake it badly.
- **MCP screenshots** run the theme stage + cheap raster effects (scanlines) in
  software so the agent sees a *themed* image; GPU-only effects stay head-local.

---

## 4. Input, the other direction

Each head maps native input → the Spectrum keyboard matrix (and Kempston/Sinclair
joystick) through one shared `input` table: desktop keydown, browser `KeyboardEvent`,
terminal keypress, and MCP `press_keys` all resolve to the same matrix pokes. Define
the host-key → matrix map once; heads supply only their native event source.

---

## 5. Why this stays cheap

This is the same principle paying off a third time: because the core emits raw
observations and owns no presentation, **heads and themes never touch `z80` /
`spectrum`.** A new head is an adapter; a new theme is a row in a table; a new
effect is a shader in the chain. The blast radius of "add a UI" or "add a CRT look"
is zero core code. That discipline — decided back when the `Bus`/`Machine` surface
was drawn — is what makes "interact over any UI, in any look" a config story
instead of a rewrite.

---

## 6. Build order

| Step | Build | When |
|---|---|---|
| 1 | `display` crate: indexed→RGB + theme enum (remap + duotone) | after core **M4** (video) |
| 2 | Desktop head already exists as the M4 window — point it at `display` | M4 |
| 3 | Preset `DisplayConfig`s (`authentic`/`terminal`/`dark`/`light`) | with step 1 |
| 4 | Effect chain as wgpu shaders (`scanlines`→`crt` preset) | after M6 |
| 5 | Web/WASM head (`wasm32`, canvas, Web Audio) | parallel, any time post-M5 |
| 6 | TUI head (block renderer + text-scrape modes) | parallel |
| 7 | MCP `set_display` + software theme in `screenshot` | extends MCP layer |
| 8 | Web/streamed head (WebSocket framebuffer) | only if shared/agent sessions wanted |

Themes (steps 1–3) are cheap and land early. Effects, extra heads, and streaming
are independent tracks you add in any order without disturbing each other or the
core.

---

## 7. Implementation note (this repo)

Step 1 + 3 landed with core M4: the `display` crate is standalone (depends on
neither `z80` nor `spectrum`), consuming an indexed `[u8]` (one logical colour
0–15 per pixel) + a border index, and emitting RGBA. `Theme` is the two-arm enum
(`Palette` remap / `Duotone` ramp); presets `authentic` / `dark` / `light` /
`terminal` (green phosphor) / `amber` / `gameboy` are named `DisplayConfig`s;
`Effect::Scanlines` is the first (software) entry in the chain. `Ula::screen_indexed`
is the raw-observation source the pipeline consumes; `screen_rgba` is just the
`authentic` preset, kept on the core as a convenience.
