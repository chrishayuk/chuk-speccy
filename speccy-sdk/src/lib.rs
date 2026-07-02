//! Native Rust game SDK — write a game in Rust; it runs on the Spectrum
//! substrate over the `ED FE` trap ABI (`docs/03-sdk-spec.md`, host-composite
//! backend). The emulated Z80 is a ~11-byte frame pump: it syncs to the 50 Hz
//! interrupt and traps to the host (`GAME_TICK`), which runs your
//! [`Game::update`] and writes the screen. Everything — input, logic, rendering
//! — is host Rust, so the result still *is* a Spectrum (themes, CRT, screenshots,
//! MCP, snapshots, RL all apply) rather than a window with a retro filter.
//!
//! Author one [`Game`]; install it on a [`spectrum::Spectrum`] with
//! [`install`] + [`load_runtime`], then spin any head (the native window, TUI,
//! headless over MCP, …). The game's logic must be a pure function of
//! `(state, input)` — seed RNG from state, count frames, no host I/O — so rewind,
//! replay and RL stay correct.
//!
//! Organised by concern, one module per file/folder — each `pub use` below is a
//! flat re-export, so `speccy_sdk::Frame`/`speccy_sdk::*` keep working unchanged
//! regardless of where a type actually lives:
//! - [`game`] — the [`Game`] trait + [`Obs`] (the author/env API).
//! - [`rng`] — [`Rng`] and the shared pure-compilable [`rng_next_u16`].
//! - [`cell`], [`entities`] — small grid types + the fixed-capacity [`Entities`].
//! - [`input`] — [`Button`]/[`Input`].
//! - [`graphics`] — [`Colour`]/[`Attr`]/[`Tile`]/[`Frame`].
//! - [`controls`] — key bindings ([`Controls`]).
//! - [`runtime`] — the host-composite frame pump + [`install`]/[`boot`].

pub use spectrum::Spectrum;

pub mod symbols;
pub use symbols::{Symbol, SymbolMap};

/// Starter game templates for `speccy new` (L0 ergonomics). Feature-free — scaffolding
/// is plain text, so the bin doesn't pull in the compiler.
pub mod templates;

/// Compile an SDK `impl Game` to a bootable `.tap` + symbol map (spec 08). Behind
/// the `compile` feature so runtime consumers don't pull in `rustz80`/`syn`.
#[cfg(feature = "compile")]
pub mod compile;

/// `speccy run` — compile a dialect game and render it running to a GIF (headless).
/// Behind the `compile` feature (it invokes the compiler).
#[cfg(feature = "compile")]
pub mod run;

/// Render a *host* [`Game`] running to a GIF, headless — the host-composite
/// counterpart of [`run`] (which renders a pure `.tap`). Feature-free: no compiler.
pub mod render;

/// The author API: the [`Game`] trait + [`Obs`].
pub mod game;
pub use game::{Game, Obs};

/// A small deterministic PRNG, and the pure-compilable core every game shares.
pub mod rng;
pub use rng::{rng_next_u16, Rng};

/// A small grid point type.
pub mod cell;
pub use cell::Cell;

/// A fixed-capacity, allocation-free vec — the subset-clean replacement for `Vec`.
pub mod entities;
pub use entities::Entities;

/// Logical buttons + this frame's held/pressed state.
pub mod input;
pub use input::{Button, Input};

/// Colours/attributes, tiles, and the [`Frame`] games draw into.
pub mod graphics;
pub use graphics::{Attr, Colour, Frame, Tile, BLOCK};

/// Key bindings: the one place a game's logical buttons map to physical keys.
pub mod controls;
pub use controls::Controls;

/// The host-composite runtime: the Z80 frame pump + the `GAME_TICK` dispatcher.
pub mod runtime;
pub use runtime::{
    boot, install, install_with_controls, load_runtime, GAME_TICK, RUNTIME, RUNTIME_ORG,
};
