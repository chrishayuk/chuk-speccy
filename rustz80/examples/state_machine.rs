//! A **vending-machine state machine** — `struct` state, `enum` states, and `&mut
//! self` methods that mutate through the receiver pointer. Shows how the dialect's
//! struct/enum/method support composes into a stateful object.
//!
//! Dialect program: [`samples/showcase/vending.rs`].
//!
//!     cargo run -p rustz80 --example state_machine

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/vending.rs");

#[derive(PartialEq, Clone, Copy)]
enum State {
    Idle,
    Paid,
}

struct Machine {
    state: State,
    credit: u16,
    dispensed: u16,
}

impl Machine {
    fn insert(&mut self, coin: u16) {
        self.credit += coin;
        if self.credit >= 30 {
            self.state = State::Paid;
        }
    }
    fn vend(&mut self) {
        if self.state == State::Paid {
            self.dispensed += 1;
            self.credit -= 30;
            self.state = State::Idle;
        }
    }
}

fn host_run() -> u16 {
    let mut m = Machine {
        state: State::Idle,
        credit: 0,
        dispensed: 0,
    };
    m.insert(10);
    m.insert(10);
    m.insert(10);
    m.vend();
    m.insert(25);
    m.insert(25);
    m.vend();
    m.dispensed * 100 + m.credit
}

fn main() {
    let got = cpu::run_value(SRC, "run", &[]);
    let want = host_run();

    println!("vending machine: insert 10,10,10 → vend → insert 25,25 → vend");
    println!("  dispensed*100 + credit  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ 2 vends, 20 credit left (= 220) on the Z80");
}
