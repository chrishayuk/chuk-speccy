# 09 — What the SDK needs, learned from classic games

> **The better the SDK, the easier to make games.** This doc works backwards from a
> spread of real ZX Spectrum titles to the *capabilities* the authoring plane (spec
> 08, roadmap **E**) needs, so the SDK backlog is driven by evidence — "what do actual
> games demand" — not by what's convenient to build.

Three rules keep this honest, and every capability below is judged against them:

1. **Leverage** — prefer capabilities that unlock the *most games per unit of effort*.
   A sprite system buys you a whole genre; a pseudo-3D road buys you two titles.
2. **The dial** (spec 08 §1) — **the kit builds real, bootable games: the pure `.tap`
   is the product.** Host-composite is only a fast-iteration mode, never the
   destination. So each capability is judged by whether its *logic* compiles **pure
   today** (inside the current subset envelope), and if not, by exactly which
   `cell80`/`rustz80` frontend feature it waits on — a compiler requirement to push, not
   a reason to settle for host.
3. **The agentability dividend** (spec 08 §3) — typed game state becomes an RL env "for
   free" via the symbol map. A richer SDK isn't just more games; it's more *benchmark
   tasks*. Each capability notes what it gives the env.

Current SDK surface this builds on (as of 2026-06): `Game` (`update`/`observe`/
`reward`/`done`/`reset`), `Frame` (`pixel`/`tile`/`fill_cell`/`clear_cell`/`text`/
`text_u16`), `Tile`, `Entities<T,N>`, `Rng` (`below_mask` subset-clean), `Cell`,
`Controls`/`Input` (`held`/`pressed`), `chuk-speccy-assets` (`convert`→`.scr`, **`bake`
→`const Tile`**, clash report), `speccy_sdk::render` (host game → GIF), `chuk-speccy-env`
(symbol-map reader + Gym surface).

---

## 1. The evidence — games → technical demands

A genre-spanning sample. For each: the *hard* demands it places on an engine (the
things that are painful without SDK support).

| Game | Genre | The demands it makes |
|---|---|---|
| **Manic Miner / Jet Set Willy** | single-screen platformer | gravity + parabolic jump (fixed arc), tile flags (solid / crumble / conveyor / hazard), patrolling guardians on fixed paths, a per-cavern timer (air), many screens as level data |
| **Booty** | flip-screen maze-platformer | a graph of ~20 flip-screens, **keyed doors** (coloured locks), patrol enemies, collectibles + score, trapdoors |
| **Dizzy / Treasure Island Dizzy** | flip-screen arcade adventure | large **room graph** (off-screen world), **somersault jump arc** (no air control), an **inventory** with pick / drop / **use** object puzzles, hazard tiles (water = death), per-room state, NPCs |
| **Mikie** | flip-screen action-comedy | flip-screen rooms, **throwable projectiles**, "bump" interactions, dodge/patrol AI, collectible hearts, scene-to-scene flow |
| **Renegade** | scrolling beat-'em-up | **multiple simultaneous enemies** with approach/attack AI, a **combat move set** decoded from input combos, health bars + round timer, multi-stage levels, horizontal scroll |
| **Gauntlet** | top-down dungeon crawler | **smooth multi-directional scrolling** over a large tilemap, **enemy hordes from generators** with chase-the-player pathing, an energy economy (constant drain + pickups), tile flags (wall / door / exit / food / treasure / key), real-time with many actors |
| **Chaos** | turn-based tactical | a **turn / phase loop**, a grid board, **creature & spell data tables**, a **spell system** with cast-success probability + illusion/real, **line-of-sight** ranged combat, **RNG resolution**, **AI opponents**, **hotseat** multi-wizard input |
| **Daley Thompson's Decathlon** | multi-event sports | **joystick-waggle** input (high-frequency alternation → speed), tight **timing** windows (jump/throw angle + power meters), fast multi-frame **run-cycle animation**, **10 distinct mini-game scenes** behind one flow, records/score HUD |
| **Chase HQ / OutRun** | pseudo-3D racer | a **pseudo-3D road** (segmented perspective, curvature + hills), **sprite scaling** (cars/scenery by distance), high-speed update, checkpoints/branching, multi-phase, **AY music** |
| **Knight Lore** | isometric ("Filmation") | **isometric room rendering** with depth-sorted masked sprites, 3D collision in a 2.5D projection, room graph |
| **Cybernoid** (in `testroms/`) | flick-screen shoot-'em-up | inertia-based ship movement, many bullets/turrets, flick-screen rooms, lots of on-screen sprites |

