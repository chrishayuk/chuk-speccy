# 08 — Speccy Kit: the Authoring Plane (one typed source, three artifacts)

Companion to the [SDK spec](./03-sdk-spec.md), the [MCP spec](./02-mcp-server-layer-spec.md),
and the [`rustz80` spec](./07-rust-z80-compiler-spec.md). Spec 03 asked "an SDK so
people build games without hand-writing Z80," answered it with the fidelity dial,
and — written before `rustz80` existed — recommended standing on z88dk/C for the
front end. That recommendation is now **superseded**: the dial is closed *in Rust*
(one `impl Game` compiles host *and* pure; `samples/bounce.rs`, `samples/move.rs`),
so the authoring language is Rust, not C. This spec is the synthesis on top of that
fact. It pins the load-bearing decisions so the kit (L1), the asset pipeline (L0),
the env layer (E), and the MCP authoring studio are all built against one precise
line instead of drifting apart.

It leads, as 03 does, with the decision everything else flows from.

---

## 0. The one invariant: one typed source, three artifacts

> **The game's Rust struct is the single source of truth. Three artifacts fall
> out of it with no retrofit: a host build, a pure `.tap`, and an agent
> environment. Anything that puts a second source of truth between the struct and
> any artifact is a defect, not a feature.**

This is the whole product. The sprites, maps, splash screens, and music are table
stakes — z88dk and MPAGD had them 20 years ago. The distinctive claim, the lane no
other retro kit occupies, is that **the same authored source is simultaneously a
real Spectrum game, a hardware-bootable tape, and a deterministic, instrumented RL
environment.** Every decision below exists to protect that invariant.

```
                       impl Game for CrystalCavern   ← the one source
                                   │
            ┌──────────────────────┼──────────────────────┐
            ▼                      ▼                      ▼
      host-composite          rustz80 →             agent env
      (chuk-speccy-sdk)            bootable .tap          (chuk-speccy-env)
      full host power,        runs on real           obs / reward / done
      instant iterate         48K hardware           from the *types*
```

Two corollaries that the rest of the spec makes precise:

1. **The dial discipline (§1–§2).** Because two of the three artifacts go through
   `rustz80`, the framework the author calls must live inside the `rustz80`
   subset, and assets must bake to `const`. The compiler is the arbiter of what
   crosses.
2. **The env falls out of the types (§3).** Because the third artifact is an
   environment, the observation/reward/done surface is *derived from the typed
   struct*, not hand-written against RAM addresses. The bridge that makes this
   true for the pure tape is the compiler-emitted **symbol map (§2)**.

### 0.1 The three contracts (the architecture in one triangle)

The invariant decomposes into three contracts, which is the cleanest way to reason
about — and divide — the work:

```
  AUTHOR contract        one subset-clean Rust `impl Game` is the only source.
        │
        ▼
  COMPILER contract      rustz80 emits the .tap AND the .sym.toml from one typed
        │                layout — the hardware artifact and the env bridge, together.
        ▼
  ENVIRONMENT contract   chuk-speccy-env reads host state directly (host build) or tape
                         RAM via .sym.toml (pure build), then evaluates the *same*
                         typed observe/reward/done.

         ── all three stand on ──
  DETERMINISM contract   bit-exact serialize_full reset (already built, spec 02).
                         Without it "reproducible episode" has nothing underneath it;
                         the environment contract is its dependent, not its peer.
```

The symbol map is the join between the compiler and environment contracts, and it is
the jewel: it is what makes "the env falls out of the types" literally true even on
hardware where the types don't exist at runtime.

---

## 1. The dial discipline — what compiles where

The dial is not a slider the author nudges; it's a hard boundary the compiler
enforces. `rustz80` accepts a subset of real Rust and rejects the rest with a
clear error (the "host-only" signal). The current subset (spec 07):

