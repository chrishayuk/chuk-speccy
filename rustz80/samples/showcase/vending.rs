// A vending-machine state machine, in the rustz80 dialect.
// Entry `run` -> dispensed*100 + leftover credit (220).
// Shows: `enum` states, a `struct` holding state, `&mut self` methods.

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
        self.credit = self.credit + coin;
        if self.credit >= 30u16 {
            self.state = State::Paid;
        }
    }
    fn vend(&mut self) {
        if self.state == State::Paid {
            self.dispensed = self.dispensed + 1u16;
            self.credit = self.credit - 30u16;
            self.state = State::Idle;
        }
    }
}

fn run() -> u16 {
    let mut m = Machine {
        state: State::Idle,
        credit: 0u16,
        dispensed: 0u16,
    };
    m.insert(10u16);
    m.insert(10u16);
    m.insert(10u16); // credit 30 -> Paid
    m.vend(); // dispense #1, credit 0
    m.insert(25u16);
    m.insert(25u16); // credit 50 -> Paid
    m.vend(); // dispense #2, credit 20

    // Pack the outcome into one u16: dispensed in the hundreds, credit in units.
    m.dispensed * 100u16 + m.credit
}
