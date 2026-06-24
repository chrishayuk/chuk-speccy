// A fixed-capacity pool of structs — `[Cell; N]` as a *struct field* (the
// `Entities<T, N>` shape). `push` appends an element through the receiver pointer;
// `checksum` reads `self.items[i].x`/`.y`. Same source under rustc and rustz80.
// Entry `run` -> 913.

struct Cell {
    x: u16,
    y: u16,
}

struct Pool {
    items: [Cell; 8],
    len: u16,
}

impl Pool {
    fn push(&mut self, x: u16, y: u16) {
        if self.len < 8u16 {
            self.items[self.len as usize] = Cell { x: x, y: y };
            self.len = self.len + 1u16;
        }
    }
    fn checksum(&self) -> u16 {
        let mut s = 0u16;
        let mut i = 0u16;
        while i < self.len {
            s = s + self.items[i as usize].x * 100u16 + self.items[i as usize].y;
            i = i + 1u16;
        }
        s
    }
}

fn run() -> u16 {
    let mut p = Pool {
        items: [Cell { x: 0u16, y: 0u16 }; 8],
        len: 0u16,
    };
    p.push(1u16, 2u16);
    p.push(3u16, 4u16);
    p.push(5u16, 6u16);
    p.checksum() + p.items[0].x // 912 + 1
}
