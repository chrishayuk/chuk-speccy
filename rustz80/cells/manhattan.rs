//! Manhattan distance between two grid points (typed state).
//! tags: grid, distance, spatial, score, navigation
//! entry: Pts::run
struct Pts { x1: u16, y1: u16, x2: u16, y2: u16, dist: u16 }
impl Pts {
    fn run(&mut self) -> u16 {
        let mut dx = 0u16;
        if self.x1 > self.x2 { dx = self.x1 - self.x2; } else { dx = self.x2 - self.x1; }
        let mut dy = 0u16;
        if self.y1 > self.y2 { dy = self.y1 - self.y2; } else { dy = self.y2 - self.y1; }
        self.dist = dx + dy;
        self.dist
    }
}
