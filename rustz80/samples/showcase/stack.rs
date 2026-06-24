// A const-generic fixed-capacity stack: the const param `N` sizes the `[u16; N]`
// array *and* bounds `push`. Used at two capacities, so two instances are generated
// (`Stack$4`, `Stack$8`) — each with its own layout and methods. Same source under
// rustc and rustz80.
// Entry `run` -> a Stack<4> sum (capacity-capped) * 1000 + a Stack<8> sum.

struct Stack<const N: usize> {
    data: [u16; N],
    len: u16,
}

impl<const N: usize> Stack<N> {
    fn push(&mut self, v: u16) {
        if self.len < N as u16 {
            self.data[self.len as usize] = v;
            self.len = self.len + 1u16;
        }
    }
    fn sum(&self) -> u16 {
        let mut s = 0u16;
        let mut i = 0u16;
        while i < self.len {
            s = s + self.data[i as usize];
            i = i + 1u16;
        }
        s
    }
}

fn run() -> u16 {
    let mut a = Stack { data: [0u16; 4], len: 0u16 };
    a.push(1u16);
    a.push(2u16);
    a.push(3u16);
    a.push(4u16);
    a.push(5u16); // dropped — capacity 4

    let mut b = Stack { data: [0u16; 8], len: 0u16 };
    let mut i = 1u16;
    while i <= 8u16 {
        b.push(i);
        i = i + 1u16;
    }

    a.sum() * 1000u16 + b.sum() // 10*1000 + 36 = 10036
}