| Crosses to pure `.tap` | Host-only (does **not** compile pure) |
|---|---|
| `u8` / `u16`, `as` casts | `f32` / floating point |
| fixed arrays `[T; N]` | `Vec`, `String`, `format!`, any heap |
| `struct` (constant field offsets), `enum` + `match` | generics (Stage 2, pending) |
| `fn`, `impl T { fn m(&mut self) }`, methods | recursion (Stage 4, pending) |
| `poke`/`peek`, bitwise, `if`/`while` | trait objects beyond recognised `impl Game` |
| declared host **traps** (compile-error unless whitelisted) | ambient `rand`, wall-clock, host I/O |

**The rule that keeps the dial closed:**

> The framework library (L1) is written entirely in the subset. Assets bake to
> `const`. The *only* host-only constructs in an authored game are explicitly
> declared escape-hatch traps.

This is a real discipline with real teeth, and the repo already violates it in its
showcase: **`speccy-sdk/src/demo.rs` Snake uses `Vec` and `format!`** — it is
host-only and cannot take the pure path. The dial-closed samples (`bounce`, `move`)
are minimal. So the first concrete gap is not "add sprites"; it is **provide the
subset-clean primitives that let a real game (Snake-with-collections) compile both
ways.** Three small types, all subset Rust, shipped in `chuk-speccy-sdk`:

```rust
Entities<T, const N: usize>   // fixed-capacity vec — replaces Vec<T>
Fx8_8                         // 8.8 fixed-point — replaces f32 for velocity/physics
Rng                           // state-seeded xorshift — the determinism contract
```

`Rng` is non-optional on purpose: it is the determinism contract made into a type.
Seed it from game state, never from the clock; then every game is a valid env
(bit-exact `serialize_full` reset is already the gate — spec 02). The demo already
hand-rolls this xorshift; the kit promotes it to a primitive so authors can't get
it wrong.

**Escape hatches are the dial, expressed.** Reaching for host physics, an LLM, or
asset decode is a declared trap (`ED FE`, id in `A`; spec 03 §4). It works in
host-composite and is a clean NOP / compile-rejection on the pure side. The subset
boundary *is* the 1982-budget / 2026-capability line: if it compiles pure, it would
run on a real Spectrum; if it needs a trap, it won't, and the author knows at build
time.

### 1.1 Two orthogonal axes of "clean" — don't collapse them

A subtlety that's easy to miss: **subset-clean and deterministic are different
properties, and only one of them is about the tape.**

| Property | Means | Verified by | Needed for |
|---|---|---|---|
| **subset-clean** | no host-only construct | `check-pure` (subset lint) | the `.tap` artifact |
| **deterministic** | reproducible from `(state, seed)` | `check-deterministic` | the env artifact |

A game can be perfectly subset-clean and still nondeterministic — it reads ambient
`rand` or a wall-clock — and it will run fine host-side, compile to `.tap` fine, and
be a **broken environment**, because `reset(seed)` won't reproduce the episode.
`check-pure` does not catch this. So the kit needs *both* lints, and the second is
cheap to enforce precisely because `Rng` is the **only** sanctioned randomness
source: the determinism check is "did you touch `SystemTime`, ambient `rand`, or read
a `static mut`?" That is the real reason `Rng` is a non-optional primitive rather
than a convenience — it makes determinism checkable by exclusion.

### 1.2 The author loop — fast host iteration, with the dial always visible

Pure-first discipline must not tax fun authoring. The loop keeps host iteration
instant while continuously reporting both cleanliness axes:

```
speccy run               fast host-composite loop (Vec/f32/println all fine here)
speccy check-pure        what blocks the .tap, with ergonomic fixes (below)
speccy check-determinism what blocks the env (non-Rng randomness, clock, statics)
speccy build-tap         succeeds only when subset-clean (the dial, enforced)
speccy env run           runs the same typed source as an episode
```

Host mode never complains about `Vec` — that's the point of the dial; the rejection
lives at `check-pure`/`build-tap`, not at `run`.

