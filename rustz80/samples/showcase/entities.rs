// The full `Entities<Cell, const N>` shape: a const-generic struct whose field is an
// array of structs (`data: [Cell; N]`), monomorphized per capacity — used here at 4 and
// 8, giving `Entities$4` and `Entities$8`. `N` bounds `add`; `N` is inferred from each
// literal's array length. Same source under rustc and rustz80. Entry `run` -> 2530.

struct Cell {
    x: u16,
    y: u16,
}

struct Entities<const N: usize> {
    data: [Cell; N],
    len: u16,
}

impl<const N: usize> Entities<N> {
    fn add(&mut self, x: u16, y: u16) {
        if self.len < N as u16 {
            self.data[self.len as usize] = Cell { x: x, y: y };
            self.len = self.len + 1u16;
        }
    }
    fn checksum(&self) -> u16 {
        let mut s = 0u16;
        let mut i = 0u16;
        while i < self.len {
            s = s + self.data[i as usize].x * 100u16 + self.data[i as usize].y;
            i = i + 1u16;
        }
        s
    }
}

fn run() -> u16 {
    let mut a = Entities {
        data: [Cell { x: 0u16, y: 0u16 }; 4],
        len: 0u16,
    };
    a.add(1u16, 2u16);
    a.add(3u16, 4u16);

    let mut b = Entities {
        data: [Cell { x: 0u16, y: 0u16 }; 8],
        len: 0u16,
    };
    b.add(5u16, 6u16);
    b.add(7u16, 8u16);
    b.add(9u16, 10u16);

    a.checksum() + b.checksum() // 406 + 2124
}
