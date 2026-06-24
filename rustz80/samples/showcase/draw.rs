// Drawing into real ZX Spectrum screen memory, in the rustz80 dialect.
// Entry `draw` -> pokes a box + both diagonals into the top-left 64x64 region.
// Shows: `poke`/`peek` raw memory, the (non-linear) screen-address math, loops.

fn addr_of(x: u16, y: u16) -> u16 {
    16384u16
        + (y / 64u16) * 2048u16
        + (y % 8u16) * 256u16
        + ((y / 8u16) % 8u16) * 32u16
        + x / 8u16
}

fn set_px(x: u16, y: u16) {
    let masks = [128u8, 64u8, 32u8, 16u8, 8u8, 4u8, 2u8, 1u8];
    let a = addr_of(x, y);
    poke(a, peek(a) | masks[(x % 8u16) as usize]);
}

fn draw() {
    // A box border around the top-left 64x64 region…
    for x in 0u16..64u16 {
        set_px(x, 0u16);
        set_px(x, 63u16);
    }
    for y in 0u16..64u16 {
        set_px(0u16, y);
        set_px(63u16, y);
    }
    // …with both diagonals through it.
    let mut i = 0u16;
    while i < 64u16 {
        set_px(i, i);
        set_px(63u16 - i, i);
        i = i + 1u16;
    }
}