**The subset is only acceptable if its rejections are ergonomic.** `rustz80` uses
rustc as the type checker, so a host-only type like `Vec` is *valid Rust* — it is
`rustz80`'s lowering pass that must reject it, which means the lowering pass is also
where the helpful message is emitted, mapping each known host-only construct to its
subset replacement:

```
error: `Vec<T>` is host-only; it has no pure 48K representation.
  --> game.rs:14:18
   |
14 |     enemies: Vec<Enemy>,
   |              ^^^^^^^^^^
   = help: use the fixed-capacity `Entities<T, N>` instead:
             enemies: Entities<Enemy, 8>,
   = note: this compiles under both rustc (host) and rustz80 (pure).
```

A bare "unsupported construct" makes the subset feel like a cage; a mapped fix makes
it feel like a guide rail. Ship the replacement table (`Vec`→`Entities`, `f32`→`Fx8_8`,
`format!`→fixed text helpers, `rand`→`Rng`) as part of L1, not as an afterthought.

---

## 2. The symbol map — the bridge that carries types across the dial

This is the novel mechanism, and it only exists because you own the compiler.

The problem it solves: "the env falls out of the types" is *literally* true only in
host-composite mode, where the env reads the live Rust struct. On the pure `.tap`,
**the Rust state does not exist at runtime — only Z80 RAM does.** The reflex (and
the three source docs all reach for it) is to hand-write a `memory_map.toml` of
score/lives/player-x addresses. That reintroduces exactly the RAM archaeology the
invariant is meant to abolish, and it creates a second place where field names live,
drifting against the struct.

The resolution uses a fact `rustz80` already has: **Stage 1c assigns every struct
field a constant offset.** The compiler therefore *knows* where `self.player.x` and
`self.score` land in RAM. So have it **emit that knowledge** as a sidecar artifact
next to the `.tap`:

```toml
# CrystalCavern.sym.toml  — emitted by rustz80, never hand-written
[state]
base = 0x8200          # struct instance base in RAM
size = 46

[fields]
"player.x"  = { addr = 0x8200, width = 1, ty = "u8" }
"player.y"  = { addr = 0x8201, width = 1, ty = "u8" }
"score"     = { addr = 0x8210, width = 2, ty = "u16" }
"lives"     = { addr = 0x8212, width = 1, ty = "u8" }
"room_id"   = { addr = 0x8213, width = 1, ty = "u8" }
# … every remaining field, to the full `size`. Not a curated subset.
```

**Rule: emit the full struct layout, always — never a curated subset.** This is the
load-bearing detail. The pure-side env doesn't read "a few interesting fields"; it
**reconstructs a whole `Self`** so it can run `reward(&self, prev: &Self)` over it. If
the map omits any field the env methods transitively touch, the reconstruction is
incomplete and `reward` silently reads garbage — the worst kind of bug, because it
only shows up tape-side and looks like a bad reward signal, not a missing field. The
tempting optimisation is reachability analysis (emit only fields the env reads); on a
48K machine the state struct is tens of bytes, so trimming it buys nothing and risks
exactly this class of silent divergence. Emit everything; let the env reconstruct any
`Self` it needs with no per-method cleverness.

Now the *same* typed annotation serves both modes, with two extraction paths and
zero archaeology:

- **Host build** → the env calls `game.reward(&prev)` on the live struct directly.
- **Pure `.tap`** → the env harness reads **all** struct fields from tape RAM via
  the symbol map, materialises a `Self`-shaped view, and runs **the same**
  host-compiled `reward`/`done`/`observe` over it.

So reward/done/observe stay ordinary, rustc-checked Rust functions. They are never
compiled into the tape (there is no reward on real hardware — it is always computed
env-side), and they run *identically* in both modes. One source of truth (the typed
state); the compiler is the bridge.

