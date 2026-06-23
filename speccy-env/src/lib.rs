//! Agent environments over the deterministic core (`docs/08-speccy-kit-authoring-plane-spec.md`).
//!
//! This is the **env side of the bridge** (spec 08 §2–§3). The compiler emits a
//! symbol map (`.sym.toml`) describing where each game-state field lives in RAM.
//! Here we read those fields off a *running `.tap`* into a [`StateView`],
//! reconstruct a host `Self` from it ([`FromState`]), and run the **same**
//! host-compiled `reward`/`done`/`observe` ([`speccy_sdk::Game`]) over it — so the
//! environment "falls out of the types" even though the types don't exist on real
//! hardware. Reset is bit-exact (`serialize_full`/`deserialize_full`), so episodes
//! are reproducible.
//!
//! ```ignore
//! let (tap, map) = speccy_sdk::compile::compile_game_with_symbols(src, "GAME")?;
//! let map = speccy_env::SymbolMap::from_toml(&map.to_toml())?;
//! let mut env = speccy_env::SpectrumEnv::new(&rom, &tap, map, 450);
//! let step = env.step_game::<MyGame>(&['o'], 4);   // hold O for 4 frames
//! // step.reward / step.done / step.obs — computed host-side from tape RAM
//! ```

use spectrum::keyboard::{self, KeyPos};
use spectrum::Spectrum;

pub mod agents;

pub use speccy_sdk::Obs;

/// The symbol map + symbol type come from the SDK (one source of truth for the
/// `.sym.toml` contract — emit on the SDK's `compile` feature, parse here).
pub use speccy_sdk::{Symbol, SymbolMap};

/// A snapshot of the game's typed fields, read from RAM via the [`SymbolMap`].
/// The full layout is always present (spec 08 §2), so any `Self` can be rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateView {
    values: Vec<(String, u16)>,
}

impl StateView {
    /// Build a synthetic view (for tests, or agents reasoning over hand-made state).
    pub fn from_pairs(pairs: &[(&str, u16)]) -> StateView {
        StateView {
            values: pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect(),
        }
    }

    /// Read a field as `u16` (little-endian); `0` if the field is unknown.
    pub fn u16(&self, name: &str) -> u16 {
        self.values
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v)
            .unwrap_or(0)
    }
    /// Read a field as `u8` (the low byte of its slot).
    pub fn u8(&self, name: &str) -> u8 {
        self.u16(name) as u8
    }
    /// Read a field as `bool` (non-zero).
    pub fn bool(&self, name: &str) -> bool {
        self.u16(name) != 0
    }
}

/// Reconstruct a host game value from a [`StateView`] read off tape RAM — the
/// other half of the dial's bridge. Implement it next to your `Game` so the env
/// can run the *same* host `reward`/`done`/`observe` over a running pure tape.
pub trait FromState {
    fn from_state(s: &StateView) -> Self;
}

/// The transition returned by [`SpectrumEnv::step_game`].
#[derive(Debug, Clone)]
pub struct Transition {
    pub obs: Obs,
    pub reward: i16,
    pub done: bool,
}

/// A Gym-style environment wrapping a running `.tap` on the deterministic core.
/// `reset` is bit-exact (a post-warmup `serialize_full` snapshot), so episodes
/// reproduce; `step` presses keys + advances frames; reward/done/observe are read
/// off RAM via the symbol map and computed host-side.
pub struct SpectrumEnv {
    spec: Spectrum,
    map: SymbolMap,
    snapshot: Vec<u8>,
}

impl SpectrumEnv {
    /// Boot the ROM, load `tap`, warm up `warmup` frames past the tape load into
    /// the game loop, then snapshot that point as the reset state.
    pub fn new(rom: &[u8], tap: &[u8], map: SymbolMap, warmup: usize) -> Self {
        let mut spec = Spectrum::new_48k(rom);
        for _ in 0..250 {
            spec.run_frame(); // boot to the K cursor
        }
        let _ = spec.load_tap(tap);
        spec.autoload_tape();
        for _ in 0..warmup {
            spec.run_frame(); // load + auto-run into the frame loop
        }
        let snapshot = spec.serialize_full();
        SpectrumEnv {
            spec,
            map,
            snapshot,
        }
    }

    /// Reset to the warmup snapshot — bit-exact, so the next episode is identical
    /// given the same actions.
    pub fn reset(&mut self) {
        self.spec
            .deserialize_full(&self.snapshot)
            .expect("own snapshot deserializes");
    }

    /// The current typed state, read off RAM via the symbol map.
    pub fn view(&self) -> StateView {
        let values = self
            .map
            .fields
            .iter()
            .map(|f| {
                let bytes = self.spec.read_memory(f.addr, f.width.max(1) as u16);
                let v = bytes.first().copied().unwrap_or(0) as u16
                    | (*bytes.get(1).unwrap_or(&0) as u16) << 8;
                (f.field.clone(), if f.width <= 1 { v & 0xFF } else { v })
            })
            .collect();
        StateView { values }
    }

    /// Reconstruct a host game value from the current RAM state.
    pub fn reconstruct<G: FromState>(&self) -> G {
        G::from_state(&self.view())
    }

    /// The indexed (palette-index) framebuffer — the pixel observation.
    pub fn frame_indexed(&self) -> Vec<u8> {
        self.spec.screen_indexed()
    }

    /// Hold `keys` (by character) down for `frames` frames, then release.
    pub fn hold(&mut self, keys: &[char], frames: usize) {
        let positions: Vec<KeyPos> = keys
            .iter()
            .filter_map(|&c| keyboard::key_for_char(c).map(|(p, _, _)| p))
            .collect();
        for &p in &positions {
            self.spec.set_key(p, true);
        }
        for _ in 0..frames {
            self.spec.run_frame();
        }
        for &p in &positions {
            self.spec.set_key(p, false);
        }
    }

    /// One agent step: reconstruct `prev`, hold `keys` for `frames`, reconstruct
    /// `cur`, then evaluate the host game's `observe`/`reward`/`done` over the live
    /// tape state (spec 08 §3 — the same code that runs in the host build).
    pub fn step_game<G: speccy_sdk::Game + FromState>(
        &mut self,
        keys: &[char],
        frames: usize,
    ) -> Transition {
        let prev: G = self.reconstruct();
        self.hold(keys, frames);
        let cur: G = self.reconstruct();
        Transition {
            obs: cur.observe(),
            reward: cur.reward(&prev),
            done: cur.done(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# emitted by rustz80
[state]
base = 0xB000
size = 4

[fields]
"score" = { addr = 0xB000, width = 2, ty = "u16" }
"started" = { addr = 0xB002, width = 1, ty = "u8" }
"#;

    #[test]
    fn parses_sym_toml() {
        let m = SymbolMap::from_toml(SAMPLE).expect("parse");
        assert_eq!(m.base, 0xB000);
        assert_eq!(m.size, 4);
        assert_eq!(m.fields.len(), 2);
        assert_eq!(m.addr_of("score"), Some(0xB000));
        assert_eq!(m.addr_of("started"), Some(0xB002));
        assert_eq!(m.fields[1].width, 1);
        assert_eq!(m.fields[0].ty, "u16");
        assert_eq!(m.addr_of("nope"), None);
    }
}