Rolled up, the recurring demands are: **sprites** (pixel-positioned, masked, animated,
many at once), **tile maps + a screen/room model**, **movement/collision/physics**,
**actors + AI**, **audio (SFX now, AY music later)**, **scene/flow + game state +
inventory**, **richer input**, **scrolling**, and at the ambitious end **pseudo-3D /
isometric**. The asset pipeline cuts across all of them (art/maps/music → `const`).

---

## 2. The capabilities (the actual backlog)

Each: what it is · which games demand it · current state · **dial** placement ·
**env** dividend. Tiered by leverage.

### Tier 1 — the arcade core (unlocks the biggest cluster: Manic Miner · Booty · Dizzy · Mikie)

**C1. Sprite system** — pixel-positioned (not cell-aligned), **masked** (transparent
background, OR-draw + restore), multi-frame **animation**, multi-cell sprites,
z-order. *Games:* every action title. *Now:* only cell-aligned `Tile`/`fill_cell` (no
sub-cell position, no mask). *Dial:* a **host** `Sprite`/`SpriteBank` lands now over
`Frame`; the **pure** form needs `&CONST → addr` const-data (relocate bitmap bytes) +
`[u8; N]` — `cell80`-gated. *Env:* sprite x/y/frame are typed fields → free positional
observations and shaped rewards (distance-to-target).

**C2. Tile maps + screen/room model** — a `TileMap` (indices into a tile bank) with
**per-tile flags** (solid / hazard / exit / pickup / climb), and a **room/screen graph**
for flip-screen worlds (which room is N/S/E/W of this one). *Games:* Manic Miner
(caverns), Booty (20 rooms), Dizzy (huge map), Mikie. *Now:* none — rooms are ad-hoc.
*Dial:* host `TileMap` now; pure needs `[u8; N]` arrays + const-data — `cell80`-gated.
*Env:* room id + tile grid are the cleanest discrete observation there is (exploration/
memory tasks fall straight out — the `maze` cognitive axis).

**C3. Movement, collision & platform physics** — pixel movement, **AABB** + **tile-flag
collision** (walk into walls, stand on floors), **gravity + jump arcs** via `Fx8_8`
fixed-point. *Games:* every platformer; Dizzy's somersault is the canonical arc. *Now:*
cell movement done by hand in each game. *Dial:* host now; pure needs persisted `i16`/
fixed-point fields + the collision helpers compiled subset-clean — partly `cell80`-gated.
*Env:* deterministic physics is the substrate contract — rollouts stay reproducible.

**C4. Beeper SFX engine** — `const`-data sound effects played as **real port-`0xFE`
edges** (never "nice generated audio" — recordings must not lie about the machine). A
small effect bank + a one-channel player. *Games:* all (jump blip, pickup chime, death).
*Now:* the core beeper works; there's no authoring-side SFX layer. *Dial:* host now
(emit real edges); pure-tape playback needs const-data routing — `cell80`-gated. *Env:*
audio events are reward-shaping signals (and observation channels later).