**This supersedes spec 03 §7's `env_report(score, done)` trap** as the primary
path. That mechanism required the game to actively push reward through a trap every
frame — it couples the game to the env and only surfaces what the author remembered
to push. The symbol-map path is *passive*: the env reads whatever typed fields it
wants and computes reward env-side from them, keeping reward logic out of the game
entirely. Keep the `env_report` trap as an optional fast-path for expensive-to-derive
signals; default to the symbol map.

**Two env paths, kept explicitly separate:**

| Path | Source of addresses | For |
|---|---|---|
| **Authored-typed** | compiler-emitted `.sym.toml` | games you wrote in the kit |
| **Found-game** | hand-written `memory_map.toml` | commercial games you didn't author (Manic Miner, etc.) |

These must not be blurred into one schema. The first is the kit's superpower and has
no hand-written addresses anywhere. The second is honest archaeology for wrapping
chaotic commercial titles, and it's fine for that job and only that job.

---

## 3. The env surface — the widened `Game` trait

The env surface is three methods on the game, with defaults so every existing game
still compiles:

```rust
pub trait Game {
    fn update(&mut self, input: &Input, frame: &mut Frame);

    // --- env surface (default impls; override to instrument) ---
    fn observe(&self) -> Obs { Obs::Screen }        // pixels, or typed features
    fn reward(&self, prev: &Self) -> i16 { 0 }      // env-side; never in the tape
    fn done(&self) -> bool { false }

    fn reset(seed: u64) -> Self where Self: Sized;  // replaces ad-hoc `new()`
}
```

Rules:

- **No string DSL for reward.** A typed `fn reward(&self, prev: &Self) -> i16` is
  rustc-checked, refactor-safe, and carried across the dial by the symbol map. A
  string expression (`"score_delta + exit_bonus"`) throws away the type system and
  re-creates the field-name drift the symbol map just eliminated. Rejected.
- **Env methods must be pure functions of `(self, prev)`.** This is the soundness
  condition for the symbol-map bridge. `reward`/`done`/`observe` run *host-side* over
  a `Self` reconstructed from RAM (§2) — so they can only see struct fields. An author
  who reads a host global, a `static mut`, or the clock inside `reward` gets a game
  that scores correctly in the host build and **diverges on the tape-side env**, where
  that external state doesn't exist in the reconstruction. State it as a rule and lint
  it: env methods touch only `self` and `prev`. (The determinism check in §1.1 catches
  most of this for free, since the forbidden reads are the same set.)
- **`reset(seed)` is the episode boundary.** It seeds `Rng` and is the
  `deserialize_full` target. Combined with the `Scene` stack (§4), an env can reset
  straight into the `Play` scene and skip the splash.
- **`observe` returns either the framebuffer or typed features.** Pixels for
  generality and vision agents; typed features (read directly host-side, via symbol
  map tape-side) for cheap clean signals. Per game, per task.

The moment someone writes a game, they have written its environment. No
`memory_map.md`, no retrofit. That is the through-line of the whole project.

---

## 4. The author API — the L1 framework (all subset-clean)

Keep `Game::update(&Input, &mut Frame)` as the **single** author ABI seam — the one
thing proven to compile both ways. Do **not** add a parallel `SpeccyGame` trait; the
higher-level systems are *libraries you call inside* `update`, not a second trait
hierarchy. (Two author traits guarantee a fork where "real" games quietly migrate to
the host-only one — the exact failure mode `demo.rs` already shows.)

The systems, every one written in the subset:

| System | Notes |
|---|---|
| `Sprite` / `SpriteSheet` | **two models, clash named in the API** (see below) |
| `TileMap` / `Camera` | TMX → `const` tilemap; `collision_at`, `tile_at` |
| `Scene` stack | splash → title → menu → play → gameover; free episode boundaries |
| `Hud` | score/lives over the ROM font |
| `SoundBank` | const data, two players (see below) |
| `Animation` | frame table indexing a `SpriteSheet` |

