//! Stream a ZEX test ROM (ZEXDOC / ZEXALL) through the z80 core, printing output
//! live. Usage: `cargo run --release --example zex -- testroms/zexdoc.com`

use std::io::Write;

fn main() {
    let path = std::env::args().nth(1).expect("usage: zex <rom.com>");
    let rom = std::fs::read(&path).expect("read ROM");
    let mut stdout = std::io::stdout();
    z80_tests::run_zex_with(&rom, 200_000_000_000, |c| {
        // ZEX uses CR (0x0D) for line breaks; normalise so the terminal scrolls.
        let b = c as u8;
        if b == b'\r' {
            let _ = stdout.write_all(b"\n");
        } else {
            let _ = stdout.write_all(&[b]);
        }
        let _ = stdout.flush();
    });
}
