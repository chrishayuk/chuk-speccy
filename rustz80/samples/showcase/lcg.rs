// A 16-bit linear-congruential PRNG, in the rustz80 dialect.
// Entry `rng_hash(seed, n)` -> XOR-fold of n outputs.
// Shows: `wrapping_mul`/`wrapping_add` (mod-2^16 via __mul16), `^`, args in HL/DE.
//
// Constants 25173/13849 are the classic ZX Spectrum 16-bit LCG pair.

fn rng_hash(seed: u16, n: u16) -> u16 {
    let mut state = seed;
    let mut acc = 0u16;
    let mut i = 0u16;
    while i < n {
        state = state.wrapping_mul(25173u16).wrapping_add(13849u16);
        acc = acc ^ state;
        i = i + 1u16;
    }
    acc
}
