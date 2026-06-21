//! Boot the 48K ROM, type `PRINT 6*7`, and show the result — proving the
//! keyboard matrix + interrupt-driven scan + BASIC all work end to end.
//! Usage: `cargo run --release -p spectrum --example type -- testroms/48.rom`

use spectrum::keyboard::KeyPos;
use spectrum::Spectrum;

fn main() {
    let path = std::env::args().nth(1).expect("usage: type <48.rom>");
    let rom = std::fs::read(&path).expect("read ROM");
    let mut spec = Spectrum::new_48k(&rom);

    // Boot to the copyright prompt / K cursor.
    for _ in 0..250 {
        spec.run_frame();
    }

    // At the K cursor the first key is a keyword: P -> PRINT.
    spec.press(None, KeyPos { row: 5, col: 0 }, 3, 3);
    // Then we're in L mode; type the expression and ENTER.
    spec.type_text("6*7\n");

    // Let it execute and settle.
    for _ in 0..30 {
        spec.run_frame();
    }

    println!("+{}+", "-".repeat(32));
    for line in spec.screen_text().lines() {
        println!("|{line}|");
    }
    println!("+{}+", "-".repeat(32));
}
