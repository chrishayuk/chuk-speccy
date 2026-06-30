// Snake, in the rustz80 dialect — a self-playing demo. Not built by cargo (it
// uses the poke prelude intrinsic); compile it to a bootable tape:
//
//     cargo run -p chuk-speccy-sdk --features compile --bin speccy-compile -- speccy-sdk/samples/snake.rs -o snake.tap
//     cargo run --release --bin speccy-gui -- testroms/48.rom snake.tap
//
// The snake (8 chunky 8x8 segments) crawls around the screen, turning every few
// steps. Tune `while s < 600` for length of play and the `delay()` bound for speed.
// Shows the dialect's bounded control flow: `for` range loops and `loop`/`break`.

fn addr_of(x: u16, y: u16) -> u16 {
    16384
        + (y / 64) * 2048
        + (y % 8) * 256
        + ((y / 8) % 8) * 32
        + x / 8
}

// Fill (v=255) or clear (v=0) an 8x8 character cell at grid (cx, cy).
fn fill_cell(cx: u16, cy: u16, v: u16) {
    let x = cx * 8;
    let y = cy * 8;
    for r in 0..8 {
        poke(addr_of(x, y + r), v as u8);
    }
}

// A busy-wait so the animation is watchable rather than instant.
fn delay() {
    for _ in 0..4000 {}
}

fn main() {
    // Clear the screen: blank the bitmap, set bright-white ink on black paper
    // (attr 0x47) so the snake stands out over a clean background.
    let mut p = 16384;
    while p < 22528 {
        poke(p, 0);
        p = p + 1;
    }
    while p < 23296 {
        poke(p, 71);
        p = p + 1;
    }

    let mut bx = [0; 8];
    let mut by = [0; 8];

    // Initial body: a horizontal run, head at cell (8, 12).
    for i in 0..8 {
        bx[i as usize] = 8 - i;
        by[i as usize] = 12;
        fill_cell(bx[i as usize], by[i as usize], 255);
    }

    let mut dir = 0; // 0=right 1=down 2=left 3=up
    let mut s = 0;
    loop {
        if s >= 600 {
            break;
        }
        // Turn clockwise every 7 steps → the snake traces boxes.
        if (s % 7) == 6 {
            dir = (dir + 1) % 4;
        }

        let mut nx = bx[0];
        let mut ny = by[0];
        match dir {
            0 => nx = (nx + 1) % 32,
            1 => ny = (ny + 1) % 24,
            2 => nx = (nx + 31) % 32,
            _ => ny = (ny + 23) % 24,
        }

        fill_cell(bx[7], by[7], 0); // erase the tail

        let mut j = 7;
        while j > 0 {
            bx[j as usize] = bx[(j - 1) as usize];
            by[j as usize] = by[(j - 1) as usize];
            j = j - 1;
        }
        bx[0] = nx;
        by[0] = ny;

        fill_cell(nx, ny, 255); // draw the new head
        delay();
        s = s + 1;
    }
}
