// A rustz80-dialect sample. Not built by cargo (it uses the poke/peek prelude
// intrinsics); compile it to a bootable tape with:
//
//     cargo run -p rustz80 --bin speccy-compile -- rustz80/samples/pixels.rs -o pixels.tap
//
// then load pixels.tap in speccy-gui (or any Spectrum). It draws a diagonal line
// and a horizontal bar, straight into screen RAM.

fn mask_of(x: u16) -> u16 {
    let masks = [128u8, 64u8, 32u8, 16u8, 8u8, 4u8, 2u8, 1u8];
    masks[(x % 8u16) as usize] as u16
}

fn addr_of(x: u16, y: u16) -> u16 {
    16384u16
        + (y / 64u16) * 2048u16
        + (y % 8u16) * 256u16
        + ((y / 8u16) % 8u16) * 32u16
        + x / 8u16
}

fn plot(x: u16, y: u16) {
    let a = addr_of(x, y);
    poke(a, peek(a) | mask_of(x));
}

fn main() {
    let mut i = 0u16;
    while i < 176u16 {
        plot(i, i); // a diagonal
        plot(i, 100u16); // a horizontal bar
        i = i + 1u16;
    }
}
