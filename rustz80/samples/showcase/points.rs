// An array of structs: `[Cell; N]`. Each element is multi-slot, so `pts[i].x` is read
// at a computed address (`&pts + i*stride + field_offset`). Same source under rustc
// and rustz80. Entry `run` -> a weighted checksum over the points.

struct Cell {
    x: u16,
    y: u16,
}

fn run() -> u16 {
    let mut pts = [Cell { x: 0u16, y: 0u16 }; 5];
    let mut i = 0u16;
    while i < 5u16 {
        pts[i as usize] = Cell { x: i, y: i * i };
        i = i + 1u16;
    }

    // sum of (x + y*10) over the points
    let mut s = 0u16;
    let mut j = 0u16;
    while j < 5u16 {
        s = s + pts[j as usize].x + pts[j as usize].y * 10u16;
        j = j + 1u16;
    }
    s
}
