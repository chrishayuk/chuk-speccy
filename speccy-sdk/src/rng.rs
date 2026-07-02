//! A small, deterministic PRNG — **seed it from game state, never the clock** (spec
//! 08 §1: the determinism contract, as a type).

/// xorshift32; seeding the env from a known value makes every episode reproducible.
#[derive(Copy, Clone)]
pub struct Rng {
    state: u32,
}

impl Default for Rng {
    fn default() -> Self {
        Rng::seed(0)
    }
}

impl Rng {
    /// Seed the generator. Zero is mapped to a fixed non-zero constant (xorshift
    /// must never have a zero state).
    pub fn seed(seed: u32) -> Self {
        Rng {
            state: if seed == 0 { 0x9E37_79B9 } else { seed },
        }
    }

    /// Next 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// A value in `[0, n)` (`n` must be non-zero). Uses a 32-bit `%`, so it is
    /// host-only — **not** subset-clean for the pure tape. For the pure path use
    /// [`Rng::below_mask`] (a power-of-two range via a bitwise mask).
    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }

    /// A value in `[0, mask + 1)` for a `mask` of the form `2^k - 1` — the
    /// **subset-clean** ranged draw (spec 08 §1): just `next_u32() & mask`, which
    /// compiles to the pure tape (u32 has `&` but no `%`). For a range that isn't a
    /// power of two, draw from the next power of two and **reject** in the caller's
    /// loop — see `Snake::spawn`.
    pub fn below_mask(&mut self, mask: u32) -> u32 {
        self.next_u32() & mask
    }
}

/// The subset-clean pure-compilable core of a `u16` xorshift step: one state in, the
/// next state out — a plain function, not a method on [`Rng`], since a pure game holds
/// its RNG state as a flat `u16` field (nested struct fields aren't yet a
/// pure-compilable construct; spec 08 §1, so [`Rng`] itself — `u32`-backed — stays
/// host-only for now). The **same** function is defined in the dialect prelude
/// (`compile::PRELUDE`): a game's source calls `rng_next_u16(self.rng)` unchanged under
/// both `rustc` (resolved via `use speccy_sdk::*`) and `rustz80` (resolved via the
/// prelude), so one function replaces every game's hand-rolled inline xorshift. The two
/// bodies are pinned equal by `dial::rng_host_and_pure_agree`.
pub fn rng_next_u16(state: u16) -> u16 {
    let mut x = state;
    x ^= x << 7;
    x ^= x >> 9;
    x ^= x << 8;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_seedable() {
        let mut a = Rng::seed(12345);
        let mut b = Rng::seed(12345);
        let seq_a: Vec<u32> = (0..8).map(|_| a.next_u32()).collect();
        let seq_b: Vec<u32> = (0..8).map(|_| b.next_u32()).collect();
        assert_eq!(seq_a, seq_b, "same seed → same sequence");
        let mut c = Rng::seed(54321);
        assert_ne!(c.next_u32(), seq_a[0], "different seed → different stream");
        let mut d = Rng::seed(7);
        for _ in 0..100 {
            assert!(d.below(6) < 6, "below(n) stays in range");
        }
    }

    #[test]
    fn below_mask_is_subset_clean_and_bounded() {
        // The pure-path ranged draw: `next_u32() & mask`, so a `2^k-1` mask bounds it.
        let mut r = Rng::seed(99);
        for _ in 0..200 {
            assert!(r.below_mask(31) < 32, "below_mask(31) stays in [0, 32)");
        }
        // It is exactly `next_u32() & mask` (the form the dialect compiles).
        let mut a = Rng::seed(1);
        let mut b = Rng::seed(1);
        assert_eq!(a.below_mask(0x0F), b.next_u32() & 0x0F);
    }
}