**C5. Scene / flow state machine** — title → play → game-over → next, plus per-event
scenes (Daley's 10 events). A `Scene` trait or a flow enum the head drives, replacing
"one `Game::update` does everything." *Games:* Daley (events), all (attract/title/HUD/
death). *Now:* a single `Game`. *Dial:* host now; pure form is a state enum field
(subset-clean once persisted-field gaps close). *Env:* scene id bounds episodes cleanly
(the `truncated` vs `done` distinction the env wants).

### Tier 2 — depth for action & strategy (Renegade · Gauntlet · Chaos · Daley)

**C6. Actor / AI system** — many concurrent entities with behaviors: **patrol** (fixed
path), **chase** (greedy toward player), **spawn waves / generators**, and a **turn-based
unit model** (stats + actions) for strategy. *Games:* Gauntlet hordes + generators,
Renegade gangs, Chaos creatures. *Now:* `Entities<T,N>` is a pool with no behavior.
*Dial:* host now; pure needs `[u8;N]`/struct-of-arrays + the behavior fns subset-clean.
*Env:* enemy positions/counts are observations; "survive the horde" is a dense reward —
the `shooter` axis.

**C7. Richer input** — **combo decoding** (punch+direction → moves, à la Renegade),
**waggle** detection (Daley), **two-player / hotseat** (Gauntlet co-op, Chaos wizards),
on top of today's `held`/`pressed`. *Games:* Renegade, Daley, Chaos, Gauntlet. *Now:*
single-player edges only. *Dial:* host now; pure form is straightforward (it's logic).
*Env:* a larger/structured **action space** — exactly what multi-button agents need.

**C8. Game state, inventory & progression** — **items/inventory** (carry, drop, use),
**keyed doors/locks**, an **economy** (energy/score/lives), level progression, optional
**save**. *Games:* Dizzy inventory + object puzzles, Booty keys, Gauntlet potions/keys.
*Now:* ad-hoc struct fields. *Dial:* host now; pure form gated on richer persisted state
(`u32`/arrays). *Env:* inventory + flags are typed state → the env can pose
*subgoal* rewards; great for the `puzzle` axis. **This is the reward-hackability
detector's richest input** (spec 08 §9): collectible loops, respawn farms.

**C9. HUD / fonts / menus** — custom fonts, status panels (score/lives/energy bars),
title screens, **selectable menus** (key-redefine, options, Chaos's spell-pick screen).
*Games:* all; Chaos's spell menu is gameplay-critical. *Now:* ROM-font `text`/`text_u16`
only. *Dial:* host now; pure font/menu draw needs const glyph data — `cell80`-gated.
*Env:* menus are part of the action space when an agent must navigate them.

### Tier 3 — ambitious / genre-expanding (do host-first, or after the core)

**C10. Scrolling** — full-screen, partial-window, and **multi-directional** scroll over
a map larger than the screen (Gauntlet camera follows the player; Renegade horizontal).
*Now:* none (full redraw each frame). *Dial:* host now (cheap host-side); pure is harder
(dirty-cell/scroll engine is a known **dial canary**, spec 08 §10.2). *Env:* a moving
viewport over a big map — the observation is the window, the state is the full map.

**C11. Pseudo-3D & isometric** — a **road renderer** (perspective segments + curvature)
with **sprite scaling** (Chase HQ, OutRun); **isometric** depth-sorted rooms (Knight
Lore). *Now:* none. *Dial:* **host-first, explicitly** — these are the games where a host
escape-hatch is honest (spec 08 §10.4); the pure path is a long way off. *Env:* the
`tracking` axis (stay on the road / catch the target).

### Tier 4 — audio depth (gated on the core, not the SDK)

**C12. AY music (128K)** — 3-channel `const` tracker songs + SFX channel. *Games:*
OutRun/Chase HQ BGM, most late titles. *Now:* 48K beeper only. *Dial:* blocked on
**128K + AY in the core** (roadmap, *below E in priority*) — then a host tracker player,
then const-data for pure. *Env:* music state is rarely an RL signal; low env priority,
high *demo* value.

### Cross-cutting — the asset pipeline (extends `chuk-speccy-assets`)

**C13. Asset baking** — the connective tissue for C1/C2/C4/C12. Today: image →
`.scr` and image → `const Tile` (single + sheet) + clash report. Next, in leverage order:
- **Spritesheet → animation bank** — a strip/grid PNG → `[Tile; N]` *plus* frame-timing
  metadata (already half-there: `bake` emits `[Tile; N]`; add named frames + a mask).
- **Tiled `.tmx` → `TileMap` const** — author maps in Tiled, bake to tile-index arrays +
  the flag table (feeds C2). The big authoring-ergonomics win for room/level games.
- **Tracker module → `const` song** — (feeds C4 beeper now, C12 AY later).
- **Clash report everywhere** — already per-cell (`.scr`) and per-tile (`bake`); extend
  to spritesheets and Tiled maps so "where will this break on hardware" covers all art.

*Dial:* all host tooling (no dial cost — it *emits* `const`). *Env:* none directly, but
it's what makes authored games (hence benchmarks) cheap to produce.

---

## 3. The dial map — what's blocked on `cell80`

Host-composite gets you **all** of the above *now* (and host games are still real
Spectrum games — themes, GIF, MCP, snapshots, RL all apply). The **pure `.tap`** path
unlocks per `cell80`/`rustz80` frontend feature (see `cell80/docs/roadmap.md`):

| `cell80` feature | Unlocks (pure) |
|---|---|
| `&CONST → addr` const-data | C1 sprite bitmaps, C2 tile banks, C4/C12 song data, C9 fonts — **the big one** (the `Frame::tile`/`text`-by-address payoff) |
| `[u8; N]` arrays | C2 tile maps, C6 actor tables, C8 inventory |
| persisted `u32`/`i16` (non-16-bit slots) | C3 fixed-point physics, C8 larger economies |
| nested struct fields (`self.player.x`) | C1/C3/C8 composable `Sprite`/`Actor` as struct fields |

So the strategy is the **build→extract→compiler loop**, pure-first: **build real games
inside today's envelope** (`fill_cell`/`pixel` draws, `u8`/`u16` state, `[u16; N]` /
`[Cell; N]` pools, direction-flag motion, free functions — see `samples/bounce.rs`,
`snake_game.rs`), **harvest** the reusable pattern into the kit, and **push the
compiler** (route a new prelude fn in `compile.rs`, or land the `cell80` frontend
feature) for what the envelope can't yet express. Crucially, most game *logic*
— movement, collision, jump arcs, actor behaviour, inventory, scene flow — compiles
**pure today** (drawn blocky with `fill_cell`); what's genuinely gated is the **art /
audio *data* layer** (bitmap sprites/tiles/fonts, tracker songs). Treat that as the next
compiler milestone, not as a cue to fall back to host. Host-composite stays a fast
preview, and an honest escape hatch only for the explicitly ambitious (pseudo-3D) — never
the default.

---

## 4. Recommended SDK build order (pure-first, via the build→extract→compiler loop)

Lead with the *logic* that compiles **pure today** (blocky `fill_cell` graphics, real
bootable `.tap`s); each step harvests kit patterns and surfaces the next compiler ask.
Every step is immediately agentable.

1. **The pure arcade core — C3 + C2-logic + C5 + C8** (build it as real sample games:
   a **platformer** then a **maze/room** game). Cell-based movement, jump arcs (phase
   counters / direction flags, no signed ints needed — see `bounce.rs`), tile collision
   over a state-resident grid (`[u16; N]` + a `solid(cx,cy)` free fn), scene flow, and
   score/lives/keys. This is the Manic Miner / Booty / Dizzy / Mikie *mechanics* as
   bootable games **now**, and it lands the `maze`/exploration benchmark axis. *Harvest:*
   the jump/collision/tilemap helpers the kit should own.
2. **C6 Actor/AI + C7 richer input** — pure logic over `[Cell; N]` / `[u16; N]` pools:
   patrols, chase-the-player, spawn waves; combos/waggle/hotseat. Gauntlet hordes,
   Renegade moves, Chaos turns. Biggest *action-space* growth for agents.
3. **Push the compiler for the art/audio *data* layer** — the genuinely gated milestone:
   `&CONST → addr` (bitmap **C1 sprites** / **C2 tile graphics** / **C9 fonts**),
   `[u8; N]`, persisted `u32`/fixed-point (an `Fx8_8` *type* and **C13**'s baked-`const`
   payoff), nested struct fields (composable `Sprite`/`Actor` fields). This turns the
   blocky pure games pretty *without re-architecting* — host and pure share one `Game` ABI.
4. **C4 Beeper SFX → C12 AY music (128K)** — `const` song data (so `&CONST→addr`-gated),
   then the 128K/AY core work. Cheap polish first, real music later.
5. **C10 scrolling, C11 pseudo-3D / isometric** — ambitious; scrolling stress-tests the
   dial, pseudo-3D is the one honest host-first escape hatch.

The throughline back to the thesis: **a deterministic task factory that expresses tasks
as real Spectrum games** (spec 08 §10). Each capability is a genre *and* a cognitive
axis — Chaos→`puzzle`, Gauntlet→`shooter`/exploration, Dizzy→exploration+memory,
Daley→reaction/rhythm, OutRun→tracking. Build the SDK and the benchmark suite grows with
it. **The better the SDK, the easier the games — and the richer the bench.**

---

*Companion to [`08 — authoring plane`](./08-speccy-kit-authoring-plane-spec.md)
(§ the dial, the symbol map, the env surface) and roadmap track **E** §2 (the kit).*
