//! The author API: the [`Game`] trait a game implements, and what it can observe.

use crate::{Frame, Input};

/// What an agent observes each step (spec 08 §3). `Screen` = the framebuffer;
/// typed-feature observations come later (read host-side, or off tape RAM via the
/// compiler-emitted symbol map).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Obs {
    Screen,
}

/// A game: pure logic + rendering, called once per 50 Hz frame.
///
/// The **env surface** — `observe`/`reward`/`done`/`reset` — has defaults so every
/// existing game compiles unchanged; override to instrument for agents (spec 08
/// §3). `reward`/`done`/`observe` must be **pure functions of `(self, prev)`**:
/// they run env-side (host, or over a `Self` reconstructed from tape RAM via the
/// symbol map), never inside the pure tape.
pub trait Game {
    fn update(&mut self, input: &Input, frame: &mut Frame);

    /// What to observe this step. Default: the screen.
    fn observe(&self) -> Obs {
        Obs::Screen
    }

    /// Reward for the transition `prev -> self`. Default: none.
    fn reward(&self, prev: &Self) -> i16
    where
        Self: Sized,
    {
        let _ = prev;
        0
    }

    /// Has the episode terminated? Default: never.
    fn done(&self) -> bool {
        false
    }

    /// Start a fresh episode from `seed` (the episode boundary; seeds [`crate::Rng`]).
    /// Defaults to `Self::default()` so games that derive `Default` need not
    /// implement it — override to actually use the seed.
    fn reset(seed: u64) -> Self
    where
        Self: Sized + Default,
    {
        let _ = seed;
        Self::default()
    }
}
