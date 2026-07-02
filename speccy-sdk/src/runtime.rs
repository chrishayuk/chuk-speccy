//! The host-composite runtime: the ~11-byte Z80 frame pump, the `GAME_TICK` host
//! trap it calls into each frame, and the [`Dispatcher`] that wires a [`Game`] to it.

use spectrum::host::{HostCalls, HostCtx};
use spectrum::Spectrum;

use crate::graphics::frame::FONT_BYTES;
use crate::{Controls, Frame, Game, Input};

/// The per-frame host syscall id (`docs/03` id map, `0x60` = game).
pub const GAME_TICK: u8 = 0x60;

/// Where the runtime pump is loaded / entered.
pub const RUNTIME_ORG: u16 = 0x8000;

/// The entire Z80 guest program (§1): `di; im 1; ei; loop: halt; ld a,0x60;
/// HOSTCALL; jr loop`. It contributes the authentic frame clock + display + I/O
/// model; the host does the rest.
pub const RUNTIME: [u8; 11] = [
    0xF3, // di
    0xED, 0x56, // im 1
    0xFB, // ei
    0x76, // loop: halt
    0x3E, 0x60, // ld a, GAME_TICK
    0xED, 0xFE, // HOSTCALL
    0x18, 0xF9, // jr loop
];

/// Load the runtime pump and point the CPU at it. Boot the ROM first (the runtime
/// relies on the ROM's IM 1 interrupt handler for the frame sync).
pub fn load_runtime(spec: &mut Spectrum) {
    spec.write_memory(RUNTIME_ORG, &RUNTIME);
    spec.cpu.regs.pc = RUNTIME_ORG;
}

/// Install `game` as the host's `GAME_TICK` handler with the default [`Controls`].
/// Pair with [`load_runtime`].
pub fn install(spec: &mut Spectrum, game: impl Game + Send + 'static) {
    install_with_controls(spec, game, Controls::default());
}

/// Install `game` with a custom key mapping (e.g. WASD). Pair with [`load_runtime`].
pub fn install_with_controls(
    spec: &mut Spectrum,
    game: impl Game + Send + 'static,
    controls: Controls,
) {
    spec.set_host_dispatcher(Box::new(Dispatcher::new(game, controls)));
}

/// Convenience: boot the ROM, load the runtime, and install `game` — ready for a
/// head to step. (`rom` is the 16K system ROM.)
pub fn boot(rom: &[u8], game: impl Game + Send + 'static) -> Spectrum {
    let mut spec = Spectrum::new_48k(rom);
    for _ in 0..200 {
        spec.run_frame(); // bring up the ROM (IM 1 handler + system vars)
    }
    install(&mut spec, game);
    load_runtime(&mut spec);
    spec
}

struct Dispatcher<G> {
    game: G,
    frame: Frame,
    controls: Controls,
    prev: u8,
    font_loaded: bool,
}

impl<G: Game> Dispatcher<G> {
    fn new(game: G, controls: Controls) -> Self {
        Dispatcher {
            game,
            frame: Frame::new(),
            controls,
            prev: 0,
            font_loaded: false,
        }
    }
}

impl<G: Game + Send + 'static> HostCalls for Dispatcher<G> {
    fn dispatch(&mut self, ctx: &mut HostCtx) -> u32 {
        if ctx.id() != GAME_TICK {
            ctx.fail();
            return 0;
        }
        // Lift the 8×8 font from the ROM once (chars 32..127 at $3D00).
        if !self.font_loaded {
            self.frame.load_font(&ctx.read(0x3D00, FONT_BYTES as u16));
            self.font_loaded = true;
        }
        let cur = self.controls.read(ctx);
        let input = Input {
            cur,
            prev: self.prev,
        };
        self.prev = cur;

        self.game.update(&input, &mut self.frame);

        ctx.write(0x4000, &self.frame.pixels);
        ctx.write(0x5800, &self.frame.attrs);
        ctx.ok();
        0
    }
}
