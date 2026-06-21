// Snake, in the rustz80 dialect — a self-playing demo. Not built by cargo (it
// uses the poke prelude intrinsic); compile it to a bootable tape:
//
//     cargo run -p rustz80 --bin speccy-compile -- rustz80/samples/snake.rs -o snake.tap
//     cargo run --release --bin speccy-gui -- testroms/48.rom snake.tap
//
// The snake (8 chunky 8x8 segments) crawls around the screen, turning every few
// steps. Tune `while s < 500` for length of play and `while d < 5000` for speed.

fn addr_of(x: u16, y: u16) -> u16 {
    16384u16
        + (y / 64u16) * 2048u16
        + (y % 8u16) * 256u16
        + ((y / 8u16) % 8u16) * 32u16
        + x / 8u16
}

// Fill (v=255) or clear (v=0) an 8x8 character cell at grid (cx, cy).
fn fill_cell(cx: u16, cy: u16, v: u16) {
    let x = cx * 8u16;
    let y = cy * 8u16;
    let mut r = 0u16;
    while r < 8u16 {
        poke(addr_of(x, y + r), v as u8);
        r = r + 1u16;
    }
}

// A busy-wait so the animation is watchable rather than instant.
fn delay() {
    let mut d = 0u16;
    while d < 4000u16 {
        d = d + 1u16;
    }
}

fn main() {
    let mut bx = [0u16; 8];
    let mut by = [0u16; 8];

    // Initial body: a horizontal run, head at cell (8, 12).
    let mut i = 0u16;
    while i < 8u16 {
        bx[i as usize] = 8u16 - i;
        by[i as usize] = 12u16;
        fill_cell(bx[i as usize], by[i as usize], 255u16);
        i = i + 1u16;
    }

    let mut dir = 0u16; // 0=right 1=down 2=left 3=up
    let mut s = 0u16;
    while s < 600u16 {
        // Turn clockwise every 7 steps → the snake traces boxes.
        if (s % 7u16) == 6u16 {
            dir = (dir + 1u16) % 4u16;
        }

        let mut nx = bx[0];
        let mut ny = by[0];
        match dir {
            0u16 => nx = (nx + 1u16) % 32u16,
            1u16 => ny = (ny + 1u16) % 24u16,
            2u16 => nx = (nx + 31u16) % 32u16,
            _ => ny = (ny + 23u16) % 24u16,
        }

        fill_cell(bx[7], by[7], 0u16); // erase the tail

        let mut j = 7u16;
        while j > 0u16 {
            bx[j as usize] = bx[(j - 1u16) as usize];
            by[j as usize] = by[(j - 1u16) as usize];
            j = j - 1u16;
        }
        bx[0] = nx;
        by[0] = ny;

        fill_cell(nx, ny, 255u16); // draw the new head
        delay();
        s = s + 1u16;
    }
}