**Sprites — be honest about colour clash; offer both models.** This is *the*
Spectrum aesthetic decision; the API names it rather than hiding it (spec 03 §6 is
emphatic on this and it's right):

- **Masked monochrome** — moves per-pixel, stays ink-only within a cell, no clash
  (Sabreman style). Pre-shifted frames for smooth motion. `SimpleSprite` is the
  beginner-safe wrapper over this.
- **Attribute / char sprite** — grid-locked, full colour by construction, no clash
  (Knight Lore style).

Later, a **dirty-cell engine** (SP1-style: 8×8 dirty cells, tiles + masks +
occlusion + planes; z88dk's SP1 is the reference model) is the performance backend
for serious pure 48K games. Treat it as the **dial canary**: it's where the subset
bites hardest (no recursion, fixed arrays, no generics). If the dirty-cell engine
compiles pure, the subset can carry real games; if it can't, you've found the dial's
true ceiling — and you want that discovered early, not at the end of a vertical
slice.

**Sound — same const data, two players (the one place the dial can't be "same
code").** Sound is *timing*, and host-composite collapses timing: `update()` runs
instantly host-side, so a real beeper busy-loop wouldn't sound. The honest design:

- Sound is **const data** — a tiny tracker-ish format, designed **AY-ready** so it
  survives the eventual 128K/AY work (spec roadmap "accuracy tail").
- **Host-composite player** = a host trap that schedules **actual port-`0xFE` bit-4
  edges across the frame's T-state budget**, producing the *real beeper sound* —
  identical to hardware. Not "nice generated audio." If the host backend
  synthesises a different sound, the MP4 recordings and audio-obs diverge from real
  silicon and the "it really *is* a Spectrum" claim — the thing that justifies the
  project over a window-with-a-retro-filter — dies for audio.
- **Pure player** = a compiled Z80 beeper routine.

Same data, two players, both emitting the same edges. The dial stays closed at the
data level, which is the level that matters.

Author-facing, this is just:

```rust
sfx.play(Sfx::Coin);
music.play(Music::Title);
```

---

## 5. The asset pipeline (L0) — bake to `const`, negotiate with the ULA

`chuk-speccy-assets`, a host-side CLI / `build.rs` step. **Bake to `const`; never decode
at runtime** — runtime decode is the tempting shortcut that splits the dial (a
host-side decoder doesn't exist on the pure tape). The same `pub const HERO: Sprite`
works in both builds.

```
PNG / Aseprite   → sprite frames + masks + animation table   → const Rust
Tiled TMX/JSON   → tilemap + collision layer                 → const Rust
PNG (256×192)    → loading screen (.scr, optional ZX0)        → const / SCR
Font PNG         → 8×8 charset                                → const Rust
tracker export   → music/SFX blob (AY-ready)                  → const Rust
```

**The Spectrum-specific value is the clash report** — the one piece of domain
knowledge a generic engine can't paper over, and it's pure host-side tooling with
zero dial tension. Build it early; it's cheap and it's a demo magnet:

```
$ speccy asset check player.png
player.png
  OK   : 16×16 mono sprite, 4 frames
  WARN : 3 cells exceed 2 colours  →  (3,1) (4,1) (4,2)
  FIX  : forced BrightYellow ink / Black paper
  OUT  : player.spr.rs, player.mask.rs   (192 B bitmap + 192 B mask)
```

A normal engine imports assets and prays. A *Spectrum game kit* negotiates with
colour clash, 8×8 attributes, 48K RAM, and the frame budget, and says so in
machine-readable form — which is also exactly what an authoring agent needs (agents
are bad at hidden platform constraints unless the tool surfaces them).

---

## 6. The manifest boundary — declarative structure only

There is a manifest, and getting its scope right is load-bearing, because the
authoring plane (§7) is a machine for generating games *at scale* — point it at a
fuzzy boundary and it mass-produces dial-breaking games efficiently.

> **The manifest owns declarative structure. Typed Rust owns all behaviour. The
> manifest scaffolds a project once; the game does not round-trip through it.**

| Manifest owns (declarative) | Types own (behaviour) |
|---|---|
| which sprite PNG / TMX map | `update` logic |
| splash image + timing, title music | `reward` / `done` / `observe` |
| control mapping (O/P/Q/Space) | physics, AI, collision response |
| template choice, target (48k/128k) | everything that runs per frame |

```toml
[game]    name = "Crystal Cavern"   template = "platformer"   target = "48k"
[player]  sprite = "player"   start = [2, 18]   lives = 3
[controls] left = "O"  right = "P"  jump = "Q"  fire = "SPACE"
[[sprites]] id = "player"  source = "assets/player.aseprite"  size = [16, 16]
[[maps]]    id = "level_1" source = "assets/level_1.tmx"      collision = "solid"
[splash]    image = "assets/loading.png"  duration_frames = 150  fade = "attr_wipe"
```

**No `expand_manifest_to_rust` as a logic representation.** The instant game logic
round-trips through TOML, the generated Rust becomes a file nobody edits and the
type-driven env dies. And there is no need for a second structured contract for game
*structure*: **`rustz80` already parses Rust with `syn`** — the typed struct *is*
the machine-readable description, and the compiler already introspects it (that's
how field offsets, hence the symbol map, exist). Authoring tools that change
structure manipulate the *one* source at the AST level (`add_field`, `add_actor`
emitting a struct, `wire_reward` emitting a method). The manifest is reserved for
the non-code glue above. One representation of structure, not two.

---

## 7. The two planes — authoring and runtime, meeting at the core

Expose the kit over MCP as a **domain-specific game studio**, not as filesystem
tools for an agent. Two planes that stay separate and meet only at the deterministic
emulator:

```
                ┌─────────────────────┐
                │  AI coding assistant │
                └──────────┬──────────┘
                           │ MCP
        ┌──────────────────▼──────────────────┐
        │  chuk-mcp-speccy-kit  (AUTHORING)    │   build & emit:
        │  scaffold · assets · compile · audit │   .tap + env wrapper + .sym.toml
        └──────────────────┬──────────────────┘
                           │
     ┌─────────────────────┼─────────────────────┐
     ▼                     ▼                     ▼
 chuk-speccy-sdk           chuk-speccy-assets          rustz80
 (runtime API)        (asset compile)        (.rs → .tap + .sym.toml)
     └─────────────────────┼─────────────────────┘
                           ▼
                   ┌───────────────┐
                   │ spectrum core │  deterministic, bit-exact reset
                   └───────┬───────┘
                           │ MCP
                ┌──────────▼──────────┐
                │  chuk-mcp-spectrum  │   (RUNTIME)
                │  play · observe ·   │   run episodes, step, rewind, record
                │  rewind · benchmark │
                └─────────────────────┘
```

**Tools are intent-level, in your domain model** — `add_actor`, `add_room`,
`build_assets`, `compile_tap`, `fix_colour_clash`, `wire_reward` — never
`write_file` / `run_command` / `poke_memory`. Intent-level tools are safer,
repeatable, and keep the agent operating on the game, not the filesystem.

**Don't smear the planes.** `run_agent_episode`, `benchmark_agents`,
`compare_agents` are *runtime* operations — they belong to `chuk-mcp-spectrum`. If
the authoring server runs episodes, the two planes grow into each other and
duplicate the runtime's job. The authoring rule: **authoring builds and emits the
artifact + env wrapper + symbol map; runtime runs it.** `agentability_report` (§9)
may live on the authoring side, but it *invokes* the runtime plane for its rollouts
rather than reimplementing stepping.

**Constraints as resources** (read-only, zero dial tension — build now):

```
speccy://constraints/48k          speccy://constraints/colour-clash
speccy://constraints/screen       speccy://docs/screen-model
speccy://templates/maze/schema    speccy://project/current/asset-report
```

The agent fetches exact constraints instead of half-remembering Spectrum details;
the screen model (256×192, 32×24 cells, ink/paper/bright/flash) becomes a
first-class resource that tools validate against.

---

## 8. The subset *is* the sandbox — the real safety boundary

MCP's spec warns that tools are arbitrary-code paths needing user control. The weak
mitigation is tool granularity ("don't expose `write_file`"); it doesn't help once
the agent compiles and *runs* Rust it authored — which is the real risk surface and
exists no matter how intent-level the tools are.

The strong safety story is one only you can tell, because you own the compiler:

> **"Compiles pure" is a security property.** A pure `.tap` cannot do host I/O by
> construction — no `Vec`, no `f32`, no syscalls except explicitly declared traps.
> It can only poke its own 48K. The escape hatches (host physics, LLM) are exactly
> the traps that *don't* compile pure, so they are enumerable and gateable at the
> ABI seam.

The safety model, therefore:

1. **Project-root jail** — all writes confined to the project directory.
2. **Compile-pure-unless-whitelisted** — agent-authored games must pass `rustz80`
   (pure) unless a specific escape-hatch trap is explicitly whitelisted for that
   project. A game that needs an un-whitelisted host trap fails to build.
3. Host-composite runs of agent-authored code stay inside the existing autonomy
   plane's per-session jail (spec 06).

That is a far harder boundary than tool taxonomy, and it falls out of machinery you
already have.

---

## 9. The agentability report — the actual research artifact

Every authored game is automatically a benchmark; the report is what makes that
concrete (and it's the thing nobody else has). It is static analysis over the typed
reward + symbol map, plus short rollouts invoked on the runtime plane:

```
Agentability — Crystal Cavern

Observation : screen ✓   probes: player_x, player_y, score, lives, room_id
Actions     : 5 buttons          Deterministic reset: ✓   Frame-skip: 4 (rec.)
Reward      : dense score_delta · sparse exit_bonus · penalties death, timeout
Baselines   : no-op 0%   random 0.7%   scripted 84%   human-playable ✓

⚠ reward-hack : score farmable by re-collecting respawned coin at tile (12,8)
⚠ curriculum  : level_2 needs pixel-perfect jump — poor for early curriculum
```

The **reward-hackability detector** is the headline. It reads the typed `reward`
function and the symbol map, runs short rollouts, and flags farmable loops and
degenerate optima. That is a genuine research output, not a demo — and it exists
only because reward is *typed and analysable* (§3), which is the dividend of
refusing the string DSL.

---

## 10. Sequencing — prove the seam before building the studio

Each layer in this design (kit → studio → task factory) widens the surface over
which the dial can silently break, and the leverage cuts both ways: an authoring
server pointed at an unproven dial is a machine for mass-producing host-only games
that *claim* to be Spectrum games. So the ordering is not cosmetic — do not multiply
a primitive you haven't watched close.

```
1. PROVE THE SEAM (an afternoon, falsifies the whole thesis)
   Snake, one typed source → host build + pure .tap + env,
   with rustz80 emitting Snake.sym.toml.

   Snake's critical path is Entities + Rng + reset + symbol map — NOT Fx8_8.
   Fixed-point is proven separately by the velocity samples (bounce/move); don't
   bundle it into this milestone or you widen the proof for no reason.

   Brutally small order:
     a. add Entities<T, N>          (replaces Vec in a subset-clean Snake)
     b. add Rng                     (state-seeded; the determinism primitive)
     c. add reset(seed) to Game
     d. rewrite Snake off Vec/format! → subset-clean
     e. rustz80 emits a minimal full-layout .sym.toml
     f. one env test: read `score` from tape RAM via .sym.toml  ← the riskiest bit
     g. add reward(prev) + done()
     h. run Snake three ways: host game · pure .tap · env episode

   Success test: `score` round-trips typed-field → emitted addr → env reads it off
   the running tape. If yes, the architecture is real. Step (e) is the only piece
   that doesn't exist yet — prove it before anything above it is built.

2. THE KIT (L1 + L0), dial-honest
   subset-clean Sprite/TileMap/Scene/Hud/SoundBank + the clash-report CLI +
   bake-to-const assets. Dirty-cell engine as the dial canary. Fx8_8 lands here.

3. CHEAP, DIAL-NEUTRAL WINS (parallel, build as plain CLIs first)
   constraints-as-resources; `speccy env audit` (the reward-hack detector).

4. THE VERTICAL SLICE — `speccy new maze --template agent_maze`
   splash · tilemap · player/enemy sprite · key/exit · beeper SFX · HUD ·
   RNG · typed probes · reward · env wrapper · random+scripted agents ·
   host run · .tap · MP4. Built on a seam already watched close; any
   host-only cell tagged as an explicit escape hatch, not smuggled in.

5. THE AUTHORING STUDIO — chuk-mcp-speccy-kit  (LAST)
   intent-level tools over the proven kit; compile-pure-unless-whitelisted
   safety; authoring emits, runtime runs.

   D. frontends (WASM) · F. reach (demo/release) — parallel, any time
   Later. 128K/AY — unblocks real AY music; below E in priority
```

The cognitive-axes template set is what turns this from a game kit into a **task
factory**: `gridworld` (planning), `runner` (reaction), `maze` (exploration +
memory), `rhythm` (the SOMA **B1⊥B2** demonstrator: a fast binary-fatal axis
orthogonal to a slow pacing axis), `shooter` (tracking), `puzzle` (symbolic state).
State the identity strongly: the product is **a deterministic task factory that
happens to express tasks as real Spectrum games** — the games are the substrate, the
separable difficulty axes are the product.

---

## 11. What not to build

- **A second author trait** (`SpeccyGame` alongside `Game`) — one ABI seam, or you
  get a host-only fork.
- **A reward DSL / string expressions** — reward is typed Rust, carried by the
  symbol map.
- **`expand_manifest_to_rust` as a logic representation** — the manifest is
  declarative glue, not a round-trip format; logic lives in types.
- **A hand-written `memory_map.toml` for authored games** — the compiler emits the
  symbol map; hand maps survive only for found commercial games.
- **A curated symbol map** — emit the *full* struct layout; a trimmed map invites
  silent tape-side reconstruction bugs (§2).
- **Env methods that read host state** — `reward`/`done`/`observe` must be pure
  functions of `(self, prev)`, or they diverge between the host and tape envs (§3).
- **A single `check-pure` treated as the env guarantee** — subset-clean and
  deterministic are different axes; you need both lints (§1.1).
- **Runtime tools on the authoring plane** — episodes/benchmarks belong to
  `chuk-mcp-spectrum`.
- **"Nice generated audio" in host-composite** — schedule real port-`0xFE` edges, or
  the recordings lie about the machine.
- **The MCP studio before the seam is proven** — it's the last mile, not the
  foundation.

---

## Crate / server layout

```
chuk-speccy-sdk            runtime API: Game, Scene, Frame, Input, Audio, Rng,
                      Entities, Fx8_8  (subset-clean; the one author seam)
chuk-speccy-assets         PNG/Aseprite/Tiled/tracker → const Rust; clash report
chuk-speccy-game           reusable systems: maps, sprites, collisions, HUD, splash, menu
chuk-speccy-env            env wrappers: Obs/reward/done, symbol-map reader, Gym surface
rustz80               .rs → .tap  +  emits .sym.toml from struct layout   ← the bridge
chuk-mcp-speccy-kit   AUTHORING plane: scaffold · assets · compile · audit (LAST)
chuk-mcp-spectrum     RUNTIME plane: play · observe · rewind · record · benchmark
```

The honest summary: **the manifest is for assets, the types are the game, the
compiler is the bridge, and the MCP server is the last mile — not the foundation.**
Everything here is a control surface over one seam — one typed source producing host
build, pure `.tap`, and env, with the compiler emitting the symbol map. Watch that
seam close on Snake first; build the rest on top of a join you trust.
