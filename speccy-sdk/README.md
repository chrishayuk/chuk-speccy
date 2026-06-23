# `speccy-sdk` — write a Spectrum game in native Rust

Write `impl Game`, get a game running on the (emulated) Spectrum. Your logic and
rendering are **plain Rust on the host**; a tiny Z80 runtime in Spectrum RAM calls
back to it once per 50 Hz frame over the [`ED FE` trap ABI](../docs/03-sdk-spec.md).
You think in pixels, tiles, and colours — never in Z80.

This is the *hybrid* end of the fidelity dial (spec 03): full host power, instant
iteration. For a **pure** build that runs on real hardware with no host, write the
same kind of game in the [`rustz80`](../rustz80/README.md) dialect instead.

## Run the demos

The demo games live in [`chuk-speccy-games`](../speccy-games) (content built *on*
this library, not part of it). Run any by name in the native window:

```bash
cargo run --release --bin speccy-gui -- testroms/48.rom snake     # a grid Snake
cargo run --release --bin speccy-gui -- testroms/48.rom keytest   # input visualiser
cargo run --release --bin speccy-gui -- testroms/48.rom typing    # font / typing test
cargo run --release --bin speccy-gui -- testroms/48.rom mover     # move a blob (WASD — remapped controls)
```

## Write one

```rust
use speccy_sdk::{boot, Button, Colour, Frame, Game, Input};

struct Pong { x: u8 }

impl Game for Pong {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if input.held(Button::Left)  { self.x = self.x.saturating_sub(1); }
        if input.held(Button::Right) { self.x = self.x.saturating_add(1); }
        frame.clear(Colour::Black);
        frame.ink(Colour::BrightCyan).text(0, 0, "PONG");
        frame.pixel(self.x, 100, true);
    }
}

fn main() {
    let rom = std::fs::read("testroms/48.rom").unwrap();
    let mut spec = boot(&rom, Pong { x: 128 });
    loop { spec.run_frame(); /* … present spec.screen_rgba() in your head … */ }
}
```

`boot(rom, game)` returns a ready [`Spectrum`](../spectrum) with the runtime
installed; drive it with `run_frame()` and present its framebuffer through any head
(the `speccy-gui`/`speccy` frontends already do this). `install(&mut spec, game)`
adds a game to a machine you built yourself.

## The author API

| Item | What |
|---|---|
| `trait Game` | `fn update(&mut self, input: &Input, frame: &mut Frame)` — called once per frame. |
| `enum Button` | `Up`/`Down`/`Left`/`Right`/`Fire` (keyboard or joystick, pre-mapped). |
| `Input` | `held(b)` — down now; `pressed(b)` — rising edge this frame. |
| `Frame` | `clear(paper)`, `ink(c)`, `pixel(x, y, on)`, `tile(&t, cx, cy)`, `attr(cx, cy, a)`, `text(cx, cy, s)`. |
| `enum Colour` | The 8 Spectrum colours + `Bright*` variants. |
| `Attr` | `Attr::new(ink, paper, flash)` / `Attr::ink(c)` — a colour cell. |
| `Tile` + `BLOCK` | An 8×8 bitmap (`Tile { rows: [u8; 8] }`); `BLOCK` is solid. |

**Screen model.** 256×192 pixels in a 32×24 grid of 8×8 cells; colour is
per-cell (one ink + one paper + bright + flash), exactly like the real machine.
`pixel`/`tile` set the bitmap; `attr`/`ink` set cell colour; `text` uses the ROM
font. `Frame` writes straight into the interleaved screen layout for you.

## Determinism

Keep all state in your `Game` and derive randomness from it (the demo seeds its RNG
from game state and counts frames). Then the machine is fully deterministic — which
is what makes a snapshot a rewindable timeline branch, and the same game usable as
an RL environment (spec 02/06). Avoid wall-clock time and ambient `rand`.

## How it runs (one paragraph)

`load_runtime` puts an 11-byte Z80 loop at `0x8000` that does
`HALT` (wait for the 50 Hz interrupt) → `ED FE` with `GAME_TICK` (`0x60`) in `A` →
loop. The host trap handler reads the keyboard into an `Input`, calls your
`Game::update` with a `Frame` over the live screen RAM, and returns — so your Rust
runs once per frame and its drawing lands in the machine's display. See
[`src/lib.rs`](./src/lib.rs) and the [SDK spec](../docs/03-sdk-spec.md).

## Tests

`cargo test -p chuk-speccy-sdk` covers the screen interleave, tiles/attrs, input
edges, `Rng`, and `Entities`. The demo games + their ROM-backed render test live in
[`chuk-speccy-games`](../speccy-games) (`cargo test -p chuk-speccy-games -- --ignored`).
