// Generic structs + methods, and a struct with a tuple field — the same source
// compiles under rustc and rustz80. Entry `run` packs results from a generic `Vec2`
// and a tuple-field `Player` into one u16.

struct Vec2<T> {
    x: T,
    y: T,
}

impl<T: Copy + core::ops::Add<Output = T>> Vec2<T> {
    fn sum(&self) -> T {
        self.x + self.y
    }
    fn shift(&mut self, d: T) {
        self.x = self.x + d;
        self.y = self.y + d;
    }
}

struct Player {
    pos: (u16, u16),
    score: u16,
}

impl Player {
    fn step(&mut self, dx: u16, dy: u16) {
        self.pos.0 = self.pos.0 + dx;
        self.pos.1 = self.pos.1 + dy;
        self.score = self.score + 1u16;
    }
    fn key(&self) -> u16 {
        self.pos.0 * 100u16 + self.pos.1
    }
}

fn run() -> u16 {
    let mut v = Vec2 { x: 3u16, y: 4u16 };
    v.shift(10u16); // (13, 14)
    let s = v.sum(); // 27

    let mut p = Player {
        pos: (5u16, 6u16),
        score: 0u16,
    };
    p.step(2u16, 3u16); // pos (7, 9), score 1
    p.step(1u16, 0u16); // pos (8, 9), score 2

    s + p.key() + p.score // 27 + 809 + 2 = 838
}
