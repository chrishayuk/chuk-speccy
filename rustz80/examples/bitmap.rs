//! **Drawing to real screen memory** — the dialect computes ZX Spectrum bitmap
//! addresses and lights pixels with `poke`/`peek`, exactly as a game would. Shows the
//! raw-memory intrinsics, the (non-linear) screen-address math, and `for`/`while`
//! loops. We then read the framebuffer back and print it as ASCII art.
//!
//! Dialect program: [`samples/showcase/draw.rs`].
//!
//!     cargo run -p rustz80 --example bitmap

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/draw.rs");

fn main() {
    let (_hl, mem) = cpu::run(SRC, "draw", &[]);

    println!("drawn on the Z80 (8x8-cell preview of the 64x64 region):\n");
    print!("{}", cpu::screen_art(&mem, 8, 8));

    // Verify: the four corners of the box and both diagonals are lit.
    let px = |x: u16, y: u16| {
        let a = 0x4000 + ((y & 0xC0) << 5) + ((y & 0x07) << 8) + ((y & 0x38) << 2) + (x >> 3);
        mem[a as usize] & (0x80 >> (x & 7)) != 0
    };
    assert!(
        px(0, 0) && px(63, 0) && px(0, 63) && px(63, 63),
        "box corners"
    );
    for i in 0..64u16 {
        assert!(px(i, i), "main diagonal at {i}");
        assert!(px(63 - i, i), "anti-diagonal at {i}");
    }
    let lit: u32 = mem[0x4000..0x5800].iter().map(|b| b.count_ones()).sum();
    println!("\n  {lit} pixels lit");
    println!("  ✓ box + both diagonals rendered into screen RAM");
}
