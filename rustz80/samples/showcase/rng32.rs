// A 32-bit **xorshift** RNG — the SDK `Rng` core — in the dialect: a `u32` state with
// `^` and constant `<<` / `>>` shifts, truncated to a `u16` per step. Steps the state 8
// times and folds the low words into a checksum. Same source under rustc and rustz80.
// Entry `run`.

fn run() -> u16 {
    let mut x: u32 = 2463534242u32; // xorshift32 seed (0x9295_8AE2)
    let mut sum = 0u16;
    let mut i = 0u16;
    while i < 8u16 {
        x = x ^ (x << 13u32);
        x = x ^ (x >> 17u32);
        x = x ^ (x << 5u32);
        sum = sum + (x as u16);
        i = i + 1u16;
    }
    sum
}
